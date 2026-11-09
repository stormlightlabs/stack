+++
id = "arcl-t-01M6H2B54RPG8TH5XFD4SNAZR3"
parent = "arcl-t-01M6H2B51T9ZV42EXGEQ7GQV5X"
title = "Write the Tasks section and the session hook"
status = "in_progress"
priority = "normal"
position = 3
blocked-by = ["arcl-t-01M6H2B5327WG5DQR0Q41P2DWZ", "arcl-t-01M6H2B546VDAZG9A2RKMMQK00", "arcl-t-01M6H4C5Z4PQ6WKD582Y05H1KJ"]
+++

dalil, fabrial and stack each carry a
hand-copied hook and Tasks section that pin
arcl twice. `sltk init` writes one of each.

## Done when

`sltk init` writes a marker-delimited Tasks
section into AGENTS.md, `.claude/hooks/
session-start.sh` and its entry in
`.claude/settings.json`, as the spike decided.
Running it twice changes nothing the second
time. `sltk init` writes the sha256 of each
`sltk` archive into the pin, read from the
`.sha256` files on the release of the version
it pins through a hash source the tests
replace with a fixture. The hook downloads the
`sltk-cli-<target>.tar.xz` archive for the
target `uname -sm` names, checks it against
that target's `sha256-<target>` line in
`.github/sltk.toml` and never runs dist's
installer script. It fails when the line is
missing or the hash differs. It hashes with
`sha256sum`, or `shasum -a 256` where that is
missing, and fails naming both when neither
is on `PATH`. Then it runs `sltk install
--yes` and builds the arcl cache. A test runs
the written hook with a locally built `sltk`
archive in place of the download and finds
arcl installed. Further tests find that the
hook fails on an archive with another hash
and with neither hashing tool on `PATH`.
`Smoke-test train changes in CI` covers
reading a real release.

## Not in this task

Replacing a hand-copied hook (adoption).

## Paths

`crates/sltk-core/src/glue.rs`, templates
under `crates/sltk-core/templates/`.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
