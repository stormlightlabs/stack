+++
id = "arcl-t-01M6H2B5A7WXHN1S9S9W57BMV7"
phase = "arcl-ph-01M6H2B57N6R59TR2REDJNKF01"
title = "Drift and train updates"
status = "pending"
priority = "normal"
position = 0
+++

Report drift and move repositories to newer trains.

## Stop rule

`sltk check` exits 1 on any drift and passes after `sltk update`
and `sltk install --yes`, and a train PR runs the smoke test in
CI.

## Must not absorb

Doctor's checks.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
