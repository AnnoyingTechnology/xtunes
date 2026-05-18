use crate::PlaylistId;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LibraryQuery {
    pub search_text: Option<String>,
    pub playlist_id: Option<PlaylistId>,
    pub sort: TrackSort,
}

impl LibraryQuery {
    pub fn all() -> Self {
        Self::default()
    }

    pub fn with_search_text(mut self, search_text: impl Into<String>) -> Self {
        let normalized = search_text.into().trim().to_owned();
        self.search_text = if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        };
        self
    }

    pub fn in_playlist(mut self, playlist_id: PlaylistId) -> Self {
        self.playlist_id = Some(playlist_id);
        self
    }

    pub fn sorted_by(mut self, sort: TrackSort) -> Self {
        self.sort = sort;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrackSort {
    pub column: TrackSortColumn,
    pub direction: SortDirection,
}

impl Default for TrackSort {
    fn default() -> Self {
        Self {
            column: TrackSortColumn::Title,
            direction: SortDirection::Ascending,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackSortColumn {
    Title,
    Artist,
    Album,
    Genre,
    Rating,
    PlayCount,
    LastPlayed,
    Duration,
    DateAdded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[cfg(test)]
mod tests {
    use super::{LibraryQuery, SortDirection, TrackSort, TrackSortColumn};

    #[test]
    fn blank_search_text_is_treated_as_no_search() {
        let query = LibraryQuery::all().with_search_text("   ");

        assert_eq!(query.search_text, None);
    }

    #[test]
    fn search_text_is_trimmed() {
        let query = LibraryQuery::all().with_search_text("  Massive Attack  ");

        assert_eq!(query.search_text.as_deref(), Some("Massive Attack"));
    }

    #[test]
    fn default_sort_is_title_ascending() {
        assert_eq!(
            LibraryQuery::default().sort,
            TrackSort {
                column: TrackSortColumn::Title,
                direction: SortDirection::Ascending
            }
        );
    }
}
