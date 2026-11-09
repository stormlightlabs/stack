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
and each private stormlightlabs repository with `.github/fabrial.toml`
on `main`. `gh repo list stormlightlabs --visibility private` lists
the private repositories to check.

## Must not absorb

Repositories without `.github/fabrial.toml`, including those that use
arcl alone.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
