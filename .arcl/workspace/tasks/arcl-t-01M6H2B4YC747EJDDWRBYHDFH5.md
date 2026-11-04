+++
id = "arcl-t-01M6H2B4YC747EJDDWRBYHDFH5"
parent = "arcl-t-01M6H2B4X9KF5M9GWPPJ3CJ2ZQ"
title = "Embed the release train in train.toml"
status = "in_progress"
priority = "normal"
position = 0
blocked-by = ["arcl-t-01M6H2B4W7WS26JET50F5DXDT3"]
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
