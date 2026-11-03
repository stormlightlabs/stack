+++
id = "arcl-t-01M6H2B5DNJQDEJ0AVAKFJW534"
phase = "arcl-ph-01M6H2B57N6R59TR2REDJNKF01"
title = "Doctor"
status = "pending"
priority = "normal"
position = 1
+++

`sltk doctor` reports what is wrong with the machine
and the repository, and changes nothing.

## Stop rule

On a repository missing the bot or a secret,
`sltk doctor` names it; on a healthy one it reports
nothing to fix.

## Must not absorb

Creating secrets, installing the App, editing rulesets.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
