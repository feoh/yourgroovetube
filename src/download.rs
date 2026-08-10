use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use thiserror::Error;

use crate::models::Video;

pub type SaveFuture<'a> = Pin<Box<dyn Future<Output = Result<PathBuf, SaveError>> + Send + 'a>>;

#[derive(Debug, Error)]
pub enum SaveError {
    #[error("yt-dlp download failed: {0}")]
    Download(String),
    #[error("the Plex destination is unavailable: {0}")]
    Destination(String),
    #[error("could not move the completed file into the Plex library: {0}")]
    Import(String),
}

pub trait VideoSaver: Send + Sync {
    fn save(&self, video: &Video) -> SaveFuture<'_>;
}
