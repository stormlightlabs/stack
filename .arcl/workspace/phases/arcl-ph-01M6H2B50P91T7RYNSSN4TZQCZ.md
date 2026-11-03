+++
id = "arcl-ph-01M6H2B50P91T7RYNSSN4TZQCZ"
plan = "arcl-pl-01M6H2B4R310QDMWBXQW9CQ7FT"
title = "v0.2: repository setup"
status = "open"
position = 1
+++

`sltk init` sets up a fresh repository and adopts one already set
up by hand.

The hook spike can run now: the hook writer depends on its
answer. The setup unit lands before the adoption unit, which
reuses its writers. Both units write AGENTS.md and `.claude/`, so
do not run them at once. `Cut the 0.2.0 release` runs last.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
