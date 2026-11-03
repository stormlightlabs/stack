+++
id = "arcl-t-01M6H2MVMRF8AZPZ74AW39TASJ"
phase = "arcl-ph-01M6H2B57N6R59TR2REDJNKF01"
title = "Run stack on sltk"
status = "pending"
priority = "normal"
position = 2
+++

stack runs on the files a released `sltk init` writes, instead of
the hook and Tasks section copied from dalil at bootstrap.

## Stop rule

stack's `main` carries `.github/sltk.toml` and a hook written by
`sltk init` at the released 0.2.0, merged through its own PR.

## Must not absorb

Adopting repositories other than stack. Changes to `sltk` itself.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
