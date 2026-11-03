+++
id = "arcl-t-01M6H2B5327WG5DQR0Q41P2DWZ"
parent = "arcl-t-01M6H2B51T9ZV42EXGEQ7GQV5X"
title = "Decide how the session hook learns the sltk version"
status = "in_progress"
priority = "normal"
position = 0
+++

The spec leaves this open: the hook must
install `sltk` before `sltk` can read
`.github/sltk.toml`. Options are a shell
one-liner that reads the file, or a version
written into the hook by `sltk init`.

No `sltk` release exists yet, so the prototype
installs arcl 0.3.0 in its place: a binary from
an existing GitHub release, at a pinned version.

## Done when

A recorded decision: the spec's Open item is
replaced by a line under Repository setup, with
the reason. A prototype hook that installs the
pinned arcl 0.3.0 binary from its GitHub release
ran in one Claude cloud session, and the result
is in the PR comment.

## Not in this task

The hook writer in `sltk`.

## Paths

The spec file under `.arcl/workspace/specs/`.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
