use std::time::SystemTime;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PlayStatistics {
    pub play_count: u64,
    pub skip_count: u64,
    pub last_played_at: Option<SystemTime>,
    pub last_skipped_at: Option<SystemTime>,
}
