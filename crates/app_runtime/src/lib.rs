#![forbid(unsafe_code)]

pub use xtunes_domain::{
    ApplicationCommand, ApplicationQuery, PlaybackState, ThemeMode, TrackId, UserSettings,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationRuntime {
    settings: UserSettings,
}

impl ApplicationRuntime {
    pub fn new() -> Self {
        Self {
            settings: UserSettings::default(),
        }
    }

    pub const fn settings(&self) -> &UserSettings {
        &self.settings
    }

    pub fn handle_command(&mut self, command: ApplicationCommand) {
        if let ApplicationCommand::UpdateSettings(settings) = command {
            self.settings = settings;
        }
    }
}

impl Default for ApplicationRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use xtunes_domain::{ApplicationCommand, ThemeMode, UserSettings};

    use super::ApplicationRuntime;

    #[test]
    fn runtime_starts_with_default_settings() {
        let runtime = ApplicationRuntime::new();

        assert_eq!(runtime.settings().theme_mode, ThemeMode::System);
    }

    #[test]
    fn runtime_accepts_settings_command() {
        let mut runtime = ApplicationRuntime::new();

        runtime.handle_command(ApplicationCommand::UpdateSettings(UserSettings {
            theme_mode: ThemeMode::Dark,
        }));

        assert_eq!(runtime.settings().theme_mode, ThemeMode::Dark);
    }
}
