use std::path::PathBuf;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UserSettings {
    pub library_path: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::UserSettings;

    #[test]
    fn library_path_is_unset_by_default() {
        assert_eq!(UserSettings::default().library_path, None);
    }

    #[test]
    fn settings_can_hold_a_library_path() {
        let settings = UserSettings {
            library_path: Some(PathBuf::from("/music")),
        };

        assert_eq!(settings.library_path, Some(PathBuf::from("/music")));
    }
}
