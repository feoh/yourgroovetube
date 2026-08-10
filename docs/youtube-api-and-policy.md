# YouTube API and policy boundary

_Last reviewed: 2026-08-10. This is a project risk statement, not legal advice._

## Official APIs used where possible

The project uses the supported YouTube Data API v3 for:

- title/tag discovery through [`search.list`](https://developers.google.com/youtube/v3/docs/search/list);
- duration, status, and metadata hydration through
  [`videos.list`](https://developers.google.com/youtube/v3/docs/videos/list);
- regional default content through `chart=mostPopular`; and
A later milestone may add user-authorized subscription metadata through
Google's installed application OAuth flow.

`search.list` costs 100 quota units per call, while `videos.list` costs one.
Search is therefore explicit rather than request-per-keystroke. The official API
does **not** expose a user's personalized YouTube Home recommendation feed.

## Unofficial playback and download

The official APIs expose neither raw media URLs nor terminal-native playback.
YouTube's supported player is the web-based IFrame Player API. This project
instead follows the user's explicit product decision to let mpv invoke a
`yt-dlp`-compatible extractor for playback and downloads.

That path is unofficial and conflicts with restrictions described in the
[YouTube API Services Developer Policies](https://developers.google.com/youtube/terms/developer-policies)
and [YouTube Terms of Service](https://www.youtube.com/static?template=terms),
including restrictions around non-API access, separating audio/video, and
copying audiovisual content. It may also break when YouTube changes its site.
Installing the extractor separately does not remove those risks.

Contributors and users are responsible for understanding applicable terms and
content rights. The project must not claim endorsement by YouTube or describe
extractor-backed playback as a supported YouTube API integration. Before each
public release, maintainers should re-read the linked policies and record any
material changes.
