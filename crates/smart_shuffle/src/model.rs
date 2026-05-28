// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Smart Shuffle persisted model. Pairs the trained
//! [`RandomForest`] with the
//! [`FeatureExtractor`] that
//! produced its inputs, plus the bookkeeping fields the library
//! store keeps alongside the blob (training-set label counts and a
//! `trained_at_unix` timestamp).
//!
//! The blob format is JSON for simplicity — the model is tiny by
//! the standards of modern ML (a few hundred KB at most) and
//! deserialisation cost is paid once at app start. Swapping to a
//! more compact binary form is a bookkeeping change behind
//! [`SmartShuffleModel::from_blob`] and [`SmartShuffleModel::to_blob`];
//! the public API does not change.

use serde::{Deserialize, Serialize};

use crate::SmartShuffleError;
use crate::feature::{FEATURE_SCHEMA_VERSION, FeatureExtractor};
use crate::forest::RandomForest;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SmartShuffleModel {
    extractor: FeatureExtractor,
    forest: RandomForest,
}

impl SmartShuffleModel {
    pub fn new(extractor: FeatureExtractor, forest: RandomForest) -> Self {
        Self { extractor, forest }
    }

    pub fn extractor(&self) -> &FeatureExtractor {
        &self.extractor
    }

    pub fn forest(&self) -> &RandomForest {
        &self.forest
    }

    /// Number of decision trees in the bagged ensemble. Surfaced so
    /// the Preferences "trained on N tracks" caption can also note
    /// how big the model is when the user is curious.
    pub fn tree_count(&self) -> usize {
        self.forest.tree_count()
    }

    pub fn feature_schema_version() -> u32 {
        FEATURE_SCHEMA_VERSION
    }

    pub fn to_blob(&self) -> Result<Vec<u8>, SmartShuffleError> {
        serde_json::to_vec(self).map_err(|_| SmartShuffleError::ModelSerialisationFailed)
    }

    pub fn from_blob(blob: &[u8]) -> Result<Self, SmartShuffleError> {
        serde_json::from_slice(blob).map_err(|_| SmartShuffleError::ModelDeserialisationFailed)
    }
}
