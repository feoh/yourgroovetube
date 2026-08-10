# Security policy

## Reporting a vulnerability

Please report vulnerabilities through GitHub's private security-advisory flow
for `feoh/yourgroovetube`. Do not open a public issue containing credentials,
signed media URLs, private API responses, filesystem details, or exploit steps
that would put users at immediate risk.

## Supported versions

Until the first tagged release, only the latest commit on `main` receives
security fixes. After releases begin, the latest release and `main` are
supported.

## Sensitive data

`yourgroovetube` does not need YouTube account cookies for its intended design.
The YouTube Data API key may be supplied through
`YOURGROOVETUBE_YOUTUBE_API_KEY` or the local platform configuration file. Users
must protect that file and must never commit it.

The application deliberately restricts thumbnail hosts, uses private local mpv
IPC, avoids shell interpolation, validates yt-dlp staging output, and suppresses
child-process stderr that could contain media URLs. Reports showing a bypass of
those boundaries are security issues.
