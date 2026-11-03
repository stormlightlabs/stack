+++
id = "arcl-c-01M6H45BG8J7N2QMTDDSJB3BNW"
title = "Decide how the session hook verifies the sltk download"
status = "captured"
created-at = "2026-11-03T16:34:01.352Z"
+++

The session hook installs `sltk` before `sltk` can check anything. The
spec's Machine install refuses to run dist's installer script for tools,
since dist publishes no sha256 for it, but the hook spike ran
`arcl-cli-installer.sh` unchecked to bootstrap the stand-in. Decide whether
the hook downloads the `sltk` archive and checks a sha256 instead, and where
that hash is pinned.
