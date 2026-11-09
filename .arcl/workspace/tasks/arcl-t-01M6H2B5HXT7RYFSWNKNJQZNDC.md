+++
id = "arcl-t-01M6H2B5HXT7RYFSWNKNJQZNDC"
phase = "arcl-ph-01M6H2B5GEK6MTHN1DA9D32D6C"
title = "Adopt the owner's repositories"
status = "pending"
priority = "normal"
position = 0
+++

Every repository that uses fabrial runs on `sltk`.

## Stop rule

`sltk check` passes on `main` in dalil, fabrial, mire, arclightning
and each private repository that uses fabrial.

## Must not absorb

Repositories that use neither fabrial nor arcl.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
