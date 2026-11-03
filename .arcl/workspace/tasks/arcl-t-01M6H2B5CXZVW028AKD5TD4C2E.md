+++
id = "arcl-t-01M6H2B5CXZVW028AKD5TD4C2E"
parent = "arcl-t-01M6H2B5A7WXHN1S9S9W57BMV7"
title = "Smoke-test train changes in CI"
status = "pending"
priority = "normal"
position = 2
blocked-by = ["arcl-t-01M6H2B5BP5M44YGAEQAARYBR8"]
+++

The spec gates each train PR on an install,
an `sltk init` in a temporary repository and
an `sltk check`.

## Done when

A workflow runs on PRs that change
`train.toml`: it builds `sltk`, runs `sltk
install --yes --with mire`, `sltk init` in a
temporary repository and `sltk check`, and fails
when any step does. A deliberately wrong hash
on a test branch makes it fail.

## Not in this task

Opening train PRs automatically.

## Paths

`.github/workflows/train.yml`.

Source: the `storm stack` spec (arcl-s-01M6H17DSG950BTBVMGD9HEHWZ).
