+++
id = "arcl-t-01M6H2B4WS2TSFRVNYFPDGZYGJ"
parent = "arcl-t-01M6H2B4TM52T6A1YW2V1TVPVB"
title = "Release sltk with cargo-dist and the Homebrew tap"
status = "completed"
priority = "normal"
position = 1
blocked-by = ["arcl-t-01M6H2B4W7WS26JET50F5DXDT3"]
evidence = "Done in #6 at 2a478ff: dist-workspace.toml (cargo-dist 0.32.0, both targets, formula sltk); dist plan passed in CI; Release PR, Release tag and publish-homebrew workflows modelled on fabrial's; first real run is the 0.1.0 cut."
+++

fab, arcl and mire release with cargo-dist
0.32 and push a formula to the shared tap.
`sltk` needs the same, so it can come from
brew and from a shell installer.

## Done when

`dist plan` passes. Release PR and Release
tag workflows, modelled on fabrial's, open a
release pull request that writes CHANGELOG.md
and tag its merge. `publish-homebrew.yml`
pushes `Formula/sltk.rb` with the thunderbot
token. Targets are `aarch64-apple-darwin` and
`x86_64-unknown-linux-gnu`.

## Not in this task

Cutting the 0.1.0 release itself; that is
`Cut the 0.1.0 release`.

## Paths

`dist-workspace.toml`, `.github/workflows/`
(release, release-pr, release-tag,
publish-homebrew), `.github/build-setup.yml`
if needed, `[profile.dist]` in `Cargo.toml`.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
