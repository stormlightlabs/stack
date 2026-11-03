+++
id = "arcl-ph-01M6H2B4RTTBQA2XBDGTEFB0N9"
plan = "arcl-pl-01M6H2B4R310QDMWBXQW9CQ7FT"
title = "v0.1: workspace and machine install"
status = "open"
position = 0
+++

The first cut: `sltk install` puts the pinned fab and arcl, and
mire when asked, on a machine.

Run the scaffold task first. Every other task builds in its
crates and runs under its `just check`. After it, the release
pipeline task and the train task can run at once.
`Cut the 0.1.0 release` runs last.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
