+++
id = "arcl-t-01M6H2NG81PEP6PQGMMW1E51PZ"
phase = "arcl-ph-01M6H2B50P91T7RYNSSN4TZQCZ"
title = "Cut the 0.2.0 release"
status = "pending"
priority = "normal"
position = 2
blocked-by = ["arcl-t-01M6H2B5327WG5DQR0Q41P2DWZ", "arcl-t-01M6H2B53NKD1JFH0BCHTW0EA2", "arcl-t-01M6H2B546VDAZG9A2RKMMQK00", "arcl-t-01M6H2B54RPG8TH5XFD4SNAZR3", "arcl-t-01M6H2B56F01QNVYY5C5K1CPPH"]
+++

`sltk` 0.2.0 reaches people as a tag, a GitHub release and a
Homebrew formula. The release workflows make them once every
task of the phase has landed.

## Done when

The `v0.2.0` tag exists on `main`, its GitHub release has the
dist archives, `stormlightlabs/homebrew-tap` has the 0.2.0
formula for `sltk`, and the arcl release 0.2.0 is completed.

## Not in this task

Any change to `sltk`'s code. The next release.

## Paths

`CHANGELOG.md`, through the release PR. The release record
under `.arcl/workspace/releases/`.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
