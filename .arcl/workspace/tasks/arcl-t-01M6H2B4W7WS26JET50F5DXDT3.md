+++
id = "arcl-t-01M6H2B4W7WS26JET50F5DXDT3"
parent = "arcl-t-01M6H2B4TM52T6A1YW2V1TVPVB"
title = "Scaffold the sltk workspace and its gate"
status = "completed"
priority = "normal"
position = 0
evidence = "Done in #5 at bb0f6f6: two-crate workspace; sltk --version prints sltk 0.0.0; just check runs fmt, clippy, tests, fab check, arcl workspace import and arcl check; the Check workflow ran just check as the check job and passed on bb0f6f6."
+++

The repository has no code. The spec names
two crates, `sltk-core` and `sltk-cli`, and
the conventions of fabrial and arclightning.

## Done when

`cargo run -p sltk-cli -- --version` prints
`sltk 0.0.0`. `just check` runs fmt --check,
clippy with warnings denied, the tests, `fab
check` and `arcl check`, and passes. A Check
workflow runs `just check` as the `check` job
on pull requests and passes on this one.

## Not in this task

Release workflows. Any command logic.

## Paths

`Cargo.toml`, `Cargo.lock`, `rustfmt.toml`,
`crates/sltk-core/`, `crates/sltk-cli/`,
`justfile`, `.github/workflows/check.yml`,
AGENTS.md's Gates section.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
