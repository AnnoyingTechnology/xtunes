// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{
    cmp::Reverse,
    time::{Duration, SystemTime},
};

use crate::{
    SmartPlaylistDateField, SmartPlaylistLimit, SmartPlaylistLimitSelection,
    SmartPlaylistMatchKind, SmartPlaylistNumberField, SmartPlaylistNumberOperator,
    SmartPlaylistRule, SmartPlaylistRuleSet, SmartPlaylistTextField, SmartPlaylistTextOperator,
    Track,
};

const SECONDS_PER_DAY: u64 = 86_400;

pub fn matching_tracks<'a>(
    tracks: &'a [Track],
    rules: &SmartPlaylistRuleSet,
    now: SystemTime,
) -> Vec<&'a Track> {
    let mut matched: Vec<&Track> = tracks
        .iter()
        .filter(|track| track_matches_rule_set(track, rules, now))
        .collect();

    if let Some(limit) = rules.limit {
        apply_limit(&mut matched, limit);
    }

    matched
}

pub fn track_matches_rule_set(
    track: &Track,
    rules: &SmartPlaylistRuleSet,
    now: SystemTime,
) -> bool {
    if rules.rules.is_empty() {
        return false;
    }

    match rules.match_kind {
        SmartPlaylistMatchKind::All => rules
            .rules
            .iter()
            .all(|rule| track_matches_rule(track, rule, now)),
        SmartPlaylistMatchKind::Any => rules
            .rules
            .iter()
            .any(|rule| track_matches_rule(track, rule, now)),
    }
}

pub fn track_matches_rule(track: &Track, rule: &SmartPlaylistRule, now: SystemTime) -> bool {
    match rule {
        SmartPlaylistRule::Text {
            field,
            operator,
            value,
        } => evaluate_text(text_field_value(track, *field).as_deref(), *operator, value),
        SmartPlaylistRule::TextIsEmpty { field } => text_field_value(track, *field)
            .map(|value| value.trim().is_empty())
            .unwrap_or(true),
        SmartPlaylistRule::TextIsPresent { field } => text_field_value(track, *field)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false),
        SmartPlaylistRule::Number {
            field,
            operator,
            value,
        } => evaluate_number(number_field_value(track, *field), *operator, *value),
        SmartPlaylistRule::Rating { operator, value } => evaluate_number(
            Some(i64::from(track.rating.stars())),
            *operator,
            i64::from(value.stars()),
        ),
        SmartPlaylistRule::DateBefore { field, date } => {
            date_field_value(track, *field).is_some_and(|track_date| track_date < *date)
        }
        SmartPlaylistRule::DateAfter { field, date } => {
            date_field_value(track, *field).is_some_and(|track_date| track_date > *date)
        }
        SmartPlaylistRule::DateInLast { field, days } => {
            let cutoff = now
                .checked_sub(Duration::from_secs(u64::from(days.get()) * SECONDS_PER_DAY))
                .unwrap_or(SystemTime::UNIX_EPOCH);
            date_field_value(track, *field).is_some_and(|track_date| track_date >= cutoff)
        }
        SmartPlaylistRule::DateNotInLast { field, days } => {
            let cutoff = now
                .checked_sub(Duration::from_secs(u64::from(days.get()) * SECONDS_PER_DAY))
                .unwrap_or(SystemTime::UNIX_EPOCH);
            match date_field_value(track, *field) {
                Some(track_date) => track_date < cutoff,
                None => true,
            }
        }
        SmartPlaylistRule::DateIsEmpty { field } => date_field_value(track, *field).is_none(),
        SmartPlaylistRule::DateIsPresent { field } => date_field_value(track, *field).is_some(),
    }
}

fn text_field_value(track: &Track, field: SmartPlaylistTextField) -> Option<String> {
    match field {
        SmartPlaylistTextField::Title => track.metadata.title.clone(),
        SmartPlaylistTextField::Artist => track.metadata.artist.clone(),
        SmartPlaylistTextField::Album => track.metadata.album.clone(),
        SmartPlaylistTextField::AlbumArtist => track.metadata.album_artist.clone(),
        SmartPlaylistTextField::Composer => track.metadata.composer.clone(),
        SmartPlaylistTextField::Genre => track.metadata.genre.clone(),
        SmartPlaylistTextField::FileName => track
            .location
            .relative_path
            .as_path()
            .file_name()
            .and_then(|os_str| os_str.to_str())
            .map(str::to_owned),
    }
}

fn number_field_value(track: &Track, field: SmartPlaylistNumberField) -> Option<i64> {
    match field {
        SmartPlaylistNumberField::PlayCount => Some(track.statistics.play_count as i64),
        SmartPlaylistNumberField::SkipCount => Some(track.statistics.skip_count as i64),
        SmartPlaylistNumberField::TrackNumber => track.metadata.track_number.map(i64::from),
        SmartPlaylistNumberField::DiscNumber => track.metadata.disc_number.map(i64::from),
        SmartPlaylistNumberField::Year => track.metadata.year.map(i64::from),
        SmartPlaylistNumberField::DurationSeconds => track
            .metadata
            .duration
            .map(|duration| duration.as_secs() as i64),
        SmartPlaylistNumberField::BitrateKbps => track.metadata.bitrate_kbps.map(i64::from),
    }
}

fn date_field_value(track: &Track, field: SmartPlaylistDateField) -> Option<SystemTime> {
    match field {
        SmartPlaylistDateField::DateAdded => track.statistics.date_added_at,
        SmartPlaylistDateField::LastPlayed => track.statistics.last_played_at,
        SmartPlaylistDateField::LastSkipped => track.statistics.last_skipped_at,
    }
}

fn evaluate_text(
    track_value: Option<&str>,
    operator: SmartPlaylistTextOperator,
    rule_value: &str,
) -> bool {
    let Some(track_value) = track_value else {
        return false;
    };
    let track = track_value.to_lowercase();
    let needle = rule_value.to_lowercase();
    match operator {
        SmartPlaylistTextOperator::Contains => track.contains(&needle),
        SmartPlaylistTextOperator::DoesNotContain => !track.contains(&needle),
        SmartPlaylistTextOperator::Is => track == needle,
        SmartPlaylistTextOperator::IsNot => track != needle,
        SmartPlaylistTextOperator::StartsWith => track.starts_with(&needle),
        SmartPlaylistTextOperator::EndsWith => track.ends_with(&needle),
    }
}

fn evaluate_number(
    track_value: Option<i64>,
    operator: SmartPlaylistNumberOperator,
    rule_value: i64,
) -> bool {
    let Some(track_value) = track_value else {
        return false;
    };
    match operator {
        SmartPlaylistNumberOperator::Equal => track_value == rule_value,
        SmartPlaylistNumberOperator::NotEqual => track_value != rule_value,
        SmartPlaylistNumberOperator::GreaterThan => track_value > rule_value,
        SmartPlaylistNumberOperator::GreaterThanOrEqual => track_value >= rule_value,
        SmartPlaylistNumberOperator::LessThan => track_value < rule_value,
        SmartPlaylistNumberOperator::LessThanOrEqual => track_value <= rule_value,
    }
}

fn apply_limit(tracks: &mut Vec<&Track>, limit: SmartPlaylistLimit) {
    sort_for_selection(tracks, limit.selection);
    tracks.truncate(limit.count.get() as usize);
}

fn sort_for_selection(tracks: &mut [&Track], selection: SmartPlaylistLimitSelection) {
    match selection {
        SmartPlaylistLimitSelection::Random => {
            tracks.sort_by_key(|track| track.id.get());
        }
        SmartPlaylistLimitSelection::TitleAscending => {
            tracks.sort_by(|left, right| {
                ci_string(left.metadata.title.as_deref())
                    .cmp(&ci_string(right.metadata.title.as_deref()))
            });
        }
        SmartPlaylistLimitSelection::ArtistAscending => {
            tracks.sort_by(|left, right| {
                ci_string(left.metadata.artist.as_deref())
                    .cmp(&ci_string(right.metadata.artist.as_deref()))
            });
        }
        SmartPlaylistLimitSelection::AlbumAscending => {
            tracks.sort_by(|left, right| {
                ci_string(left.metadata.album.as_deref())
                    .cmp(&ci_string(right.metadata.album.as_deref()))
            });
        }
        SmartPlaylistLimitSelection::GenreAscending => {
            tracks.sort_by(|left, right| {
                ci_string(left.metadata.genre.as_deref())
                    .cmp(&ci_string(right.metadata.genre.as_deref()))
            });
        }
        SmartPlaylistLimitSelection::HighestRating => {
            tracks.sort_by_key(|track| Reverse(track.rating.stars()));
        }
        SmartPlaylistLimitSelection::LowestRating => {
            tracks.sort_by_key(|track| track.rating.stars());
        }
        SmartPlaylistLimitSelection::MostRecentlyPlayed => {
            tracks.sort_by_key(|track| Reverse(track.statistics.last_played_at));
        }
        SmartPlaylistLimitSelection::LeastRecentlyPlayed => {
            tracks.sort_by_key(|track| track.statistics.last_played_at);
        }
        SmartPlaylistLimitSelection::MostOftenPlayed => {
            tracks.sort_by_key(|track| Reverse(track.statistics.play_count));
        }
        SmartPlaylistLimitSelection::LeastOftenPlayed => {
            tracks.sort_by_key(|track| track.statistics.play_count);
        }
        SmartPlaylistLimitSelection::MostRecentlyAdded => {
            tracks.sort_by_key(|track| Reverse(track.statistics.date_added_at));
        }
        SmartPlaylistLimitSelection::LeastRecentlyAdded => {
            tracks.sort_by_key(|track| track.statistics.date_added_at);
        }
    }
}

fn ci_string(value: Option<&str>) -> String {
    value.unwrap_or("").to_lowercase()
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;
    use std::time::{Duration, SystemTime};

    use crate::{
        PlayStatistics, Rating, SmartPlaylistDateField, SmartPlaylistLimit,
        SmartPlaylistLimitSelection, SmartPlaylistMatchKind, SmartPlaylistNumberField,
        SmartPlaylistNumberOperator, SmartPlaylistRule, SmartPlaylistRuleSet,
        SmartPlaylistTextField, SmartPlaylistTextOperator, Track, TrackId, TrackLocation,
        TrackMetadata, TrackRelativePath,
    };

    use super::{matching_tracks, track_matches_rule, track_matches_rule_set};

    fn unix(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn relative(path: &str) -> TrackRelativePath {
        TrackRelativePath::new(path).expect("relative path is valid")
    }

    fn track(id: i64, genre: Option<&str>, play_count: u64, year: Option<i32>) -> Track {
        Track {
            id: TrackId::new(id).expect("positive id"),
            location: TrackLocation::available(relative(&format!("track-{id}.flac"))),
            metadata: TrackMetadata {
                title: Some(format!("Title {id}")),
                artist: Some("Artist".to_owned()),
                album: Some("Album".to_owned()),
                album_artist: None,
                composer: None,
                genre: genre.map(str::to_owned),
                track_number: None,
                disc_number: None,
                year,
                duration: Some(Duration::from_secs(200)),
                bitrate_kbps: Some(320),
            },
            rating: Rating::unrated(),
            statistics: PlayStatistics {
                play_count,
                skip_count: 0,
                last_played_at: None,
                last_skipped_at: None,
                date_added_at: Some(unix(1_000)),
            },
        }
    }

    #[test]
    fn text_contains_is_case_insensitive() {
        let track = track(1, Some("Trip-Hop"), 0, None);
        let rule = SmartPlaylistRule::Text {
            field: SmartPlaylistTextField::Genre,
            operator: SmartPlaylistTextOperator::Contains,
            value: "trip".to_owned(),
        };

        assert!(track_matches_rule(&track, &rule, unix(2_000)));
    }

    #[test]
    fn text_is_not_rejects_exact_match() {
        let track = track(1, Some("Jazz"), 0, None);
        let rule = SmartPlaylistRule::Text {
            field: SmartPlaylistTextField::Genre,
            operator: SmartPlaylistTextOperator::IsNot,
            value: "Jazz".to_owned(),
        };

        assert!(!track_matches_rule(&track, &rule, unix(2_000)));
    }

    #[test]
    fn missing_text_field_does_not_match_text_rule() {
        let track = track(1, None, 0, None);
        let rule = SmartPlaylistRule::Text {
            field: SmartPlaylistTextField::Genre,
            operator: SmartPlaylistTextOperator::Contains,
            value: "Rock".to_owned(),
        };

        assert!(!track_matches_rule(&track, &rule, unix(2_000)));
    }

    #[test]
    fn number_greater_than_or_equal_matches_play_count() {
        let track = track(1, Some("Jazz"), 5, None);
        let rule = SmartPlaylistRule::Number {
            field: SmartPlaylistNumberField::PlayCount,
            operator: SmartPlaylistNumberOperator::GreaterThanOrEqual,
            value: 5,
        };

        assert!(track_matches_rule(&track, &rule, unix(2_000)));
    }

    #[test]
    fn missing_number_field_does_not_match_number_rule() {
        let track = track(1, Some("Jazz"), 0, None);
        let rule = SmartPlaylistRule::Number {
            field: SmartPlaylistNumberField::Year,
            operator: SmartPlaylistNumberOperator::LessThan,
            value: 2000,
        };

        assert!(!track_matches_rule(&track, &rule, unix(2_000)));
    }

    #[test]
    fn rating_rule_uses_rating_stars() {
        let mut track = track(1, Some("Jazz"), 0, None);
        track.rating = Rating::new(4).expect("valid rating");
        let rule = SmartPlaylistRule::Rating {
            operator: SmartPlaylistNumberOperator::GreaterThanOrEqual,
            value: Rating::new(4).expect("valid rating"),
        };

        assert!(track_matches_rule(&track, &rule, unix(2_000)));
    }

    #[test]
    fn date_in_last_uses_injected_clock() {
        let mut track = track(1, Some("Jazz"), 0, None);
        track.statistics.last_played_at = Some(unix(900_000));
        let rule = SmartPlaylistRule::DateInLast {
            field: SmartPlaylistDateField::LastPlayed,
            days: NonZeroU32::new(2).expect("positive days"),
        };

        let now_inside_window = unix(900_000 + 86_400);
        let now_outside_window = unix(900_000 + 5 * 86_400);

        assert!(track_matches_rule(&track, &rule, now_inside_window));
        assert!(!track_matches_rule(&track, &rule, now_outside_window));
    }

    #[test]
    fn date_not_in_last_treats_missing_date_as_matching() {
        let track = track(1, Some("Jazz"), 0, None);
        let rule = SmartPlaylistRule::DateNotInLast {
            field: SmartPlaylistDateField::LastPlayed,
            days: NonZeroU32::new(7).expect("positive days"),
        };

        assert!(track_matches_rule(&track, &rule, unix(2_000)));
    }

    #[test]
    fn match_all_requires_every_rule_to_match() {
        let track = track(1, Some("Jazz"), 5, Some(1995));
        let rules = SmartPlaylistRuleSet {
            match_kind: SmartPlaylistMatchKind::All,
            rules: vec![
                SmartPlaylistRule::Text {
                    field: SmartPlaylistTextField::Genre,
                    operator: SmartPlaylistTextOperator::Is,
                    value: "Jazz".to_owned(),
                },
                SmartPlaylistRule::Number {
                    field: SmartPlaylistNumberField::Year,
                    operator: SmartPlaylistNumberOperator::LessThan,
                    value: 2000,
                },
            ],
            limit: None,
        };

        assert!(track_matches_rule_set(&track, &rules, unix(2_000)));
    }

    #[test]
    fn match_any_passes_when_one_rule_matches() {
        let track = track(1, Some("Jazz"), 0, Some(2010));
        let rules = SmartPlaylistRuleSet {
            match_kind: SmartPlaylistMatchKind::Any,
            rules: vec![
                SmartPlaylistRule::Text {
                    field: SmartPlaylistTextField::Genre,
                    operator: SmartPlaylistTextOperator::Is,
                    value: "Jazz".to_owned(),
                },
                SmartPlaylistRule::Number {
                    field: SmartPlaylistNumberField::Year,
                    operator: SmartPlaylistNumberOperator::LessThan,
                    value: 2000,
                },
            ],
            limit: None,
        };

        assert!(track_matches_rule_set(&track, &rules, unix(2_000)));
    }

    #[test]
    fn empty_rule_set_matches_nothing() {
        let track = track(1, Some("Jazz"), 0, None);
        let rules = SmartPlaylistRuleSet {
            match_kind: SmartPlaylistMatchKind::All,
            rules: Vec::new(),
            limit: None,
        };

        assert!(!track_matches_rule_set(&track, &rules, unix(2_000)));
    }

    #[test]
    fn limit_truncates_to_count_after_sorting_by_play_count() {
        let mut tracks = vec![
            track(1, Some("Jazz"), 10, None),
            track(2, Some("Jazz"), 1, None),
            track(3, Some("Jazz"), 5, None),
            track(4, Some("Jazz"), 7, None),
        ];
        for (index, value) in [10_u64, 1, 5, 7].iter().enumerate() {
            tracks[index].statistics.play_count = *value;
        }

        let rules = SmartPlaylistRuleSet {
            match_kind: SmartPlaylistMatchKind::All,
            rules: vec![SmartPlaylistRule::Text {
                field: SmartPlaylistTextField::Genre,
                operator: SmartPlaylistTextOperator::Is,
                value: "Jazz".to_owned(),
            }],
            limit: Some(SmartPlaylistLimit {
                count: NonZeroU32::new(2).expect("positive count"),
                selection: SmartPlaylistLimitSelection::MostOftenPlayed,
            }),
        };

        let matched = matching_tracks(&tracks, &rules, unix(2_000));
        let matched_ids: Vec<i64> = matched.iter().map(|track| track.id.get()).collect();

        assert_eq!(matched_ids, vec![1, 4]);
    }

    #[test]
    fn limit_sorted_by_most_recently_added_puts_newer_first() {
        let mut tracks = vec![
            track(1, Some("Jazz"), 0, None),
            track(2, Some("Jazz"), 0, None),
            track(3, Some("Jazz"), 0, None),
        ];
        tracks[0].statistics.date_added_at = Some(unix(100));
        tracks[1].statistics.date_added_at = Some(unix(300));
        tracks[2].statistics.date_added_at = Some(unix(200));

        let rules = SmartPlaylistRuleSet {
            match_kind: SmartPlaylistMatchKind::All,
            rules: vec![SmartPlaylistRule::Text {
                field: SmartPlaylistTextField::Genre,
                operator: SmartPlaylistTextOperator::Is,
                value: "Jazz".to_owned(),
            }],
            limit: Some(SmartPlaylistLimit {
                count: NonZeroU32::new(3).expect("positive count"),
                selection: SmartPlaylistLimitSelection::MostRecentlyAdded,
            }),
        };

        let matched = matching_tracks(&tracks, &rules, unix(2_000));
        let matched_ids: Vec<i64> = matched.iter().map(|track| track.id.get()).collect();

        assert_eq!(matched_ids, vec![2, 3, 1]);
    }
}
