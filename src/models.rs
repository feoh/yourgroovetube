use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel_title: String,
    pub description: String,
    pub duration_seconds: Option<u64>,
    pub published_at: Option<String>,
    pub thumbnail_url: Option<String>,
    pub embeddable: Option<bool>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Video {
    pub fn watch_url(&self) -> String {
        format!("https://www.youtube.com/watch?v={}", self.id)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackMode {
    #[default]
    Video,
    Audio,
}

impl PlaybackMode {
    pub fn toggle(self) -> Self {
        match self {
            Self::Video => Self::Audio,
            Self::Audio => Self::Video,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio + thumbnail",
        }
    }
}
