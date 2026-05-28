// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Hand-rolled Random Forest binary classifier for the Smart Shuffle
//! engagement model.
//!
//! The implementation is intentionally small and dependency-free —
//! [`SmartShuffleTrainer`](crate::SmartShuffleTrainer) calls
//! [`RandomForest::train`] with a labelled feature matrix, the
//! resulting forest serialises through `serde` to the blob stored
//! in the library database, and the picker calls
//! [`RandomForest::predict_positive_probability`] on candidate
//! feature vectors.
//!
//! Algorithm sketch:
//! - Bagging: each tree is fit on a bootstrap sample (sampled with
//!   replacement) of the labelled set.
//! - Feature subsetting: each split node considers only `sqrt(F)`
//!   features chosen at random, where `F` is the full feature
//!   dimension. This decorrelates the trees and is the canonical
//!   Random Forest knob.
//! - Split criterion: Gini impurity reduction. A handful of
//!   candidate thresholds is tried per feature (the 25 / 50 / 75
//!   percentiles of the feature's values within the node, plus
//!   `0.5` as a sentinel for the one-hot genre columns).
//! - Stopping rules: a node becomes a leaf when it reaches
//!   `max_depth`, when it falls below `min_samples_leaf` after a
//!   split, or when it is already pure (all labels identical).
//!
//! All randomness is sourced from a [`SplitMix64`] PRNG seeded by
//! the caller, so training is reproducible-given-the-seed and
//! produces no thread-of-control non-determinism even under
//! identical inputs.

use serde::{Deserialize, Serialize};

/// Default ensemble size — empirically a good trade-off between
/// variance reduction and training time for libraries in the
/// 1k–50k track range.
pub const DEFAULT_TREE_COUNT: usize = 64;

/// Default maximum tree depth. Deeper trees overfit small label
/// sets fast; ten levels is enough to capture the meaningful
/// genre × engagement interactions without memorising the
/// training labels.
pub const DEFAULT_MAX_DEPTH: usize = 10;

/// Minimum number of samples in a node before it is allowed to
/// split. Anything below this becomes a leaf — small leaves are
/// where decision-tree overfit lives.
pub const DEFAULT_MIN_SAMPLES_LEAF: usize = 5;

#[derive(Clone, Copy, Debug)]
pub struct ForestHyperparameters {
    pub tree_count: usize,
    pub max_depth: usize,
    pub min_samples_leaf: usize,
}

impl Default for ForestHyperparameters {
    fn default() -> Self {
        Self {
            tree_count: DEFAULT_TREE_COUNT,
            max_depth: DEFAULT_MAX_DEPTH,
            min_samples_leaf: DEFAULT_MIN_SAMPLES_LEAF,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RandomForest {
    trees: Vec<DecisionTree>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DecisionTree {
    /// Heap-stored arena of nodes. Index `0` is always the root.
    /// Storing the tree as a flat `Vec<Node>` instead of recursive
    /// `Box<Node>` keeps serialisation simple and predictably
    /// laid out in memory.
    nodes: Vec<Node>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Node {
    Split {
        feature_index: usize,
        threshold: f32,
        left: usize,
        right: usize,
    },
    Leaf {
        positive_probability: f32,
    },
}

impl RandomForest {
    pub fn train(
        feature_matrix: &[Vec<f32>],
        labels: &[bool],
        hyperparameters: ForestHyperparameters,
        seed: u64,
    ) -> Self {
        assert!(
            !feature_matrix.is_empty() && feature_matrix.len() == labels.len(),
            "feature matrix and label vector must be non-empty and aligned"
        );
        let feature_count = feature_matrix[0].len();
        // Square-root feature subsetting, clamped at >= 1 so a
        // tiny feature space still gets some randomness.
        let features_per_split = (feature_count as f32).sqrt().ceil() as usize;
        let features_per_split = features_per_split.max(1).min(feature_count);

        let mut rng = SplitMix64::new(seed);
        let trees: Vec<DecisionTree> = (0..hyperparameters.tree_count)
            .map(|_| {
                let bootstrap = bootstrap_sample(feature_matrix.len(), &mut rng);
                DecisionTree::fit(
                    feature_matrix,
                    labels,
                    &bootstrap,
                    feature_count,
                    features_per_split,
                    hyperparameters.max_depth,
                    hyperparameters.min_samples_leaf,
                    &mut rng,
                )
            })
            .collect();

        Self { trees }
    }

    pub fn tree_count(&self) -> usize {
        self.trees.len()
    }

    /// Mean positive-class probability across the ensemble. The
    /// caller is responsible for ensuring the feature vector has
    /// the same shape the trees were trained against; the
    /// `feature_schema_version` mechanism around this crate enforces
    /// that invariant at load time.
    pub fn predict_positive_probability(&self, features: &[f32]) -> f32 {
        if self.trees.is_empty() {
            return 0.5;
        }
        let total: f32 = self.trees.iter().map(|tree| tree.predict(features)).sum();
        total / self.trees.len() as f32
    }
}

impl DecisionTree {
    #[allow(clippy::too_many_arguments)]
    fn fit(
        feature_matrix: &[Vec<f32>],
        labels: &[bool],
        bootstrap_indices: &[usize],
        feature_count: usize,
        features_per_split: usize,
        max_depth: usize,
        min_samples_leaf: usize,
        rng: &mut SplitMix64,
    ) -> Self {
        let mut nodes: Vec<Node> = Vec::new();
        let mut stack: Vec<PendingNode> = vec![PendingNode {
            indices: bootstrap_indices.to_vec(),
            depth: 0,
            parent_slot: None,
        }];

        while let Some(PendingNode {
            indices,
            depth,
            parent_slot,
        }) = stack.pop()
        {
            let leaf_node_index = nodes.len();
            let positive_count = indices.iter().filter(|index| labels[**index]).count();
            let total = indices.len().max(1);
            let positive_probability = positive_count as f32 / total as f32;

            // Hard-stop conditions: too shallow to bother, too few
            // samples to split, or pure node.
            if depth >= max_depth
                || indices.len() < (min_samples_leaf * 2)
                || positive_count == 0
                || positive_count == indices.len()
            {
                nodes.push(Node::Leaf {
                    positive_probability,
                });
                Self::link_parent(&mut nodes, parent_slot, leaf_node_index);
                continue;
            }

            // Pick `features_per_split` distinct feature indices.
            let candidate_features = sample_indices(feature_count, features_per_split, rng);
            let split = best_split(
                feature_matrix,
                labels,
                &indices,
                &candidate_features,
                min_samples_leaf,
            );

            let Some(BestSplit {
                feature_index,
                threshold,
                left_indices,
                right_indices,
            }) = split
            else {
                nodes.push(Node::Leaf {
                    positive_probability,
                });
                Self::link_parent(&mut nodes, parent_slot, leaf_node_index);
                continue;
            };

            // Reserve the split node with placeholder children;
            // patch them once the recursion materialises the
            // child node indices.
            let split_node_index = nodes.len();
            nodes.push(Node::Split {
                feature_index,
                threshold,
                left: usize::MAX,
                right: usize::MAX,
            });
            Self::link_parent(&mut nodes, parent_slot, split_node_index);

            // Push children in reverse so the LIFO pops them in
            // the natural left-then-right order. Each carries the
            // information needed to back-patch its slot in the
            // parent split node.
            stack.push(PendingNode {
                indices: right_indices,
                depth: depth + 1,
                parent_slot: Some(ChildSlot {
                    parent_node_index: split_node_index,
                    side: Side::Right,
                }),
            });
            stack.push(PendingNode {
                indices: left_indices,
                depth: depth + 1,
                parent_slot: Some(ChildSlot {
                    parent_node_index: split_node_index,
                    side: Side::Left,
                }),
            });
        }

        Self { nodes }
    }

    fn link_parent(nodes: &mut [Node], parent_slot: Option<ChildSlot>, child_index: usize) {
        let Some(ChildSlot {
            parent_node_index,
            side,
        }) = parent_slot
        else {
            return;
        };
        if let Some(Node::Split { left, right, .. }) = nodes.get_mut(parent_node_index) {
            match side {
                Side::Left => *left = child_index,
                Side::Right => *right = child_index,
            }
        }
    }

    fn predict(&self, features: &[f32]) -> f32 {
        if self.nodes.is_empty() {
            return 0.5;
        }
        let mut cursor = 0;
        loop {
            match &self.nodes[cursor] {
                Node::Leaf {
                    positive_probability,
                } => return *positive_probability,
                Node::Split {
                    feature_index,
                    threshold,
                    left,
                    right,
                } => {
                    let value = features.get(*feature_index).copied().unwrap_or(0.0);
                    cursor = if value <= *threshold { *left } else { *right };
                }
            }
        }
    }
}

#[derive(Debug)]
struct PendingNode {
    indices: Vec<usize>,
    depth: usize,
    parent_slot: Option<ChildSlot>,
}

#[derive(Clone, Copy, Debug)]
struct ChildSlot {
    parent_node_index: usize,
    side: Side,
}

#[derive(Clone, Copy, Debug)]
enum Side {
    Left,
    Right,
}

#[derive(Debug)]
struct BestSplit {
    feature_index: usize,
    threshold: f32,
    left_indices: Vec<usize>,
    right_indices: Vec<usize>,
}

fn best_split(
    feature_matrix: &[Vec<f32>],
    labels: &[bool],
    indices: &[usize],
    candidate_features: &[usize],
    min_samples_leaf: usize,
) -> Option<BestSplit> {
    let parent_gini = gini_impurity(indices, labels);
    let parent_size = indices.len() as f32;
    let mut best: Option<(f32, BestSplit)> = None;

    for feature_index in candidate_features.iter().copied() {
        // Collect the feature's values within this node, then
        // probe at the 25/50/75 percentile positions and the
        // 0.5 sentinel (one-hot columns). We accept the first
        // split that meaningfully reduces impurity over the
        // current best.
        let mut values: Vec<f32> = indices
            .iter()
            .map(|index| feature_matrix[*index][feature_index])
            .collect();
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        if values.first() == values.last() {
            // Constant feature within this node — no split would
            // reduce impurity.
            continue;
        }

        let percentile = |fraction: f32| -> f32 {
            let position = ((values.len() as f32 - 1.0) * fraction).round().max(0.0) as usize;
            values[position.min(values.len() - 1)]
        };
        let mut thresholds = [percentile(0.25), percentile(0.5), percentile(0.75), 0.5];
        thresholds.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        // Dedup adjacent equals to avoid wasted Gini computations.
        let mut probed: Vec<f32> = Vec::with_capacity(thresholds.len());
        for threshold in thresholds {
            if probed
                .last()
                .is_none_or(|prev| (*prev - threshold).abs() > f32::EPSILON)
            {
                probed.push(threshold);
            }
        }

        for threshold in probed {
            let mut left = Vec::with_capacity(indices.len());
            let mut right = Vec::with_capacity(indices.len());
            for index in indices.iter().copied() {
                if feature_matrix[index][feature_index] <= threshold {
                    left.push(index);
                } else {
                    right.push(index);
                }
            }
            if left.len() < min_samples_leaf || right.len() < min_samples_leaf {
                continue;
            }
            let weighted_child_gini = (left.len() as f32 / parent_size)
                * gini_impurity(&left, labels)
                + (right.len() as f32 / parent_size) * gini_impurity(&right, labels);
            let gain = parent_gini - weighted_child_gini;
            if gain <= 0.0 {
                continue;
            }
            if best.as_ref().is_none_or(|(best_gain, _)| gain > *best_gain) {
                best = Some((
                    gain,
                    BestSplit {
                        feature_index,
                        threshold,
                        left_indices: left,
                        right_indices: right,
                    },
                ));
            }
        }
    }

    best.map(|(_, split)| split)
}

fn gini_impurity(indices: &[usize], labels: &[bool]) -> f32 {
    if indices.is_empty() {
        return 0.0;
    }
    let positive = indices.iter().filter(|index| labels[**index]).count() as f32;
    let total = indices.len() as f32;
    let p_pos = positive / total;
    let p_neg = 1.0 - p_pos;
    1.0 - (p_pos * p_pos + p_neg * p_neg)
}

fn bootstrap_sample(size: usize, rng: &mut SplitMix64) -> Vec<usize> {
    (0..size).map(|_| rng.next_bounded(size)).collect()
}

/// Sample `count` distinct indices from `[0, total)`. When
/// `count >= total` the full set is returned. The internal
/// Fisher-Yates pass is bounded by `O(count)` swaps over a
/// `O(total)` scratch buffer; the trainer reaches this with
/// `count ≈ sqrt(total)`, keeping the per-split cost manageable.
fn sample_indices(total: usize, count: usize, rng: &mut SplitMix64) -> Vec<usize> {
    if count >= total {
        return (0..total).collect();
    }
    let mut pool: Vec<usize> = (0..total).collect();
    for swap_target in 0..count {
        let pick = swap_target + rng.next_bounded(total - swap_target);
        pool.swap(swap_target, pick);
    }
    pool.truncate(count);
    pool
}

/// Deterministic PRNG shared across the trainer and the picker.
/// Same algorithm as the existing pure-shuffle in the domain crate;
/// duplicated here so the smart-shuffle crate has no domain-private
/// dependency on a particular RNG implementation.
#[derive(Clone, Copy, Debug)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D049BB133111EB);
        value ^ (value >> 31)
    }

    pub fn next_unit_interval(&mut self) -> f32 {
        // Mantissa-aligned scaling: take the top 24 bits of the
        // u64 output, divide by 2^24. Produces a uniform [0, 1)
        // float.
        let bits = (self.next_u64() >> 40) as u32;
        bits as f32 / (1u32 << 24) as f32
    }

    pub fn next_bounded(&mut self, upper_bound_exclusive: usize) -> usize {
        if upper_bound_exclusive <= 1 {
            return 0;
        }
        let upper_bound = upper_bound_exclusive as u64;
        let rejection_threshold = u64::MAX - (u64::MAX % upper_bound);
        loop {
            let value = self.next_u64();
            if value < rejection_threshold {
                return (value % upper_bound) as usize;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ForestHyperparameters, RandomForest, SplitMix64};

    #[test]
    fn forest_predicts_higher_probability_on_engaged_inputs() {
        // Synthetic dataset: two features, label is true when
        // feature 0 is high AND feature 1 is high. A random forest
        // should pick this up and assign clearly different
        // probabilities to the two corners.
        let mut feature_matrix: Vec<Vec<f32>> = Vec::new();
        let mut labels: Vec<bool> = Vec::new();
        let mut rng = SplitMix64::new(42);
        for _ in 0..400 {
            let a = rng.next_unit_interval();
            let b = rng.next_unit_interval();
            feature_matrix.push(vec![a, b]);
            labels.push(a > 0.6 && b > 0.6);
        }

        let forest = RandomForest::train(
            &feature_matrix,
            &labels,
            ForestHyperparameters::default(),
            7,
        );

        let engaged = forest.predict_positive_probability(&[0.9, 0.9]);
        let unengaged = forest.predict_positive_probability(&[0.1, 0.1]);
        assert!(
            engaged > unengaged,
            "engaged corner ({engaged}) should outscore unengaged ({unengaged})"
        );
        assert!(
            engaged > 0.5,
            "engaged corner should land above the prior, got {engaged}"
        );
        assert!(
            unengaged < 0.3,
            "unengaged corner should land below the prior, got {unengaged}"
        );
    }

    #[test]
    fn forest_handles_pure_label_set() {
        let feature_matrix = vec![vec![0.0, 0.0]; 50];
        let labels = vec![true; 50];
        let forest = RandomForest::train(
            &feature_matrix,
            &labels,
            ForestHyperparameters::default(),
            1,
        );
        let probability = forest.predict_positive_probability(&[0.0, 0.0]);
        assert!(
            (probability - 1.0).abs() < 0.01,
            "pure-positive training set should produce near-1.0 predictions; got {probability}"
        );
    }

    #[test]
    fn forest_is_deterministic_given_the_same_seed() {
        let mut rng = SplitMix64::new(123);
        let feature_matrix: Vec<Vec<f32>> = (0..200)
            .map(|_| vec![rng.next_unit_interval(), rng.next_unit_interval()])
            .collect();
        let labels: Vec<bool> = feature_matrix
            .iter()
            .map(|row| row[0] + row[1] > 1.0)
            .collect();

        let first = RandomForest::train(
            &feature_matrix,
            &labels,
            ForestHyperparameters::default(),
            999,
        );
        let second = RandomForest::train(
            &feature_matrix,
            &labels,
            ForestHyperparameters::default(),
            999,
        );
        assert_eq!(first.tree_count(), second.tree_count());
        for sample in [[0.1, 0.1], [0.5, 0.5], [0.9, 0.9]] {
            let p1 = first.predict_positive_probability(&sample);
            let p2 = second.predict_positive_probability(&sample);
            assert!(
                (p1 - p2).abs() < 1e-6,
                "identical seeds should produce identical predictions ({p1} vs {p2} at {sample:?})"
            );
        }
    }

    #[test]
    fn split_mix_next_bounded_stays_in_range() {
        let mut rng = SplitMix64::new(0xfeed);
        for _ in 0..256 {
            let value = rng.next_bounded(10);
            assert!(value < 10);
        }
    }
}
