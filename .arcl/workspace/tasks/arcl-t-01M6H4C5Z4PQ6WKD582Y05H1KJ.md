+++
id = "arcl-t-01M6H4C5Z4PQ6WKD582Y05H1KJ"
parent = "arcl-t-01M6H2B51T9ZV42EXGEQ7GQV5X"
title = "Decide how the session hook verifies the sltk download"
status = "pending"
priority = "normal"
position = 0
+++

The session hook installs `sltk` before `sltk`
can check anything. Machine install in the spec
refuses dist's installer script, because dist
publishes a sha256 for each archive but none for
the script. The prototype hook for the version
decision ran `arcl-cli-installer.sh` unchecked,
so the spec does not yet say how the hook checks
what it downloads.

## Done when

A recorded decision: a line under Repository
setup in the spec says how the hook checks the
`sltk` it downloads, and where any hash it
checks against is pinned, with the reason.

## Not in this task

The hook writer in `sltk`. How `sltk install`
checks the tools it installs.

## Paths

The spec file under `.arcl/workspace/specs/`.

Source: the `storm stack` spec
(arcl-s-01M6H17DSG950BTBVMGD9HEHWZ) and the
standard review of PR #4.
