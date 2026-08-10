use thiserror::Error;

use crate::models::{PlaybackMode, Video};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaybackSnapshot {
    pub current: Option<Video>,
    pub mode: PlaybackMode,
    pub paused: bool,
    pub position_seconds: f64,
    pub duration_seconds: f64,
}

impl PlaybackSnapshot {
    pub fn progress_ratio(&self) -> f64 {
        if self.duration_seconds <= 0.0 {
            return 0.0;
        }
        (self.position_seconds / self.duration_seconds).clamp(0.0, 1.0)
    }
}

#[derive(Debug, Error)]
pub enum PlaybackError {
    #[error("mpv is not installed or could not be started: {0}")]
    Spawn(String),
    #[error("mpv IPC failed: {0}")]
    Ipc(String),
    #[error("yt-dlp could not resolve the selected video: {0}")]
    Resolve(String),
}

pub trait PlaybackEngine {
    fn play(&mut self, video: &Video, mode: PlaybackMode) -> Result<(), PlaybackError>;
    fn set_paused(&mut self, paused: bool) -> Result<(), PlaybackError>;
    fn set_mode(&mut self, mode: PlaybackMode) -> Result<(), PlaybackError>;
    fn snapshot(&self) -> &PlaybackSnapshot;
    fn stop(&mut self) -> Result<(), PlaybackError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_clamped_to_gauge_range() {
        let snapshot = PlaybackSnapshot {
            position_seconds: 15.0,
            duration_seconds: 10.0,
            ..PlaybackSnapshot::default()
        };

        assert_eq!(snapshot.progress_ratio(), 1.0);
    }
}
