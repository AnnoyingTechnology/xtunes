// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VolumePercent(u8);

impl VolumePercent {
    pub const MAX: u8 = 100;

    pub const fn new(value: u8) -> Option<Self> {
        if value <= Self::MAX {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn from_clamped(value: u8) -> Self {
        if value > Self::MAX {
            Self(Self::MAX)
        } else {
            Self(value)
        }
    }

    pub fn from_scalar(value: f64) -> Self {
        if !value.is_finite() {
            return Self(0);
        }

        let percent = (value.clamp(0.0, 1.0) * f64::from(Self::MAX)).round();
        Self(percent as u8)
    }

    pub const fn get(self) -> u8 {
        self.0
    }

    pub fn as_scalar(self) -> f64 {
        f64::from(self.0) / f64::from(Self::MAX)
    }
}

impl Default for VolumePercent {
    fn default() -> Self {
        Self(Self::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::VolumePercent;

    #[test]
    fn volume_percent_accepts_only_percent_range() {
        assert_eq!(VolumePercent::new(100).map(VolumePercent::get), Some(100));
        assert_eq!(VolumePercent::new(101), None);
    }

    #[test]
    fn volume_percent_converts_from_scalar() {
        assert_eq!(VolumePercent::from_scalar(0.425).get(), 43);
        assert_eq!(VolumePercent::from_scalar(2.0).get(), 100);
        assert_eq!(VolumePercent::from_scalar(f64::NAN).get(), 0);
    }
}
