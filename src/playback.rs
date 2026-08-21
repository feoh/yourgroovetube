use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::{Value, json};
use tempfile::TempDir;
use thiserror::Error;

use crate::models::{PlaybackMode, Video};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaybackSnapshot {
    pub current: Option<Video>,
    pub mode: PlaybackMode,
    pub paused: bool,
    pub position_seconds: f64,
    pub duration_seconds: f64,
    pub connected: bool,
    pub idle: bool,
    pub eof_reached: bool,
    pub last_error: Option<String>,
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
    #[error("could not create a private mpv runtime directory: {0}")]
    RuntimeDirectory(#[source] std::io::Error),
    #[error("mpv could not be started; install mpv and ensure it is on PATH: {0}")]
    Start(#[source] std::io::Error),
    #[error("could not inspect the mpv process: {0}")]
    Process(#[source] std::io::Error),
    #[error("could not connect to mpv IPC at {path}: {source}")]
    Connect {
        path: String,
        source: std::io::Error,
    },
    #[error("could not encode an mpv command: {0}")]
    Encode(#[from] serde_json::Error),
    #[error("could not send a command to mpv: {0}")]
    Command(#[source] std::io::Error),
    #[error("mpv playback state is unavailable")]
    State,
}

pub trait PlaybackEngine {
    fn play(&mut self, video: &Video, mode: PlaybackMode) -> Result<(), PlaybackError>;
    fn set_paused(&mut self, paused: bool) -> Result<(), PlaybackError>;
    fn set_mode(&mut self, mode: PlaybackMode) -> Result<(), PlaybackError>;
    fn snapshot(&self) -> Result<PlaybackSnapshot, PlaybackError>;
    fn stop(&mut self) -> Result<(), PlaybackError>;
}

pub struct MpvEngine {
    child: Option<Child>,
    writer: Option<Box<dyn Write + Send>>,
    reader: Option<JoinHandle<()>>,
    runtime_directory: Option<TempDir>,
    ipc_path: Option<String>,
    next_request_id: u64,
    cookies_from_browser: Option<String>,
    snapshot: Arc<Mutex<PlaybackSnapshot>>,
}

impl Default for MpvEngine {
    fn default() -> Self {
        Self::new(None)
    }
}

impl MpvEngine {
    pub fn new(cookies_from_browser: Option<String>) -> Self {
        Self {
            child: None,
            writer: None,
            reader: None,
            runtime_directory: None,
            ipc_path: None,
            next_request_id: 100,
            cookies_from_browser,
            snapshot: Arc::new(Mutex::new(PlaybackSnapshot {
                idle: true,
                ..PlaybackSnapshot::default()
            })),
        }
    }

    fn ensure_started(&mut self) -> Result<(), PlaybackError> {
        if let Some(child) = self.child.as_mut() {
            match child.try_wait().map_err(PlaybackError::Process)? {
                None if self.writer.is_some() => return Ok(()),
                None | Some(_) => self.cleanup(),
            }
        }

        let runtime_directory = tempfile::Builder::new()
            .prefix("yourgroovetube-mpv-")
            .tempdir()
            .map_err(PlaybackError::RuntimeDirectory)?;
        let ipc_path = ipc_endpoint(&runtime_directory);
        let mut child = Command::new("mpv")
            .args(mpv_arguments(
                &ipc_path,
                self.cookies_from_browser.as_deref(),
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(PlaybackError::Start)?;

        let mut last_error = std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "mpv IPC endpoint was not created",
        );
        for _ in 0..100 {
            match connect_ipc(&ipc_path) {
                Ok((reader, writer)) => {
                    self.child = Some(child);
                    self.writer = Some(writer);
                    self.reader = Some(read_messages(reader, Arc::clone(&self.snapshot)));
                    self.runtime_directory = Some(runtime_directory);
                    self.ipc_path = Some(ipc_path);
                    self.update_snapshot(|snapshot| {
                        snapshot.connected = true;
                        snapshot.idle = true;
                        snapshot.last_error = None;
                    })?;
                    if let Err(error) = self.initialize_observers() {
                        self.cleanup();
                        return Err(error);
                    }
                    return Ok(());
                }
                Err(error) => last_error = error,
            }
            if child.try_wait().map_err(PlaybackError::Process)?.is_some() {
                return Err(PlaybackError::Start(std::io::Error::other(
                    "mpv exited before its IPC endpoint became available",
                )));
            }
            thread::sleep(Duration::from_millis(20));
        }

        let _ = child.kill();
        let _ = child.wait();
        Err(PlaybackError::Connect {
            path: ipc_path,
            source: last_error,
        })
    }

    fn initialize_observers(&mut self) -> Result<(), PlaybackError> {
        for (observer_id, property) in [
            (1, "time-pos"),
            (2, "duration"),
            (3, "pause"),
            (4, "idle-active"),
            (5, "eof-reached"),
        ] {
            self.write_command(json!(["observe_property", observer_id, property]))?;
        }
        Ok(())
    }

    fn command(&mut self, command: Value) -> Result<(), PlaybackError> {
        self.ensure_started()?;
        self.write_command(command)
    }

    fn write_command(&mut self, command: Value) -> Result<(), PlaybackError> {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        let mut payload = command_payload(command, request_id)?;
        payload.push(b'\n');
        let writer = self.writer.as_mut().ok_or_else(|| {
            PlaybackError::Command(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "mpv IPC connection is unavailable",
            ))
        })?;
        writer
            .write_all(&payload)
            .and_then(|()| writer.flush())
            .map_err(PlaybackError::Command)
    }

    fn update_snapshot(
        &self,
        update: impl FnOnce(&mut PlaybackSnapshot),
    ) -> Result<(), PlaybackError> {
        let mut snapshot = self.snapshot.lock().map_err(|_| PlaybackError::State)?;
        update(&mut snapshot);
        Ok(())
    }

    fn process_running(&mut self) -> Result<bool, PlaybackError> {
        let Some(child) = self.child.as_mut() else {
            return Ok(false);
        };
        match child.try_wait().map_err(PlaybackError::Process)? {
            None => Ok(true),
            Some(_) => {
                self.cleanup();
                Ok(false)
            }
        }
    }

    fn cleanup(&mut self) {
        self.writer = None;
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        self.ipc_path = None;
        self.runtime_directory = None;
        if let Ok(mut snapshot) = self.snapshot.lock() {
            snapshot.connected = false;
            snapshot.idle = true;
        }
    }
}

impl PlaybackEngine for MpvEngine {
    fn play(&mut self, video: &Video, mode: PlaybackMode) -> Result<(), PlaybackError> {
        self.ensure_started()?;
        for command in play_commands(&video.watch_url(), mode) {
            self.write_command(command)?;
        }
        self.update_snapshot(|snapshot| {
            snapshot.current = Some(video.clone());
            snapshot.mode = mode;
            snapshot.paused = false;
            snapshot.position_seconds = 0.0;
            snapshot.duration_seconds = video.duration_seconds.unwrap_or_default() as f64;
            snapshot.idle = false;
            snapshot.eof_reached = false;
            snapshot.last_error = None;
        })
    }

    fn set_paused(&mut self, paused: bool) -> Result<(), PlaybackError> {
        if !self.process_running()? {
            return Ok(());
        }
        self.command(json!(["set_property", "pause", paused]))?;
        self.update_snapshot(|snapshot| snapshot.paused = paused)
    }

    fn set_mode(&mut self, mode: PlaybackMode) -> Result<(), PlaybackError> {
        if self.process_running()? {
            self.command(json!(["set_property", "vid", mode.mpv_vid()]))?;
        }
        self.update_snapshot(|snapshot| snapshot.mode = mode)
    }

    fn snapshot(&self) -> Result<PlaybackSnapshot, PlaybackError> {
        self.snapshot
            .lock()
            .map(|snapshot| snapshot.clone())
            .map_err(|_| PlaybackError::State)
    }

    fn stop(&mut self) -> Result<(), PlaybackError> {
        if !self.process_running()? {
            return Ok(());
        }
        self.command(json!(["stop"]))?;
        self.update_snapshot(|snapshot| {
            snapshot.current = None;
            snapshot.paused = false;
            snapshot.position_seconds = 0.0;
            snapshot.duration_seconds = 0.0;
            snapshot.idle = true;
        })
    }
}

impl Drop for MpvEngine {
    fn drop(&mut self) {
        self.cleanup();
    }
}

impl PlaybackMode {
    fn mpv_vid(self) -> &'static str {
        match self {
            Self::Video => "auto",
            Self::Audio => "no",
        }
    }
}

fn mpv_arguments(ipc_path: &str, cookies_from_browser: Option<&str>) -> Vec<String> {
    let mut arguments = vec![
        "--idle=yes".to_owned(),
        "--no-terminal".to_owned(),
        "--really-quiet".to_owned(),
        "--ytdl=yes".to_owned(),
        "--script-opts=ytdl_hook-ytdl_path=yt-dlp".to_owned(),
        format!("--input-ipc-server={ipc_path}"),
    ];
    if let Some(browser) = cookies_from_browser {
        // -append takes one key/value pair verbatim, so a profile or container
        // suffix cannot be mistaken for a second mpv option.
        arguments.push(format!(
            "--ytdl-raw-options-append=cookies-from-browser={browser}"
        ));
    }
    arguments
}

// mpv's pause flag belongs to the player rather than the file, so it survives
// loadfile, and an unchanged property emits no property-change to observe. A
// pause left over from the previous track would therefore load the new one into
// silence that nothing ever reports.
fn play_commands(url: &str, mode: PlaybackMode) -> [Value; 3] {
    [
        json!(["set_property", "pause", false]),
        json!(["set_property", "vid", mode.mpv_vid()]),
        json!(["loadfile", url, "replace"]),
    ]
}

fn command_payload(command: Value, request_id: u64) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&json!({
        "command": command,
        "request_id": request_id,
    }))
}

fn read_messages(
    reader: Box<dyn Read + Send>,
    snapshot: Arc<Mutex<PlaybackSnapshot>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => match serde_json::from_str::<Value>(&line) {
                    Ok(message) => {
                        if let Ok(mut snapshot) = snapshot.lock() {
                            apply_mpv_message(&mut snapshot, &message);
                        } else {
                            break;
                        }
                    }
                    Err(_) => {
                        if let Ok(mut snapshot) = snapshot.lock() {
                            snapshot.last_error =
                                Some("mpv returned malformed JSON over IPC".to_owned());
                        }
                        break;
                    }
                },
                Err(error) => {
                    if let Ok(mut snapshot) = snapshot.lock() {
                        snapshot.last_error = Some(format!("mpv IPC read failed: {error}"));
                    }
                    break;
                }
            }
        }
        if let Ok(mut snapshot) = snapshot.lock() {
            snapshot.connected = false;
        }
    })
}

fn apply_mpv_message(snapshot: &mut PlaybackSnapshot, message: &Value) {
    match message.get("event").and_then(Value::as_str) {
        Some("property-change") => apply_property_change(snapshot, message),
        Some("start-file") | Some("file-loaded") => {
            snapshot.idle = false;
            snapshot.eof_reached = false;
        }
        Some("end-file") => {
            snapshot.idle = true;
            snapshot.paused = false;
            if message.get("reason").and_then(Value::as_str) == Some("eof") {
                snapshot.eof_reached = true;
            }
            if message.get("reason").and_then(Value::as_str) == Some("error") {
                snapshot.last_error = Some(
                    message
                        .get("file_error")
                        .and_then(Value::as_str)
                        .unwrap_or("mpv could not play this video")
                        .to_owned(),
                );
            }
        }
        Some("shutdown") => snapshot.connected = false,
        _ => {
            if let Some(error) = message
                .get("error")
                .and_then(Value::as_str)
                .filter(|error| *error != "success")
            {
                snapshot.last_error = Some(format!("mpv command failed: {error}"));
            }
        }
    }
}

fn apply_property_change(snapshot: &mut PlaybackSnapshot, message: &Value) {
    let Some(name) = message.get("name").and_then(Value::as_str) else {
        return;
    };
    let data = message.get("data").unwrap_or(&Value::Null);
    match name {
        "time-pos" => snapshot.position_seconds = data.as_f64().unwrap_or_default(),
        "duration" => snapshot.duration_seconds = data.as_f64().unwrap_or_default(),
        "pause" => snapshot.paused = data.as_bool().unwrap_or_default(),
        "idle-active" => snapshot.idle = data.as_bool().unwrap_or_default(),
        "eof-reached" => snapshot.eof_reached = data.as_bool().unwrap_or_default(),
        _ => {}
    }
}

#[cfg(unix)]
fn ipc_endpoint(runtime_directory: &TempDir) -> String {
    runtime_directory
        .path()
        .join("mpv.sock")
        .to_string_lossy()
        .into_owned()
}

#[cfg(windows)]
fn ipc_endpoint(runtime_directory: &TempDir) -> String {
    let unique = runtime_directory
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("yourgroovetube");
    format!(r"\\.\pipe\{unique}")
}

#[cfg(not(any(unix, windows)))]
fn ipc_endpoint(runtime_directory: &TempDir) -> String {
    runtime_directory
        .path()
        .join("mpv.ipc")
        .to_string_lossy()
        .into_owned()
}

#[cfg(unix)]
fn connect_ipc(path: &str) -> std::io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    let stream = std::os::unix::net::UnixStream::connect(path)?;
    let reader = stream.try_clone()?;
    Ok((Box::new(reader), Box::new(stream)))
}

#[cfg(windows)]
fn connect_ipc(path: &str) -> std::io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    let pipe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    let reader = pipe.try_clone()?;
    Ok((Box::new(reader), Box::new(pipe)))
}

#[cfg(not(any(unix, windows)))]
fn connect_ipc(_path: &str) -> std::io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "mpv IPC is unsupported on this platform",
    ))
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

    #[test]
    fn mpv_commands_use_newline_delimited_json_ipc_payloads() {
        let Ok(mut payload) = command_payload(
            json!(["loadfile", "https://example.test/a b", "replace"]),
            42,
        ) else {
            panic!("command should encode");
        };
        payload.push(b'\n');
        let Ok(decoded) = serde_json::from_slice::<Value>(&payload) else {
            panic!("command should be valid JSON");
        };

        assert_eq!(
            decoded,
            json!({
                "command": ["loadfile", "https://example.test/a b", "replace"],
                "request_id": 42,
            })
        );
    }

    #[test]
    fn property_events_update_progress_and_pause_state() {
        let mut snapshot = PlaybackSnapshot::default();

        apply_mpv_message(
            &mut snapshot,
            &json!({"event": "property-change", "name": "time-pos", "data": 12.5}),
        );
        apply_mpv_message(
            &mut snapshot,
            &json!({"event": "property-change", "name": "duration", "data": 50.0}),
        );
        apply_mpv_message(
            &mut snapshot,
            &json!({"event": "property-change", "name": "pause", "data": true}),
        );

        assert_eq!(snapshot.position_seconds, 12.5);
        assert_eq!(snapshot.duration_seconds, 50.0);
        assert!(snapshot.paused);
        assert_eq!(snapshot.progress_ratio(), 0.25);
    }

    #[test]
    fn cookie_extraction_is_requested_only_when_a_browser_is_configured() {
        let anonymous = mpv_arguments("/tmp/mpv.sock", None);
        let authenticated = mpv_arguments("/tmp/mpv.sock", Some("firefox:default"));

        assert!(anonymous.contains(&"--input-ipc-server=/tmp/mpv.sock".to_owned()));
        assert!(
            !anonymous
                .iter()
                .any(|argument| argument.contains("cookies-from-browser"))
        );
        assert_eq!(
            authenticated.last().map(String::as_str),
            Some("--ytdl-raw-options-append=cookies-from-browser=firefox:default")
        );
    }

    #[test]
    #[ignore = "requires mpv on PATH"]
    fn mpv_process_exposes_a_live_json_ipc_connection() -> Result<(), PlaybackError> {
        let mut engine = MpvEngine::new(None);

        engine.ensure_started()?;
        thread::sleep(Duration::from_millis(100));
        let snapshot = engine.snapshot()?;

        assert!(snapshot.connected);
        assert!(snapshot.idle);
        assert_eq!(snapshot.last_error, None);
        Ok(())
    }

    #[test]
    fn end_file_errors_are_visible_without_exposing_stream_urls() {
        let mut snapshot = PlaybackSnapshot::default();

        apply_mpv_message(
            &mut snapshot,
            &json!({
                "event": "end-file",
                "reason": "error",
                "file_error": "loading failed"
            }),
        );

        assert_eq!(snapshot.last_error.as_deref(), Some("loading failed"));
        assert!(snapshot.idle);
    }

    #[test]
    fn playing_releases_a_pause_left_behind_by_the_previous_track() {
        let commands = play_commands("https://example.test/watch?v=abc", PlaybackMode::Video);

        assert_eq!(commands[0], json!(["set_property", "pause", false]));
        assert_eq!(commands[1], json!(["set_property", "vid", "auto"]));
        assert_eq!(
            commands[2],
            json!(["loadfile", "https://example.test/watch?v=abc", "replace"])
        );
    }

    #[test]
    fn a_failed_load_reports_neither_playback_nor_a_finished_file() {
        let mut snapshot = PlaybackSnapshot::default();

        apply_mpv_message(&mut snapshot, &json!({"event": "start-file"}));
        apply_mpv_message(
            &mut snapshot,
            &json!({"event": "end-file", "reason": "error", "file_error": "unrecognized file format"}),
        );
        apply_mpv_message(
            &mut snapshot,
            &json!({"event": "property-change", "name": "idle-active", "data": true}),
        );

        assert!(snapshot.idle);
        assert!(!snapshot.eof_reached);
    }
}
