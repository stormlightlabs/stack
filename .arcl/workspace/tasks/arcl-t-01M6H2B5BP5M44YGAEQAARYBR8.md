+++
id = "arcl-t-01M6H2B5BP5M44YGAEQAARYBR8"
parent = "arcl-t-01M6H2B5A7WXHN1S9S9W57BMV7"
title = "Report drift with sltk check"
status = "in_progress"
priority = "normal"
position = 0
blocked-by = ["arcl-t-01M6H2B56F01QNVYY5C5K1CPPH"]
+++

`fab check` covers only fabrial's files.
Nothing reports a stale arcl, skill or hook.

## Done when

`sltk check` compares the pin with the
installed tools, runs `fab check` and `arcl
skill install --check`, checks the hook and
Tasks section against what `sltk init` writes,
lists each difference and exits 1 when there
is any. Tests cover a clean repository and one
drift of each kind.

## Not in this task

Fixing the drift.

## Paths

`crates/sltk-core/src/check.rs`, the `check`
subcommand.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
