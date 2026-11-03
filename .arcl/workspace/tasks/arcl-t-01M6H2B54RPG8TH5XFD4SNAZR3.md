+++
id = "arcl-t-01M6H2B54RPG8TH5XFD4SNAZR3"
parent = "arcl-t-01M6H2B51T9ZV42EXGEQ7GQV5X"
title = "Write the Tasks section and the session hook"
status = "pending"
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
time. The hook installs `sltk`, runs `sltk
install --yes` and builds the arcl cache. A
test runs the written hook with a locally built
`sltk` in place of the download and finds arcl
installed.

## Not in this task

Replacing a hand-copied hook (adoption).

## Paths

`crates/sltk-core/src/glue.rs`, templates
under `crates/sltk-core/templates/`.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
