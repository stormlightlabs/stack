+++
id = "arcl-c-01M6Z5CJ88Y3AY79AWETM1ZRWR"
title = "Install the pin's optional tools in the session hook"
status = "captured"
created-at = "2026-11-09T03:24:48.264Z"
+++

The session hook runs `sltk install --yes`, which installs fab and arcl and
any `--with` tool. It does not read `optional` in `.github/sltk.toml`, so a
repository that pins mire gets no mire in a cloud session. The spec says the
hook runs `sltk install --yes` from the pin. Either `sltk install` reads the
pin's optional tools when it finds one, or the hook passes them as `--with`.

Found while writing the hook (arcl-t-01M6H2B54RPG8TH5XFD4SNAZR3).
