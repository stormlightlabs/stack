+++
id = "arcl-t-01M6H2B505YH5GKY4W8JEV76MA"
parent = "arcl-t-01M6H2B4X9KF5M9GWPPJ3CJ2ZQ"
title = "Pick optional tools on a terminal"
status = "in_progress"
priority = "normal"
position = 3
blocked-by = ["arcl-t-01M6H2B4ZF2SX34E7YSBVGKWFJ"]
+++

The spec puts flags first: the picker appears
only on a terminal and only when the flags
leave a choice open. mire starts unticked.

## Done when

With no terminal and no flags, `sltk install`
installs the essentials without prompting. On a
terminal with no `--with` or `--yes`, it shows a
picker with fab and arcl fixed and mire
unticked. Tests drive the choice logic through
a fake terminal and a fixture train that lists
mire.

## Not in this task

Prompts for `sltk init`.

## Paths

`crates/sltk-cli/src/prompt.rs`.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
