+++
id = "arcl-t-01M6H2B557EB9CNG08ACBRPG1P"
phase = "arcl-ph-01M6H2B50P91T7RYNSSN4TZQCZ"
title = "Adopt repositories set up by hand"
status = "pending"
priority = "normal"
position = 1
+++

`sltk init` takes over a repository that already has
fabrial and arcl, showing the diff before it writes.

## Stop rule

In a test, on a copy of a repository set up by hand, `sltk init`
without `--yes` changes nothing, and `sltk init --yes` leaves `fab
check` and `arcl check` passing.

## Must not absorb

Adopting real repositories. stack adopts itself in v0.3, once 0.2.0
is released.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
