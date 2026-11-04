+++
id = "arcl-t-01M6H2B4YC747EJDDWRBYHDFH5"
parent = "arcl-t-01M6H2B4X9KF5M9GWPPJ3CJ2ZQ"
title = "Embed the release train in train.toml"
status = "completed"
priority = "normal"
position = 0
blocked-by = ["arcl-t-01M6H2B4W7WS26JET50F5DXDT3"]
evidence = "Done in #7 at 4d3f325: train.toml pins fab 0.8.0 and arcl 0.3.0 for both platforms with hashes from dist's .sha256 files; sltk-core parses and validates it; sltk train prints the pinned set; mire follows in arcl-t-01M6J62CDRJY6YQR5NEZG913RR."
+++

Each repository pins tool versions by hand,
and the pins have drifted (fab 0.8.0, 0.7.1,
0.4.0). The spec puts one tested set in
`train.toml`, embedded in the binary.

## Done when

`train.toml` lists fab and arcl, each with
tier, binary, version, a download URL
template, platforms, the sha256 of each
platform's archive, copied from dist's
`.sha256` files, and setup commands: `fab
init` for fab, `arcl init --workspace` and
`arcl skill install` for arcl. mire has no
release yet, so it joins in the follow-up task
"Add mire to the release train". `sltk-core`
parses it, and `sltk train` prints the pinned
set. Tests reject a train with a missing hash
or an unknown tier.

## Not in this task

Downloading anything. Running the setup
commands. The mire entry.

## Paths

`train.toml`, `crates/sltk-core/src/train.rs`,
the `train` subcommand in `crates/sltk-cli/`.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
