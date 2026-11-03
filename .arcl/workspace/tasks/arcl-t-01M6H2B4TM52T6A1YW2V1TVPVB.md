+++
id = "arcl-t-01M6H2B4TM52T6A1YW2V1TVPVB"
phase = "arcl-ph-01M6H2B4RTTBQA2XBDGTEFB0N9"
title = "Workspace and release pipeline"
status = "pending"
priority = "normal"
position = 0
+++

Give `sltk` a Rust workspace, a gate CI runs, and a
release pipeline that publishes it like fab and arcl.

## Stop rule

`just check` passes in CI, `dist plan` passes, and the
Release PR, Release tag and `publish-homebrew.yml`
workflows are on `main`. Cutting a release belongs to
`Cut the 0.1.0 release`.

## Must not absorb

Any `sltk` command beyond `--version` and `--help`.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
