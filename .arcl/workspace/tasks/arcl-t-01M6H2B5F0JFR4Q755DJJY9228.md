+++
id = "arcl-t-01M6H2B5F0JFR4Q755DJJY9228"
parent = "arcl-t-01M6H2B5DNJQDEJ0AVAKFJW534"
title = "Report machine problems with sltk doctor"
status = "completed"
priority = "normal"
position = 0
blocked-by = ["arcl-t-01M6H2B4ZF2SX34E7YSBVGKWFJ"]
evidence = "Done in #14 at 3709fea: sltk doctor reports tools missing from the bin dir, versions other than the train's, a PATH binary ahead of the bin dir (marked when from Homebrew), the bin dir missing from PATH and a pin of another release; fake binaries on a constructed PATH cover each."
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
