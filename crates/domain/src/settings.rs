// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LibrarySettings {
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UserSettings {
    pub library: LibrarySettings,
}

impl UserSettings {
    pub fn with_library_path(library_path: Option<PathBuf>) -> Self {
        Self {
            library: LibrarySettings { path: library_path },
        }
    }

    pub fn library_path(&self) -> Option<&Path> {
        self.library.path.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::UserSettings;

    #[test]
    fn library_path_is_unset_by_default() {
        assert_eq!(UserSettings::default().library.path, None);
    }

    #[test]
    fn settings_can_hold_a_library_path() {
        let settings = UserSettings::with_library_path(Some(PathBuf::from("/music")));

        assert_eq!(settings.library.path, Some(PathBuf::from("/music")));
    }
}
