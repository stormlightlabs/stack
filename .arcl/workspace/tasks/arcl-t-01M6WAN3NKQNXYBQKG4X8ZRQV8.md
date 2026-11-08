+++
id = "arcl-t-01M6WAN3NKQNXYBQKG4X8ZRQV8"
phase = "arcl-ph-01M6H2B4RTTBQA2XBDGTEFB0N9"
title = "Make stormlightlabs/stack public"
status = "pending"
priority = "normal"
position = 1
+++

The session hook, the Homebrew formula and the
README's install line fetch `sltk` release
assets without a token, which a private
repository refuses. The owner decided to make
`stormlightlabs/stack` public before 0.1.0.
The owner does this task: it is a settings
change.

## Done when

`gh api repos/stormlightlabs/stack` reports
`"private": false`.

## Not in this task

Release assets and the cut itself.

## Paths

None; the repository's settings.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
