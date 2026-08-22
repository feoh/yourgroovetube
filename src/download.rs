use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;

use thiserror::Error;
use tokio::fs;
use tokio::process::Command;

use crate::models::Video;

pub type SaveFuture<'a> = Pin<Box<dyn Future<Output = Result<PathBuf, SaveError>> + Send + 'a>>;

#[derive(Debug, Error)]
pub enum SaveError {
    #[error("the Plex destination is unavailable: {0}")]
    Destination(String),
    #[error("yt-dlp could not be started: {0}")]
    Start(String),
    #[error("yt-dlp download failed with exit status {0}")]
    Download(String),
    #[error("yt-dlp did not report a completed output file")]
    MissingOutput,
    #[error("yt-dlp reported an output outside the private staging directory")]
    UnsafeOutput,
    #[error("could not move the completed file into the Plex library: {0}")]
    Import(String),
}

#[derive(Clone, Debug)]
pub struct YoutubeSaver {
    library_directory: PathBuf,
    cookies_from_browser: Option<String>,
}

impl YoutubeSaver {
    pub fn new(
        library_directory: impl Into<PathBuf>,
        cookies_from_browser: Option<String>,
    ) -> Self {
        Self {
            library_directory: library_directory.into(),
            cookies_from_browser,
        }
    }

    async fn save_inner(&self, video: &Video) -> Result<PathBuf, SaveError> {
        fs::create_dir_all(&self.library_directory)
            .await
            .map_err(|error| SaveError::Destination(error.to_string()))?;
        let library = fs::canonicalize(&self.library_directory)
            .await
            .map_err(|error| SaveError::Destination(error.to_string()))?;
        let staging = library.join(".yourgroovetube-staging");
        fs::create_dir_all(&staging)
            .await
            .map_err(|error| SaveError::Destination(error.to_string()))?;
        let staging = fs::canonicalize(&staging)
            .await
            .map_err(|error| SaveError::Destination(error.to_string()))?;
        if staging.parent() != Some(library.as_path()) {
            return Err(SaveError::UnsafeOutput);
        }

        let base_name = safe_video_name(video);
        let output_template = staging.join(format!("{base_name}.%(ext)s"));
        let output = Command::new("yt-dlp")
            .args(download_arguments(
                &output_template,
                &video.watch_url(),
                self.cookies_from_browser.as_deref(),
            ))
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|error| SaveError::Start(error.to_string()))?;
        if !output.status.success() {
            return Err(SaveError::Download(output.status.code().map_or_else(
                || "terminated by signal".to_owned(),
                |code| code.to_string(),
            )));
        }
        let output_path = String::from_utf8_lossy(&output.stdout)
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .map(str::trim)
            .map(PathBuf::from)
            .ok_or(SaveError::MissingOutput)?;
        let output_path = fs::canonicalize(output_path)
            .await
            .map_err(|_| SaveError::MissingOutput)?;
        if !output_path.starts_with(&staging) || !output_path.is_file() {
            return Err(SaveError::UnsafeOutput);
        }
        let file_name = output_path.file_name().ok_or(SaveError::UnsafeOutput)?;
        let destination = library.join(file_name);
        if fs::try_exists(&destination)
            .await
            .map_err(|error| SaveError::Import(error.to_string()))?
        {
            fs::remove_file(&output_path)
                .await
                .map_err(|error| SaveError::Import(error.to_string()))?;
            return Ok(destination);
        }
        fs::rename(&output_path, &destination)
            .await
            .map_err(|error| SaveError::Import(error.to_string()))?;
        Ok(destination)
    }
}

pub trait VideoSaver: Send + Sync {
    fn save<'a>(&'a self, video: &'a Video) -> SaveFuture<'a>;
}

impl VideoSaver for YoutubeSaver {
    fn save<'a>(&'a self, video: &'a Video) -> SaveFuture<'a> {
        Box::pin(self.save_inner(video))
    }
}

fn download_arguments(
    output_template: &Path,
    watch_url: &str,
    cookies_from_browser: Option<&str>,
) -> Vec<OsString> {
    let mut arguments: Vec<OsString> = [
        "--no-playlist",
        "--no-progress",
        "--format",
        "bestvideo*+bestaudio/best",
        "--merge-output-format",
        "mp4",
        "--remux-video",
        "mp4",
        "--print",
        "after_move:filepath",
        "--output",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    arguments.push(output_template.into());
    if let Some(browser) = cookies_from_browser {
        arguments.push("--cookies-from-browser".into());
        arguments.push(browser.into());
    }
    arguments.push(watch_url.into());
    arguments
}

fn safe_video_name(video: &Video) -> String {
    let mut title = video
        .title
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, ' ' | '-' | '_' | '.' | '(' | ')')
            {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    title = title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(['.', ' '])
        .chars()
        .take(120)
        .collect();
    if title.is_empty() {
        title = "YouTube Video".to_owned();
    }
    format!("{title} [{}]", video.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filenames_remove_template_and_filesystem_metacharacters() {
        let video = Video {
            id: "abc123".to_owned(),
            title: "../100%: great? *video*".to_owned(),
            ..Video::default()
        };

        let name = safe_video_name(&video);

        assert_eq!(name, "_100__ great_ _video_ [abc123]");
        assert!(!name.contains('%'));
        assert!(!name.contains('/'));
    }

    #[test]
    fn cookie_extraction_is_opt_in_and_never_displaces_the_video_url() {
        let template = PathBuf::from("/library/.staging/video.%(ext)s");
        let url = "https://example.test/watch?v=abc";

        let anonymous = download_arguments(&template, url, None);
        let authenticated = download_arguments(&template, url, Some("brave"));

        assert!(
            !anonymous
                .iter()
                .any(|argument| argument == "--cookies-from-browser")
        );
        assert_eq!(
            anonymous.last().map(OsString::as_os_str),
            Some(url.as_ref())
        );
        assert_eq!(
            authenticated.last().map(OsString::as_os_str),
            Some(url.as_ref())
        );
        let flag = authenticated
            .iter()
            .position(|argument| argument == "--cookies-from-browser");
        assert_eq!(
            flag.and_then(|index| authenticated.get(index + 1)),
            Some(&OsString::from("brave"))
        );
    }

    #[tokio::test]
    #[ignore = "downloads a real YouTube video with yt-dlp"]
    async fn downloads_and_imports_a_real_video() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let cookies = crate::config::AppConfig::load()
            .ok()
            .and_then(|config| config.cookies_from_browser());
        let saver = YoutubeSaver::new(directory.path(), cookies);
        let video = Video {
            id: "jNQXAC9IVRw".to_owned(),
            title: "Me at the zoo".to_owned(),
            ..Video::default()
        };

        let saved = saver.save(&video).await?;

        assert!(saved.starts_with(directory.path()));
        assert!(saved.is_file());
        Ok(())
    }
}
