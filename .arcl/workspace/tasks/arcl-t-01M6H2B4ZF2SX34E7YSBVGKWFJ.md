+++
id = "arcl-t-01M6H2B4ZF2SX34E7YSBVGKWFJ"
parent = "arcl-t-01M6H2B4X9KF5M9GWPPJ3CJ2ZQ"
title = "Install tools from pinned release archives"
status = "completed"
priority = "normal"
position = 2
blocked-by = ["arcl-t-01M6H2B4YXHTGS3010SYPPVR95"]
evidence = "Done in #11 at c213bf8: sltk install --yes downloads each archive into memory, checks the train's sha256, extracts only the binary and renames it into CARGO_HOME/bin; --with adds optional tools; local HTTP fixture tests cover good and bad hashes; a real run installed fab 0.8.0 and arcl 0.3.0."
+++

dist publishes a sha256 for each archive but
none for its installer script, so `sltk`
downloads archives and checks them itself.

## Done when

`sltk install --yes` downloads each essential
tool's archive, checks it against the train's
sha256, and unpacks the binary into
`~/.cargo/bin` (or `$CARGO_HOME/bin`).
`--with mire` adds mire from a fixture train
that lists it. An integration test against a
local HTTP fixture installs on a good hash and
writes nothing on a bad one.

## Not in this task

The interactive picker. Repository setup.

## Paths

`crates/sltk-core/src/install.rs`, the
`install` subcommand, `crates/*/tests/`.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
