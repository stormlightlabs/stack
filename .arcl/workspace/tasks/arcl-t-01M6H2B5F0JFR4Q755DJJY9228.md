+++
id = "arcl-t-01M6H2B5F0JFR4Q755DJJY9228"
parent = "arcl-t-01M6H2B5DNJQDEJ0AVAKFJW534"
title = "Report machine problems with sltk doctor"
status = "in_progress"
priority = "normal"
position = 0
blocked-by = ["arcl-t-01M6H2B4ZF2SX34E7YSBVGKWFJ"]
+++

A brew-installed tool ahead of the pinned one
on `PATH`, or a missing tool, breaks the
stack without saying why.

## Done when

`sltk doctor` reports missing tools, versions
that differ from the pin, and a Homebrew `fab`,
`arcl` or `mire` ahead of the pinned one on
`PATH`. Tests use fake binaries on a
constructed `PATH`.

## Not in this task

Repository checks through `gh`.

## Paths

`crates/sltk-core/src/doctor.rs`, the
`doctor` subcommand.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
