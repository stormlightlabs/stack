+++
id = "arcl-t-01M6H2B51T9ZV42EXGEQ7GQV5X"
phase = "arcl-ph-01M6H2B50P91T7RYNSSN4TZQCZ"
title = "Fresh repository setup"
status = "pending"
priority = "normal"
position = 0
+++

`sltk init` runs each tool's own setup and writes the
pin, the Tasks section and the session hook.

## Stop rule

In an empty git repository, `sltk init --owner o
--trailer 'N <e>' --gates 'just check' --yes` leaves a
repository where `fab check` and `arcl check` pass, and
a cloud session there gets arcl from the hook.

## Must not absorb

Adopting a repository that is already set up.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
