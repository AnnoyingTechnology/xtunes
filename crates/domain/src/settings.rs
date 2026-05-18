#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UserSettings {
    pub theme_mode: ThemeMode,
}

#[cfg(test)]
mod tests {
    use super::{ThemeMode, UserSettings};

    #[test]
    fn system_theme_is_the_default() {
        assert_eq!(UserSettings::default().theme_mode, ThemeMode::System);
    }
}
