# Architecture

## Goals

`yourgroovetube` is a keyboard-driven terminal client with four replaceable
boundaries:

1. **Video catalog** — the official YouTube Data API v3 supplies search results,
   metadata, thumbnails, and a non-personalized default feed.
2. **Media resolver/playback engine** — `mpv` remains persistent and is
   controlled with newline-delimited JSON over a private local IPC endpoint.
   mpv delegates YouTube URL extraction to `yt-dlp`.
3. **Thumbnail renderer** — terminal capability detection chooses Kitty,
   iTerm2, Sixel, or a Unicode fallback. Audio mode keeps this image visible
   while mpv video output is disabled.
4. **Video saver** — an explicit action downloads to a temporary partial file
   and atomically moves a completed, safely named file into the configured Plex
   library directory.

The traits in `provider.rs`, `playback.rs`, and `download.rs` prevent YouTube
access policy, TUI state, mpv process control, and filesystem import from
collapsing into one component.

## Discovery

Search uses `search.list(type=video)` only after explicit submission because it
costs 100 quota units per call. Returned IDs are hydrated in a single
`videos.list(part=snippet,contentDetails,status)` call, which costs one unit.
Pages are cached in memory for five minutes and pagination is explicit. Public
and unlisted playlists use `playlistItems.list(part=contentDetails)` followed by
the same ordered metadata hydration; private playlists remain out of scope until
OAuth support exists. API errors deliberately omit request URLs so query-string
credentials cannot leak through error displays.

The YouTube Data API does not expose the signed-in user's personalized Home
recommendations. The default screen therefore uses
`videos.list(chart=mostPopular)` for the configured two-letter region. A later
OAuth milestone may offer an app-defined feed derived from subscriptions,
clearly labeled as such.

## Playback

The playback engine starts one `mpv --idle=yes` process with an IPC socket in a
random, private temporary directory. A dedicated reader thread parses
newline-delimited JSON, records command failures, and publishes observed
`time-pos`, `duration`, `pause`, `eof-reached`, and `idle-active` properties.
The UI refreshes the shared snapshot at ten frames per second.

Video mode allows mpv to render normally. Audio mode sets `vid=no` and pins the
currently playing video's thumbnail in the TUI. The renderer detects Kitty,
iTerm2, and Sixel support through `ratatui-image`, with a Unicode half-block
fallback and a `--no-images` override. Playlist playback builds an ordered queue
from hydrated playlist items, advances on mpv's EOF event, and supports manual
previous/next controls. Mode is a playback concern rather than a search or
catalog concern.

## Plex import

Saving is never automatic. The user must press the save key while a video is
playing. The implementation will:

- verify the destination is inside the configured library root;
- download to a unique partial file outside the final path;
- sanitize the title and include the YouTube video ID;
- move the completed file atomically where the filesystem permits; and
- preserve the partial file and report a useful error when recovery is possible.

The default destination is `/nas/video/Saved Youtube Videos`.

## Security

- API credentials and OAuth tokens must never be committed.
- mpv JSON IPC is unauthenticated and command-capable, so its endpoint must be
  local, private, unpredictable, and deleted at shutdown.
- Thumbnail fetching accepts only credential-free HTTPS URLs on `ytimg.com`
  hosts, revalidates redirects, and rejects responses larger than 10 MiB.
- Video titles are untrusted input and must never become unsanitized shell or
  filesystem arguments.
- Child processes receive argument arrays directly; no shell interpolation.
