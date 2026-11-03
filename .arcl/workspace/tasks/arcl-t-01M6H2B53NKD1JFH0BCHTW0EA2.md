+++
id = "arcl-t-01M6H2B53NKD1JFH0BCHTW0EA2"
parent = "arcl-t-01M6H2B51T9ZV42EXGEQ7GQV5X"
title = "Read and write the repository pin"
status = "pending"
priority = "normal"
position = 1
blocked-by = ["arcl-t-01M6H2B4YC747EJDDWRBYHDFH5"]
+++

A repository records its train version and
optional tools in `.github/sltk.toml`.

## Done when

`sltk-core` reads and writes
`.github/sltk.toml` with `version` and
`optional`. Writing then reading returns the
same value, and an unknown tool name is an
error that names it.

## Not in this task

Running any tool's setup.

## Paths

`crates/sltk-core/src/pin.rs`.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
