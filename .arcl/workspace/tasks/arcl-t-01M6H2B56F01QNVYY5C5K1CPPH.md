+++
id = "arcl-t-01M6H2B56F01QNVYY5C5K1CPPH"
parent = "arcl-t-01M6H2B557EB9CNG08ACBRPG1P"
title = "Adopt a set-up repository with a diff first"
status = "completed"
priority = "normal"
position = 0
blocked-by = ["arcl-t-01M6H2B54RPG8TH5XFD4SNAZR3"]
evidence = "Done in #18 at 04c4f29: sltk init adopts a set-up repository, showing the glue diff and the files fab update will rewrite before writing"
+++

Five repositories already use fabrial, with
drifted pins, a stale skill in mire and a
missing one in dalil.

## Done when

On a repository with `.github/fabrial.toml`,
`sltk init` keeps its settings, runs `fab
update`, replaces a copied hook, rewrites the
Tasks section in place, and runs `arcl skill
install` when `--check` exits 4. Without
`--yes` or a yes it prints the diff and writes
nothing; a test checks the tree is unchanged.

## Not in this task

`sltk update` to a newer train.

## Paths

`crates/sltk-core/src/adopt.rs`, the `init`
subcommand.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
