use std::env;
use std::fs;
use std::path::PathBuf;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const API_KEY_ENV: &str = "YOURGROOVETUBE_YOUTUBE_API_KEY";

#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AppConfig {
    #[serde(default)]
    pub youtube: YoutubeConfig,
    #[serde(default)]
    pub plex: PlexConfig,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct YoutubeConfig {
    pub api_key: Option<String>,
    #[serde(default = "default_region_code")]
    pub region_code: String,
    #[serde(default = "default_results_per_page")]
    pub results_per_page: u8,
}

impl Default for YoutubeConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            region_code: default_region_code(),
            results_per_page: default_results_per_page(),
        }
    }
}

fn default_region_code() -> String {
    "US".to_owned()
}

const fn default_results_per_page() -> u8 {
    25
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlexConfig {
    pub library_dir: PathBuf,
}

impl Default for PlexConfig {
    fn default() -> Self {
        Self {
            library_dir: PathBuf::from("/nas/video/Saved Youtube Videos"),
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not determine the platform configuration directory")]
    NoConfigDirectory,
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not parse {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
}

impl AppConfig {
    pub fn load() -> Result<Self, ConfigError> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(&path).map_err(|source| ConfigError::Read {
            path: path.clone(),
            source,
        })?;
        toml::from_str(&contents).map_err(|source| ConfigError::Parse { path, source })
    }

    pub fn youtube_api_key(&self) -> Option<String> {
        env::var(API_KEY_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| self.youtube.api_key.clone())
    }
}

pub fn config_path() -> Result<PathBuf, ConfigError> {
    ProjectDirs::from("com", "feoh", "yourgroovetube")
        .map(|dirs| dirs.config_dir().join("config.toml"))
        .ok_or(ConfigError::NoConfigDirectory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips_as_toml() {
        let config = AppConfig {
            youtube: YoutubeConfig {
                api_key: Some("local-development-key".to_owned()),
                region_code: "CA".to_owned(),
                results_per_page: 40,
            },
            plex: PlexConfig::default(),
        };

        let Ok(encoded) = toml::to_string(&config) else {
            panic!("config should serialize");
        };
        let Ok(decoded) = toml::from_str::<AppConfig>(&encoded) else {
            panic!("config should parse");
        };

        assert!(decoded == config);
    }

    #[test]
    fn default_plex_directory_matches_requested_library() {
        assert_eq!(
            PlexConfig::default().library_dir,
            PathBuf::from("/nas/video/Saved Youtube Videos")
        );
    }
}
