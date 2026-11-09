+++
id = "arcl-t-01M6H2B5MS4CY0VJJH3DF18B56"
parent = "arcl-t-01M6H2B5HXT7RYFSWNKNJQZNDC"
title = "Adopt the private repositories with sltk"
status = "pending"
priority = "normal"
position = 2
blocked-by = ["arcl-t-01M6H2B570Q2DBG1C30CXAGJST"]
+++

Private repositories that use fabrial set up the stack by hand, like
the public ones. This repository is public, so each private
repository's own tracker holds the details of its adoption.

## Done when

Each repository in the private set has merged a PR made with
`sltk init` at a released `sltk`, and `sltk check` passes on its
`main`. The parent task defines the private set, dates it, and says
who can check it and how.

## Not in this task

Changes to those repositories' code beyond what their own adoption
tasks name.

## Paths

None in this repository; each private repository's tracker names
its own.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
