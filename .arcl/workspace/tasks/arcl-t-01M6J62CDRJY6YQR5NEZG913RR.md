+++
id = "arcl-t-01M6J62CDRJY6YQR5NEZG913RR"
parent = "arcl-t-01M6H2B5A7WXHN1S9S9W57BMV7"
title = "Add mire to the release train"
status = "parked"
priority = "normal"
position = 3
blocked-by = ["arcl-t-01M6H2B4ZF2SX34E7YSBVGKWFJ"]
+++

mire is optional in the spec, but it had no
tags or releases on 2026-11-04 (checked with
the GitHub releases API), so the first train
pins only fab and arcl.

## Parked

Waits on mire's first GitHub release; mire
had none as of 2026-11-04. Whoever cuts that
release unparks this task with `arcl task
unpark arcl-t-01M6J62CDRJY6YQR5NEZG913RR`.

## Done when

`train.toml` has a mire entry: tier
`optional`, no setup commands, and an archive
for both supported platforms, Linux x86-64 and
macOS on Apple silicon, each with the sha256
copied from dist's `.sha256` file. `sltk
install --yes --with mire` installs it on
Linux x86-64; the pinned hash covers macOS.
The tests pass.

## Not in this task

Cutting mire's release, which is the owner's.

## Paths

`train.toml`.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
