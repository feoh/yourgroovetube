use std::collections::VecDeque;
use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;

use thiserror::Error;
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::models::Video;

pub type SaveFuture<'a> = Pin<Box<dyn Future<Output = Result<PathBuf, SaveError>> + Send + 'a>>;

const PROGRESS_PREFIX: &str = "yourgroovetube-progress:";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SaveProgress {
    Preparing,
    Downloading(String),
    Finalizing,
}

#[derive(Debug, Error)]
pub enum SaveError {
    #[error("the Plex destination is unavailable: {0}")]
    Destination(String),
    #[error("yt-dlp could not be started: {0}")]
    Start(String),
    #[error("yt-dlp download failed with exit status {status}: {message}")]
    Download { status: String, message: String },
    #[error("could not read yt-dlp output: {0}")]
    Output(String),
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

    async fn save_inner(
        &self,
        video: &Video,
        progress: Option<mpsc::UnboundedSender<SaveProgress>>,
    ) -> Result<PathBuf, SaveError> {
        send_progress(&progress, SaveProgress::Preparing);
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
        let mut child = Command::new("yt-dlp")
            .args(download_arguments(
                &output_template,
                &video.watch_url(),
                self.cookies_from_browser.as_deref(),
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| SaveError::Start(error.to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SaveError::Output("yt-dlp stdout was not available".to_owned()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| SaveError::Output("yt-dlp stderr was not available".to_owned()))?;
        let (status, stdout, diagnostics) = tokio::join!(
            child.wait(),
            read_stdout(stdout),
            read_stderr(stderr, progress.clone()),
        );
        let status = status.map_err(|error| SaveError::Output(error.to_string()))?;
        let stdout = stdout.map_err(|error| SaveError::Output(error.to_string()))?;
        let diagnostics = diagnostics.map_err(|error| SaveError::Output(error.to_string()))?;
        if !status.success() {
            return Err(SaveError::Download {
                status: status.code().map_or_else(
                    || "terminated by signal".to_owned(),
                    |code| code.to_string(),
                ),
                message: diagnostics
                    .iter()
                    .rev()
                    .find(|line| line.contains("ERROR:"))
                    .or_else(|| diagnostics.back())
                    .cloned()
                    .unwrap_or_else(|| "no diagnostic was reported".to_owned()),
            });
        }
        send_progress(&progress, SaveProgress::Finalizing);
        let output_path = String::from_utf8_lossy(&stdout)
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

    fn save_with_progress<'a>(
        &'a self,
        video: &'a Video,
        progress: mpsc::UnboundedSender<SaveProgress>,
    ) -> SaveFuture<'a> {
        drop(progress);
        self.save(video)
    }
}

impl VideoSaver for YoutubeSaver {
    fn save<'a>(&'a self, video: &'a Video) -> SaveFuture<'a> {
        Box::pin(self.save_inner(video, None))
    }

    fn save_with_progress<'a>(
        &'a self,
        video: &'a Video,
        progress: mpsc::UnboundedSender<SaveProgress>,
    ) -> SaveFuture<'a> {
        Box::pin(self.save_inner(video, Some(progress)))
    }
}

fn send_progress(sender: &Option<mpsc::UnboundedSender<SaveProgress>>, progress: SaveProgress) {
    if let Some(sender) = sender {
        let _ = sender.send(progress);
    }
}

async fn read_stdout(reader: impl AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut reader = BufReader::new(reader);
    reader.read_to_end(&mut output).await?;
    Ok(output)
}

async fn read_stderr(
    reader: impl AsyncRead + Unpin,
    progress: Option<mpsc::UnboundedSender<SaveProgress>>,
) -> std::io::Result<VecDeque<String>> {
    let mut diagnostics = VecDeque::with_capacity(8);
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        if let Some(percent) = parse_progress(&line) {
            send_progress(&progress, SaveProgress::Downloading(percent));
        } else if line.contains("ERROR:") || line.contains("WARNING:") {
            if diagnostics.len() == 8 {
                diagnostics.pop_front();
            }
            diagnostics.push_back(redact_urls(&line));
        }
    }
    Ok(diagnostics)
}

fn parse_progress(line: &str) -> Option<String> {
    let value = line.trim().strip_prefix(PROGRESS_PREFIX)?.trim();
    let percent = value.strip_suffix('%')?.trim().parse::<f64>().ok()?;
    if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
        return None;
    }
    Some(format!("{percent:.1}%"))
}

fn redact_urls(text: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            if word.contains("http://") || word.contains("https://") {
                "[URL]"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn download_arguments(
    output_template: &Path,
    watch_url: &str,
    cookies_from_browser: Option<&str>,
) -> Vec<OsString> {
    let mut arguments: Vec<OsString> = [
        "--no-playlist",
        "--newline",
        "--progress-template",
        "download:yourgroovetube-progress:%(progress._percent_str)s",
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
    fn progress_lines_are_parsed_without_exposing_arbitrary_output() {
        assert_eq!(
            parse_progress("yourgroovetube-progress:  42.5%"),
            Some("42.5%".to_owned())
        );
        assert_eq!(
            parse_progress("[download] https://signed.example/video"),
            None
        );
        assert_eq!(parse_progress("yourgroovetube-progress: unknown"), None);
    }

    #[test]
    fn diagnostics_redact_urls_before_they_can_reach_the_status_line() {
        assert_eq!(
            redact_urls("ERROR: failed https://signed.example/video?token=secret now"),
            "ERROR: failed [URL] now"
        );
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
