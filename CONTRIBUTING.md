# Contributing

Contributions are welcome. Please keep catalog access, playback, thumbnail
rendering, and downloads behind their existing component boundaries.

## Development

Install Rust 1.90 or newer, mpv, and yt-dlp, then run:

```console
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo package --locked
```

`pre-commit run --all-files` runs the standard local validation sequence.
Tests marked ignored require mpv, network access, or a real YouTube download and
must remain opt-in.

## YouTube boundary

Read [`docs/youtube-api-and-policy.md`](docs/youtube-api-and-policy.md) before
changing discovery, playback, audio-only, or download behavior. Never describe
mpv/yt-dlp extraction as an official YouTube API. Do not commit API keys, OAuth
tokens, cookies, signed media URLs, or captured private responses.

Use direct child-process argument arrays rather than shell interpolation. Treat
video titles, API metadata, playlist URLs, output paths, and redirects as
untrusted input.
