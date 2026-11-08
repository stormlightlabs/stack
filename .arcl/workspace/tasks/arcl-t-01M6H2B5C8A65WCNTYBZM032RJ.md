+++
id = "arcl-t-01M6H2B5C8A65WCNTYBZM032RJ"
parent = "arcl-t-01M6H2B5A7WXHN1S9S9W57BMV7"
title = "Move a repository to a newer train"
status = "pending"
priority = "normal"
position = 1
blocked-by = ["arcl-t-01M6H2B5BP5M44YGAEQAARYBR8"]
+++

A new `sltk` release carries a new train.
Repositories need one command to follow it.

## Done when

`sltk update` writes this binary's train
version into the pin, with the sha256 of each
`sltk` archive copied from the `.sha256` files
on that release, runs `fab update`, then the
glue writers, and shows the diff first as
adoption does. It refuses to move to an older
train. In a test, after `sltk update` and
`sltk install --yes`, `sltk check` passes.

## Not in this task

Installing tools on the machine; `sltk
install` does that.

## Paths

The `update` subcommand.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
