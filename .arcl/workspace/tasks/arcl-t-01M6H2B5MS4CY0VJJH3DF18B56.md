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

## Done when

A PR in stormlightlabs/cloud made with `sltk init` at a released
`sltk` is merged, and `sltk check` passes on its `main`.
`scripts/dev-setup.sh` installs fab and arcl through
`sltk` and keeps its other tools.

## Not in this task

Changes to the repository's own code.

## Paths

`cloud/scripts/dev-setup.sh`, `cloud/.claude/`, `cloud/AGENTS.md`,
`cloud/.github/sltk.toml`.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
