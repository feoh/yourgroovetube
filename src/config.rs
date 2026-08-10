use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

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
    #[error("could not parse {path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("could not serialize configuration: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("could not create configuration directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl AppConfig {
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from(&config_path()?)
    }

    fn load_from(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        toml::from_str(&contents).map_err(|source: toml::de::Error| ConfigError::Parse {
            path: path.to_path_buf(),
            message: source.message().to_owned(),
        })
    }

    pub fn youtube_api_key(&self) -> Option<String> {
        env::var(API_KEY_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                self.youtube
                    .api_key
                    .as_ref()
                    .filter(|value| !value.trim().is_empty())
                    .cloned()
            })
    }

    pub fn save(&self) -> Result<PathBuf, ConfigError> {
        let path = config_path()?;
        self.save_to(&path)?;
        Ok(path)
    }

    fn save_to(&self, path: &Path) -> Result<(), ConfigError> {
        let contents = toml::to_string_pretty(self)?;
        let parent = path.parent().ok_or(ConfigError::NoConfigDirectory)?;
        fs::create_dir_all(parent).map_err(|source| ConfigError::CreateDirectory {
            path: parent.to_path_buf(),
            source,
        })?;

        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;

            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|source| ConfigError::Write {
                    path: path.to_path_buf(),
                    source,
                })?;
        }
        file.write_all(contents.as_bytes())
            .map_err(|source| ConfigError::Write {
                path: path.to_path_buf(),
                source,
            })
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
    fn malformed_config_errors_do_not_echo_api_keys() {
        let Ok(directory) = tempfile::tempdir() else {
            panic!("temporary directory should be created");
        };
        let path = directory.path().join("config.toml");
        let secret = "REVIEW_FAKE_SECRET_123";
        if fs::write(&path, format!("[youtube]\napi_key = \"{secret}\n")).is_err() {
            panic!("malformed config fixture should be written");
        }

        let Err(error) = AppConfig::load_from(&path) else {
            panic!("malformed config should fail");
        };
        let message = error.to_string();

        assert!(message.contains("could not parse"));
        assert!(!message.contains(secret));
    }

    #[test]
    fn save_creates_a_restricted_round_trip_config() {
        let Ok(directory) = tempfile::tempdir() else {
            panic!("temporary directory should be created");
        };
        let path = directory.path().join("nested/config.toml");
        let config = AppConfig {
            youtube: YoutubeConfig {
                api_key: Some("saved-key".to_owned()),
                ..YoutubeConfig::default()
            },
            plex: PlexConfig::default(),
        };

        if let Err(error) = config.save_to(&path) {
            panic!("config should save: {error}");
        }
        let Ok(contents) = fs::read_to_string(&path) else {
            panic!("saved config should be readable");
        };
        let Ok(decoded) = toml::from_str::<AppConfig>(&contents) else {
            panic!("saved config should parse");
        };

        assert!(decoded == config);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let Ok(metadata) = fs::metadata(&path) else {
                panic!("saved config metadata should be readable");
            };
            assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn default_plex_directory_matches_requested_library() {
        assert_eq!(
            PlexConfig::default().library_dir,
            PathBuf::from("/nas/video/Saved Youtube Videos")
        );
    }
}
