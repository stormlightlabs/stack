+++
id = "arcl-t-01M6H2B5Q0NS72C43EB3JCH2Z7"
parent = "arcl-t-01M6H2B5HXT7RYFSWNKNJQZNDC"
title = "Decide how sltk install treats brew-installed tools"
status = "pending"
priority = "normal"
position = 5
blocked-by = ["arcl-t-01M6H2B5F0JFR4Q755DJJY9228", "arcl-t-01M6H2B5KZV1QBVFKGRKMWXS5H"]
+++

The spec leaves open whether `sltk install` warns about, or
offers to remove, a brew-installed `fab`, `arcl` or `mire`.
The first adoption on the owner's Mac settles it. That is the
fabrial adoption, so this task waits for it.

## Done when

A recorded decision: the spec's Open item is replaced by a
line under Machine install, with what the fabrial adoption on
the owner's Mac showed. If the decision needs code, a task for
it is filed.

## Not in this task

Implementing the decision.

## Paths

The spec file under `.arcl/workspace/specs/`.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
