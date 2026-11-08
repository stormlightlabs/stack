+++
id = "arcl-c-01M6WAB52VFZDCGDPY1BQVNXVP"
title = "Make sltk releases downloadable without a token"
status = "captured"
created-at = "2026-11-08T00:53:41.595Z"
+++

`stormlightlabs/stack` is private (GitHub API, checked 2026-11-08). Release
assets of a private repository need an authenticated request, so the session
hook's `curl`, the Homebrew formula and the README's install line cannot fetch
an `sltk` archive until the repository is public or they send a token. Decide
which before the 0.1.0 release.
