+++
id = "arcl-t-01M6H2B4YXHTGS3010SYPPVR95"
parent = "arcl-t-01M6H2B4X9KF5M9GWPPJ3CJ2ZQ"
title = "Detect the platform and name what it cannot install"
status = "completed"
priority = "normal"
position = 1
blocked-by = ["arcl-t-01M6H2B4YC747EJDDWRBYHDFH5"]
evidence = "Done in #8 at b3b5a9b: sltk-core maps the build target to a supported platform (64-bit gnu Linux x86-64 or Apple silicon) or names it unsupported; the install plan lists unsupported tools before any download; sltk install exits 1 with that list."
+++

fab and mire build only macOS ARM and Linux
x86-64; arcl builds more. The spec says `sltk`
names the tools a platform cannot get instead
of failing partway.

## Done when

`sltk-core` maps the host to a target triple.
For a platform a tool has no archive for, the
install plan lists that tool as unsupported
before downloading anything, and `sltk install`
exits non-zero with that list. Unit tests cover
both supported targets and one unsupported one.

## Not in this task

Building from source on other platforms.

## Paths

`crates/sltk-core/src/platform.rs`.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
