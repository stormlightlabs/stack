+++
id = "arcl-t-01M6H2B4X9KF5M9GWPPJ3CJ2ZQ"
phase = "arcl-ph-01M6H2B4RTTBQA2XBDGTEFB0N9"
title = "Release train and machine install"
status = "pending"
priority = "normal"
position = 1
+++

`sltk install` downloads, verifies and installs the
tools the embedded train pins.

## Stop rule

On macOS ARM and Linux x86-64, `sltk install --yes`
leaves fab 0.8.0 and arcl 0.3.0 in `~/.cargo/bin`, and
`--with mire` adds mire. A wrong hash installs nothing.

## Must not absorb

Anything that writes into a repository.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
