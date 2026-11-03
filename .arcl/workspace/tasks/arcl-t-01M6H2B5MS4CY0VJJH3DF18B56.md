+++
id = "arcl-t-01M6H2B5MS4CY0VJJH3DF18B56"
parent = "arcl-t-01M6H2B5HXT7RYFSWNKNJQZNDC"
title = "Adopt cloud with sltk"
status = "pending"
priority = "normal"
position = 2
blocked-by = ["arcl-t-01M6H2B570Q2DBG1C30CXAGJST"]
+++

cloud's `scripts/dev-setup.sh` pins fab and arcl a third way.
`scripts/check-nix.sh` lines 17-25 read `fab_version` and
`arcl_version` from it and compare them with the pins in
`conf/worker/default.nix`, so dropping those lines breaks the
check unless it reads the versions from `sltk` instead.

## Done when

A PR in stormlightlabs/cloud made with `sltk init` at a released
`sltk` is merged, and `sltk check` passes on its `main`.
`scripts/dev-setup.sh` installs fab and arcl through
`sltk` and keeps its other tools. `scripts/check-nix.sh` still
passes, comparing the worker image's fab and arcl pins with the
versions `sltk` pins.

## Not in this task

Changes to cloud's code beyond these pins and the check that
compares them.

## Paths

`cloud/scripts/dev-setup.sh`, `cloud/scripts/check-nix.sh`,
`cloud/conf/worker/default.nix`, `cloud/.claude/`,
`cloud/AGENTS.md`, `cloud/.github/sltk.toml`.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
