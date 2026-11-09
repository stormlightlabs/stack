+++
id = "arcl-t-01M6H2B5HXT7RYFSWNKNJQZNDC"
phase = "arcl-ph-01M6H2B5GEK6MTHN1DA9D32D6C"
title = "Adopt the owner's repositories"
status = "pending"
priority = "normal"
position = 0
+++

Every repository that uses fabrial runs on `sltk`.

## Stop rule

`sltk check` passes on `main` in dalil, fabrial, mire, arclightning
and each repository in the private set.

The private set is the private stormlightlabs repositories that have
`.github/fabrial.toml` on `main` when this task starts. A repository
that adopts fabrial later gets its own task and does not reopen this
one. The owner records the set in their private tracker, not here.

Only the owner, or a member with access to the organization, can see
the private set. They list the candidates with
`gh repo list stormlightlabs --visibility private` and keep each one
where
`gh api 'repos/stormlightlabs/<repo>/contents/.github/fabrial.toml?ref=main'`
succeeds.

## Must not absorb

Repositories without `.github/fabrial.toml`, including those that use
arcl alone.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
