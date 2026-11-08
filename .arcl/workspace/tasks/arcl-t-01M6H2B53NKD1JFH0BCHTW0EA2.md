+++
id = "arcl-t-01M6H2B53NKD1JFH0BCHTW0EA2"
parent = "arcl-t-01M6H2B51T9ZV42EXGEQ7GQV5X"
title = "Read and write the repository pin"
status = "completed"
priority = "normal"
position = 1
blocked-by = ["arcl-t-01M6H2B4YC747EJDDWRBYHDFH5"]
evidence = "Done in #9 at 2cdad72: sltk-core reads and writes .github/sltk.toml with exact version and per-platform sha256 lines that the hook's sed reads (tested with real sed), validated optional tools, strict x.y.z versions, and an atomic write."
+++

A repository records its train version,
optional tools and the hashes of the `sltk`
archives in `.github/sltk.toml`.

## Done when

`sltk-core` reads and writes
`.github/sltk.toml` with `version`,
`optional`, `sha256-aarch64-apple-darwin` and
`sha256-x86_64-unknown-linux-gnu`. Writing
then reading returns the same value, and an
unknown tool name is an error that names it.
The written file has `version` as the exact
line `version = "x.y.z"` and each hash as the
exact line `sha256-<target> = "<hex>"`, with
64 lowercase hexadecimal digits, all before
any table, which the session hook reads. A
test checks those lines. Reading a file
without the `version` line or either hash
line is an error that names it.

## Not in this task

Running any tool's setup. Fetching the hashes
from the `sltk` release.

## Paths

`crates/sltk-core/src/pin.rs`.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
