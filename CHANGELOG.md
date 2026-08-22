# Changelog

All notable changes will be documented here.

## Unreleased

## 0.1.3 - 2026-08-22

- Report why a video failed to load instead of mpv's generic
  `unrecognized file format`. Error-level mpv log messages are now requested
  over the same IPC connection, and the first error of a load attempt wins, so
  the yt-dlp reason survives the generic message that mpv sends afterwards.
  Stream URLs are redacted so a signed playback URL cannot reach the status line.
- Document what current YouTube extraction requires, since a missing JavaScript
  runtime or missing `yt-dlp-ejs` challenge solvers both surface as a
  `[stopped]` track, often reported by YouTube as a bot check rather than as the
  underlying runtime problem.
- Note that `cookies_from_browser` needs an explicit profile when the YouTube
  login is not in the browser's `profiles.ini` default profile.

## 0.1.2 - 2026-08-21

- Fix playback that reported itself as playing but stayed frozen at `0:00` with
  no sound. Starting a track now clears a pause left behind by the previous
  one, because mpv's pause flag survives `loadfile` and emits no property
  change when it is already set.
- Report `[stopped]` rather than `[playing]` once mpv has gone idle or its IPC
  connection has dropped, so a failed `yt-dlp` resolution or an exited mpv is no
  longer displayed as active playback.
- Add optional `youtube.cookies_from_browser` configuration that reuses an
  existing browser login for both mpv playback and Plex saving, reducing
  anonymous extraction bot checks. It stays unset by default and is reported by
  `doctor`.

## 0.1.1 - 2026-08-10

- Require a validated YouTube Data API key before starting the TUI, securely
  prompt for a missing key, and save first-run configuration with restrictive
  permissions where supported.
- Accept enhanced-terminal repeat key events and cover search input with
  Ratatui `TestBackend` frame-by-frame tests.

## 0.1.0 - 2026-08-10

- Official YouTube Data API search, metadata hydration, pagination, caching, and
  regional popular-video feed.
- Persistent mpv JSON IPC playback with yt-dlp resolution, pause/resume,
  video/audio mode, and live progress.
- Terminal thumbnails with Kitty, iTerm2, Sixel, and Unicode half-block support.
- Public and unlisted playlist browsing with queue playback.
- Explicit asynchronous save-to-Plex workflow.
- Linux, macOS, and Windows CI plus Rust 1.90 minimum-version validation.
