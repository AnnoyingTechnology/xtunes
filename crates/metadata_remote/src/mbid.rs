// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Cheap structural validation for MusicBrainz IDs.
//!
//! Used as a pre-flight check before reaching for the network: a real
//! MBID is a lowercase canonical UUID (`8-4-4-4-12` hex). Rejecting
//! anything else avoids assembling malformed URLs and keeps the
//! shared rate-limit budget for legitimate calls.

pub(crate) fn is_well_formed(value: &str) -> bool {
    let segments: Vec<&str> = value.split('-').collect();
    if segments.len() != 5 {
        return false;
    }
    let lengths = [8, 4, 4, 4, 12];
    for (segment, expected) in segments.iter().zip(lengths) {
        if segment.len() != expected
            || !segment
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::is_well_formed;

    #[test]
    fn accepts_canonical_lowercase_uuid() {
        assert!(is_well_formed("3b3d130a-87a8-4a47-b9fb-920f2530d134"));
    }

    #[test]
    fn rejects_short_or_long_segments() {
        assert!(!is_well_formed("3b3d130-87a8-4a47-b9fb-920f2530d134"));
        assert!(!is_well_formed("3b3d130a-87a8-4a47-b9fb-920f2530d1345"));
    }

    #[test]
    fn rejects_non_hex_characters() {
        assert!(!is_well_formed("3b3d130z-87a8-4a47-b9fb-920f2530d134"));
    }

    #[test]
    fn empty_string_is_rejected() {
        assert!(!is_well_formed(""));
    }
}
