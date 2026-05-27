// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

/// The twelve major and twelve minor keys of Western tonality. Used by
/// the analysis crate to report detected key and by the UI to display
/// and sort by it. Lives in the domain layer (not in the analysis
/// crate) so the Track model and view code can carry the value without
/// pulling in symphonia/DSP dependencies.
///
/// Discriminant order is chromatic, majors first (C..B), then minors
/// (Cm..Bm). Enharmonic spellings collapse to the flat variant for
/// black-key majors (Db/Eb/Gb/Ab/Bb) and the sharp variant for black-key
/// minors (C#m/D#m... no, see below). This matches what most music tools
/// surface to the user and avoids representing the same pitch class with
/// two distinct enum values.
///
/// Minor-key spelling note: Cm/Dm/Em/Fm/Gm/Am/Bm use natural roots; the
/// black-key minors use sharps for C#m and F#m (because their flat
/// spellings Dbm/Gbm are extremely rare in printed music), and flats for
/// Ebm/Abm/Bbm (the conventional jazz/classical spellings). The DM minor
/// black-key with both common spellings is D#m/Ebm; we pick Ebm because
/// it dominates jazz/rock chart usage.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MusicalKey {
    CMajor,
    DbMajor,
    DMajor,
    EbMajor,
    EMajor,
    FMajor,
    GbMajor,
    GMajor,
    AbMajor,
    AMajor,
    BbMajor,
    BMajor,
    CMinor,
    CsMinor,
    DMinor,
    EbMinor,
    EMinor,
    FMinor,
    FsMinor,
    GMinor,
    AbMinor,
    AMinor,
    BbMinor,
    BMinor,
}

impl MusicalKey {
    /// Every variant in declaration (chromatic) order. Useful for UI
    /// dropdowns, round-trip tests, and database enum mappings.
    pub const ALL: [Self; 24] = [
        Self::CMajor,
        Self::DbMajor,
        Self::DMajor,
        Self::EbMajor,
        Self::EMajor,
        Self::FMajor,
        Self::GbMajor,
        Self::GMajor,
        Self::AbMajor,
        Self::AMajor,
        Self::BbMajor,
        Self::BMajor,
        Self::CMinor,
        Self::CsMinor,
        Self::DMinor,
        Self::EbMinor,
        Self::EMinor,
        Self::FMinor,
        Self::FsMinor,
        Self::GMinor,
        Self::AbMinor,
        Self::AMinor,
        Self::BbMinor,
        Self::BMinor,
    ];

    /// Compact display form (e.g. "C", "Ebm", "F#m"). Stable: this is
    /// the canonical value persisted in shared tag frames like ID3v2
    /// `TKEY` so other tools can read it.
    pub const fn short_code(self) -> &'static str {
        match self {
            Self::CMajor => "C",
            Self::DbMajor => "Db",
            Self::DMajor => "D",
            Self::EbMajor => "Eb",
            Self::EMajor => "E",
            Self::FMajor => "F",
            Self::GbMajor => "Gb",
            Self::GMajor => "G",
            Self::AbMajor => "Ab",
            Self::AMajor => "A",
            Self::BbMajor => "Bb",
            Self::BMajor => "B",
            Self::CMinor => "Cm",
            Self::CsMinor => "C#m",
            Self::DMinor => "Dm",
            Self::EbMinor => "Ebm",
            Self::EMinor => "Em",
            Self::FMinor => "Fm",
            Self::FsMinor => "F#m",
            Self::GMinor => "Gm",
            Self::AbMinor => "Abm",
            Self::AMinor => "Am",
            Self::BbMinor => "Bbm",
            Self::BMinor => "Bm",
        }
    }

    /// Human-readable form (e.g. "C major", "F# minor"). For tooltips
    /// and verbose listings; the table view uses `short_code`.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::CMajor => "C major",
            Self::DbMajor => "Db major",
            Self::DMajor => "D major",
            Self::EbMajor => "Eb major",
            Self::EMajor => "E major",
            Self::FMajor => "F major",
            Self::GbMajor => "Gb major",
            Self::GMajor => "G major",
            Self::AbMajor => "Ab major",
            Self::AMajor => "A major",
            Self::BbMajor => "Bb major",
            Self::BMajor => "B major",
            Self::CMinor => "C minor",
            Self::CsMinor => "C# minor",
            Self::DMinor => "D minor",
            Self::EbMinor => "Eb minor",
            Self::EMinor => "E minor",
            Self::FMinor => "F minor",
            Self::FsMinor => "F# minor",
            Self::GMinor => "G minor",
            Self::AbMinor => "Ab minor",
            Self::AMinor => "A minor",
            Self::BbMinor => "Bb minor",
            Self::BMinor => "B minor",
        }
    }

    /// True for the twelve major keys; false for the twelve minor keys.
    pub const fn is_major(self) -> bool {
        (self as u8) < 12
    }

    /// Parse a `short_code` back into a key. Returns `None` for any
    /// other input; callers that accept user-typed values should
    /// normalize whitespace/case first.
    pub fn from_short_code(code: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|key| key.short_code() == code)
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::MusicalKey;

    #[test]
    fn all_keys_round_trip_through_short_code() {
        for key in MusicalKey::ALL {
            assert_eq!(MusicalKey::from_short_code(key.short_code()), Some(key));
        }
    }

    #[test]
    fn first_twelve_keys_are_major() {
        for key in MusicalKey::ALL.iter().take(12) {
            assert!(key.is_major(), "{} should be major", key.display_name());
        }
        for key in MusicalKey::ALL.iter().skip(12) {
            assert!(!key.is_major(), "{} should be minor", key.display_name());
        }
    }

    #[test]
    fn short_codes_distinguish_majors_and_minors() {
        // The minor suffix `m` is what separates "C" from "Cm" at a glance;
        // also covers the sharp/flat black-key spellings.
        assert_eq!(MusicalKey::CMajor.short_code(), "C");
        assert_eq!(MusicalKey::CMinor.short_code(), "Cm");
        assert_eq!(MusicalKey::DbMajor.short_code(), "Db");
        assert_eq!(MusicalKey::CsMinor.short_code(), "C#m");
        assert_eq!(MusicalKey::EbMinor.short_code(), "Ebm");
        assert_eq!(MusicalKey::FsMinor.short_code(), "F#m");
    }

    #[test]
    fn from_short_code_rejects_unknown_input() {
        assert_eq!(MusicalKey::from_short_code(""), None);
        assert_eq!(MusicalKey::from_short_code("H"), None);
        assert_eq!(MusicalKey::from_short_code("c"), None); // case-sensitive
    }
}
