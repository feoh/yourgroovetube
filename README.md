# yourgroovetube

A keyboard-driven terminal YouTube viewer built with Rust and
[Ratatui](https://ratatui.rs/).

> [!IMPORTANT]
> Search and metadata will use the official YouTube Data API wherever possible.
> Playback and downloads use mpv with a `yt-dlp`-compatible extractor and are
> **not** supported YouTube API features. They carry policy, content-rights, and
> maintenance risks. Read [the policy boundary](docs/youtube-api-and-policy.md).

## Vision

`yourgroovetube` is designed for fast keyboard discovery and playback:

- search video titles and tags;
- start from a regional popular-video feed (the official API does not expose
  personalized YouTube Home recommendations);
- toggle between normal video and audio with the thumbnail retained in the TUI;
- show elapsed time and progress while mpv plays; and
- explicitly save the current video into
  `/nas/video/Saved Youtube Videos` for Plex to discover.

## Current status

The repository contains a runnable application with live, official YouTube Data
API discovery, persistent mpv playback, terminal thumbnails, YouTube playlist
queues, and explicit Plex-library saving. Packaging and release polish remain.

The application already includes:

- a regional `mostPopular` default feed;
- explicit title/tag search with batched metadata hydration;
- five-minute in-memory result caching and explicit pagination;
- persistent mpv playback controlled through newline-delimited JSON IPC;
- yt-dlp-backed YouTube URL resolution, pause/resume, and video/audio switching;
- observed playback position, duration, end state, and IPC errors;
- automatic Kitty, iTerm2, Sixel, or Unicode half-block thumbnails;
- audio mode that keeps the current video's thumbnail visible while `vid=no`;
- public/unlisted playlist loading with pagination, automatic queue advancement,
  and manual previous/next controls;
- asynchronous, explicit yt-dlp saving into the configured Plex library;
- keyboard/search state and a responsive Ratatui layout;
- a live now-playing progress gauge;
- platform-standard configuration loading;
- `doctor` and `config path` commands; and
- tests plus GitHub Actions validation.

## Requirements

- Rust 1.90 or newer when building from source
- [`mpv`](https://mpv.io/) on `PATH`
- [`yt-dlp`](https://github.com/yt-dlp/yt-dlp) on `PATH`
- a YouTube Data API v3 key for discovery

## Install

Install the latest source revision with Cargo:

```console
cargo install --git https://github.com/feoh/yourgroovetube
```

Tagged releases publish Linux x86-64, macOS Apple Silicon, and Windows x86-64
archives on the [GitHub Releases](https://github.com/feoh/yourgroovetube/releases)
page.

## Build and run

```console
git clone https://github.com/feoh/yourgroovetube.git
cd yourgroovetube
cargo run
```

Force portable Unicode half-block thumbnails when terminal image detection is
unwanted:

```console
cargo run -- --no-images
```

Inspect prerequisites and the configuration path:

```console
cargo run -- doctor
cargo run -- config path
```

Set the API key without writing it to disk:

```console
export YOURGROOVETUBE_YOUTUBE_API_KEY='your-key'
cargo run
```

Alternatively, create the file printed by `yourgroovetube config path`:

```toml
[youtube]
api_key = "your-key"
region_code = "US"
results_per_page = 25

[plex]
library_dir = "/nas/video/Saved Youtube Videos"
```

Do not commit API keys, tokens, cookies, or captured YouTube responses.

## Planned keybindings

| Key | Action |
| --- | --- |
| `/` | Search by title or tags |
| `j`/`k` or arrows | Select a video |
| `n` | Load the next result or playlist page |
| `P` | Open a public/unlisted playlist URL or ID |
| `[` / `]` | Play the previous/next loaded playlist video |
| `Enter` or `p` | Play the selected video |
| `m` | Toggle video / audio-with-thumbnail mode |
| `Space` | Pause or resume |
| `s` | Save the current video to the Plex directory |
| `?` | Show keyboard help |
| `q` | Quit |

## Implementation ranking

| Stack | Runtime performance | Ease of implementation | Decision |
| --- | ---: | ---: | --- |
| Rust + Ratatui | 1 | 3 | **Selected** — best runtime/distribution and proven local patterns |
| Go + Bubble Tea | 2 | 2 | Good compromise, less reusable project code |
| Python + Textual | 3 | 1 | Fastest prototype, higher runtime and packaging overhead |

For this network/process-bound application, all three would be responsive. Rust
was selected for efficient long-running playback, a single distributable binary,
and direct control over mpv IPC and terminal image lifetimes.

See [the architecture](docs/architecture.md) for component boundaries and the
quota-conscious API plan.

## Development

```console
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

## License

MIT. See [LICENSE](LICENSE).
