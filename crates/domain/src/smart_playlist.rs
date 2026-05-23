// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{num::NonZeroU32, time::SystemTime};

use crate::{Rating, SmartPlaylistId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmartPlaylist {
    pub id: SmartPlaylistId,
    pub name: String,
    pub rules: SmartPlaylistRuleSet,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmartPlaylistRuleSet {
    pub match_kind: SmartPlaylistMatchKind,
    pub rules: Vec<SmartPlaylistRule>,
    pub limit: Option<NonZeroU32>,
}

impl SmartPlaylistRuleSet {
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

impl Default for SmartPlaylistRuleSet {
    fn default() -> Self {
        Self {
            match_kind: SmartPlaylistMatchKind::All,
            rules: Vec::new(),
            limit: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmartPlaylistMatchKind {
    All,
    Any,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SmartPlaylistRule {
    Text {
        field: SmartPlaylistTextField,
        operator: SmartPlaylistTextOperator,
        value: String,
    },
    TextIsEmpty {
        field: SmartPlaylistTextField,
    },
    TextIsPresent {
        field: SmartPlaylistTextField,
    },
    Number {
        field: SmartPlaylistNumberField,
        operator: SmartPlaylistNumberOperator,
        value: i64,
    },
    Rating {
        operator: SmartPlaylistNumberOperator,
        value: Rating,
    },
    DateBefore {
        field: SmartPlaylistDateField,
        date: SystemTime,
    },
    DateAfter {
        field: SmartPlaylistDateField,
        date: SystemTime,
    },
    DateInLast {
        field: SmartPlaylistDateField,
        days: NonZeroU32,
    },
    DateNotInLast {
        field: SmartPlaylistDateField,
        days: NonZeroU32,
    },
    DateIsEmpty {
        field: SmartPlaylistDateField,
    },
    DateIsPresent {
        field: SmartPlaylistDateField,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmartPlaylistTextField {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Composer,
    Genre,
    FileName,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmartPlaylistTextOperator {
    Contains,
    DoesNotContain,
    Is,
    IsNot,
    StartsWith,
    EndsWith,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmartPlaylistNumberField {
    PlayCount,
    SkipCount,
    TrackNumber,
    DiscNumber,
    Year,
    DurationSeconds,
    BitrateKbps,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmartPlaylistNumberOperator {
    Equal,
    NotEqual,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmartPlaylistDateField {
    DateAdded,
    LastPlayed,
    LastSkipped,
}

#[cfg(test)]
mod tests {
    use super::{SmartPlaylistMatchKind, SmartPlaylistRuleSet};

    #[test]
    fn rule_sets_default_to_matching_all_rules_without_a_limit() {
        let rule_set = SmartPlaylistRuleSet::default();

        assert_eq!(rule_set.match_kind, SmartPlaylistMatchKind::All);
        assert!(rule_set.is_empty());
        assert_eq!(rule_set.limit, None);
    }
}
