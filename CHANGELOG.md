# Changelog

All notable changes will be documented here.

## Unreleased

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
