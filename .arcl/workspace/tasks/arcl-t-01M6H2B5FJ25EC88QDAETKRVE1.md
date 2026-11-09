+++
id = "arcl-t-01M6H2B5FJ25EC88QDAETKRVE1"
parent = "arcl-t-01M6H2B5DNJQDEJ0AVAKFJW534"
title = "Check the repository through a Host trait"
status = "completed"
priority = "normal"
position = 1
blocked-by = ["arcl-t-01M6H2B5F0JFR4Q755DJJY9228"]
evidence = "Done in #16 at 1ec3922: sltk doctor checks the App, its secrets and storm/review on main through a Host trait whose GitHub implementation uses gh"
+++

The spec keeps the git host behind one
interface, with GitHub as its only
implementation, so installs and doctor can
move hosts later.

## Done when

A `Host` trait in `sltk-core` with a GitHub
implementation that uses `gh`. `sltk doctor`
reports whether stormlight-thunderbot is
installed, `THUNDERBOT_APP_ID` and
`THUNDERBOT_PRIVATE_KEY` are available, and
`storm/review` is required on `main`. Without
`gh` it skips these and says so. Tests use a
fake `Host`.

## Not in this task

A second host.

## Paths

`crates/sltk-core/src/host.rs`, `doctor.rs`.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
