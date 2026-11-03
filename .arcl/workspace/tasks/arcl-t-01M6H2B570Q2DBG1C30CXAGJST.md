+++
id = "arcl-t-01M6H2B570Q2DBG1C30CXAGJST"
parent = "arcl-t-01M6H2B557EB9CNG08ACBRPG1P"
title = "Adopt the stack repository with sltk init"
status = "pending"
priority = "normal"
position = 1
blocked-by = ["arcl-t-01M6H2B56F01QNVYY5C5K1CPPH"]
+++

stack's hook and Tasks section were copied
from dalil at bootstrap. The spec says stack
adopts itself once the CLI can.

## Done when

A PR in stack whose changes come only from
`sltk init --yes` at the released 0.2.0, with
`.github/sltk.toml` added. A cloud session on
the branch gets arcl from the new hook.

## Not in this task

Other repositories.

## Paths

`AGENTS.md`, `.claude/hooks/`,
`.claude/settings.json`, `.github/sltk.toml`.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
