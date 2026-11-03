+++
id = "arcl-t-01M6H2B546VDAZG9A2RKMMQK00"
parent = "arcl-t-01M6H2B51T9ZV42EXGEQ7GQV5X"
title = "Run fab init and arcl setup from sltk init"
status = "pending"
priority = "normal"
position = 2
blocked-by = ["arcl-t-01M6H2B4ZF2SX34E7YSBVGKWFJ", "arcl-t-01M6H2B53NKD1JFH0BCHTW0EA2"]
+++

`fab init` needs `--owner`, `--trailer` and
`--gates`. arcl needs `arcl init --workspace`
and `arcl skill install`.

## Done when

`sltk init` takes the three fab flags, asks
for missing ones on a terminal, runs `fab init`,
`arcl init --workspace` and `arcl skill
install`, and writes the pin. An integration
test with fake `fab` and `arcl` on `PATH`
checks the calls and their arguments.

## Not in this task

The Tasks section and the hook.

## Paths

`crates/sltk-core/src/setup.rs`, the `init`
subcommand.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
