#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Rating(u8);

impl Rating {
    pub const MAX_STARS: u8 = 5;

    pub const fn new(stars: u8) -> Option<Self> {
        if stars <= Self::MAX_STARS {
            Some(Self(stars))
        } else {
            None
        }
    }

    pub const fn unrated() -> Self {
        Self(0)
    }

    pub const fn stars(self) -> u8 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::Rating;

    #[test]
    fn rating_matches_itunes_star_range() {
        assert_eq!(Rating::new(0).map(Rating::stars), Some(0));
        assert_eq!(Rating::new(5).map(Rating::stars), Some(5));
        assert_eq!(Rating::new(6), None);
    }
}
