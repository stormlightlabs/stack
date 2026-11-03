+++
id = "arcl-ph-01M6H2B4RTTBQA2XBDGTEFB0N9"
plan = "arcl-pl-01M6H2B4R310QDMWBXQW9CQ7FT"
title = "v0.1: workspace and machine install"
status = "open"
position = 0
+++

The first cut: `sltk install` puts the pinned fab and arcl, and
mire when asked, on a machine.

Run the workspace unit first. Every other unit builds in its
crates and runs under its `just check`. The release pipeline task
and the train task both edit the workspace `Cargo.toml`, so land
the pipeline before the train.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
