+++
id = "arcl-s-01M6H17DSG950BTBVMGD9HEHWZ"
title = "storm stack"
status = "open"
+++

`sltk` installs the storm stack onto a machine and sets it up in a repository.
The stack is fabrial (`fab`) and arclightning (`arcl`), with Mire (`mire`) as
an option. Each tool keeps its own installer and its own setup command; `sltk`
pins a set of their versions and runs them together.

## Problem

Each repository that uses the stack sets it up by hand, and the copies have
drifted:

- `.github/fabrial.toml` pins fab 0.8.0 in dalil, fabrial and cloud, 0.7.1 in
  mire and 0.4.0 in arclightning.
- `arcl skill install` was never run in dalil, and mire's copy of the
  arclightning skill differs from the one in arclightning's `skills/`.
- dalil and fabrial each carry a `.claude/hooks/session-start.sh` copied by
  hand; the two differ only in comments. Each pins arcl 0.3.0 twice: in the
  script, and in dalil's AGENTS.md Tasks section or fabrial's PLAN.md Work
  section.
- cloud's `scripts/dev-setup.sh` pins fab and arcl a third way.

Each binary already has a one-line install. fab, arcl and mire all release
with cargo-dist 0.32, publish a shell installer, push a formula to
`stormlightlabs/homebrew-tap` and install into `~/.cargo/bin`. What is missing
is one pinned set of versions and one command that sets a repository up from
it.

## Decisions

### The tool

- `sltk` is its own tool in `stormlightlabs/stack`, separate from fab.
  fabrial's PLAN.md decides that fabrial depends on no tracker and links no
  tracker's code, so code that combines fab with arcl does not belong there.
- The binary is `sltk`. Haskell's Stack already installs `stack`, and `stk` is
  taken in Homebrew. `sltk` is free in Homebrew and on crates.io.
- It is written in Rust, in two crates. `sltk-core` reads the release train
  and the repository pin and plans what to install or write. `sltk-cli` runs
  the plan and asks the questions. Planning apart from running is what lets
  adoption show its diff before it changes anything.
- It follows the conventions of arclightning and fabrial: the 2024 edition,
  their `rustfmt.toml`, rustdoc on exported items, cargo-dist releases with
  shell and Homebrew installers, conventional-commit PR titles, and the
  Apache-2.0 license.

### Tools and tiers

- The first release covers fab, arcl and mire.
- fab and arcl are essentials: they always install, and every repository set
  up by `sltk` has both.
- mire is optional. It installs only when asked, with `--with mire` or by
  ticking it in the picker, where it starts unticked. It needs no setup in a
  repository.
- A tool joins with an entry in the release train: its name, tier, binary,
  download URL template, platforms, the sha256 of each platform's archive and
  setup commands. dalil can join that
  way once it has a release. A tool that needs written glue, as arcl needs the
  Tasks section, also needs code in `sltk`.
- A repository set up by `sltk` cannot use fab with a tracker other than
  arcl. fab still supports that when run on its own.

### Release train

- An `sltk` release pins one tested version of each tool. Its source holds
  them in `train.toml`, which the binary embeds, so the `sltk` version and the
  train version are one number. A new train needs a new `sltk` release.
- Moving to a new train is a pull request that edits `train.toml`. CI on that
  pull request installs the train, runs `sltk init` in a temporary repository,
  then runs `sltk check`.

### Machine install

- `sltk install` downloads each tool's release archive for the platform from
  the release the train pins, checks it against the sha256 the train pins, and
  unpacks the binary into `~/.cargo/bin`. It does not run dist's installer
  script: dist publishes a sha256 for each archive but none for the script.
- A train pull request copies each archive's sha256 from the `.sha256` file
  dist publishes beside it. Pinning the hash in `train.toml` also catches an
  asset uploaded again after the train was tested.
- A machine holds one version of each tool. `sltk check` reports when a
  repository's pin differs from what is installed.
- Tools come from their GitHub releases, never from Homebrew: the tap holds
  only each tool's latest formula, so it cannot honor a pin. `sltk` itself can
  come from the tap.
- `sltk` supports macOS on Apple silicon and Linux on x86-64, the two targets
  every tool builds. On any other platform it names the tools it cannot
  install instead of failing partway.

### Repository setup

- A repository records its pin in `.github/sltk.toml`: the train version and
  the optional tools it chose. The essentials are implied.
- `sltk init` runs each tool's own setup: `fab init`, `arcl init --workspace`
  and `arcl skill install`. `fab init` requires `--owner`, `--trailer` and
  `--gates`, so on a fresh repository `sltk init` takes the same three flags,
  asks for any left out when on a terminal, and passes them to `fab init`.
- `sltk init` writes only the glue itself: the AGENTS.md Tasks section and
  one session hook. Writing fab's or arcl's files directly would copy their
  templates, and those copies are the drift this tool removes.
- The session hook installs the pinned `sltk`, runs `sltk install --yes` from
  `.github/sltk.toml`, then builds the arcl cache. It replaces the hand-copied
  `session-start.sh`. cloud's `dev-setup.sh` calls `sltk` for fab and arcl and
  keeps installing its other tools itself.
- `sltk init` adopts a repository that is already set up. It keeps the
  settings in `.github/fabrial.toml` and runs `fab update`, replaces a copied
  `session-start.sh`, and rewrites the Tasks section in place. It runs
  `arcl skill install --check`, which exits 4 when a skill file is missing or
  differs, and runs `arcl skill install` when it does. It shows the diff and
  changes nothing without `--yes` or a yes at the prompt.
- `sltk update` moves a repository to a newer train. `sltk check` reports
  drift across every tool and exits 1 when there is any.
- The stack repository adopts itself with `sltk init` once the CLI can.

### Doctor

- `sltk doctor` reports missing tools, wrong versions and a `fab`, `arcl` or
  `mire` from Homebrew that comes before the pinned one on `PATH`.
- When `gh` is installed it also checks the repository: stormlight-thunderbot
  installed, `THUNDERBOT_APP_ID` and `THUNDERBOT_PRIVATE_KEY` available, and
  `storm/review` required on `main`. Without `gh` it skips these and says so.
- It reports and changes nothing. Installing the App and sharing its secrets
  stay with the owner, who holds those credentials.

### Running without a person

- Every command takes flags for every choice. The picker appears only on a
  terminal and only when the flags leave a choice open. Session hooks and CI
  run `sltk` more often than people do.

### Git host

- v1 works with GitHub only, but the code keeps the host behind one
  interface. A tool's download URL is a template in the train, not a GitHub
  repository name, and `doctor`'s repository checks go through a `Host` trait
  whose one implementation is GitHub.
- Installs and `doctor` can move to another host by adding a `Host`
  implementation and new URL templates. fabrial cannot: its workflows,
  rulesets and App are GitHub's, so a repository set up by `sltk` stays tied
  to GitHub until fabrial changes.

## Out of scope

- Windows, Intel macOS and Linux on ARM. fab and mire do not build them.
- Holding several versions of a tool on one machine, as rustup or mise do with
  a versioned store and shims.
- Opening release-train pull requests automatically when a tool releases.
- Creating secrets, installing the App or editing rulesets.
- Tools beyond fab, arcl and mire.

## Open

- The session hook installs `sltk` before `.github/sltk.toml` can be read by
  `sltk`. The hook needs the version some other way: read the file with a
  shell one-liner, or write the version into the hook when `sltk init` writes
  it. A spike of the hook settles it.
- Whether `sltk install` on a Mac with Homebrew warns about, or offers to
  remove, a brew-installed `fab`, `arcl` or `mire`. The first adoption on the
  owner's Mac settles it.

## Sources

- `fabrial/PLAN.md`, decision "A task source owns the work, and fabrial
  depends on none": fabrial names no tracker and links no tracker's code.
- `fabrial/dist-workspace.toml`, `arclightning/dist-workspace.toml`,
  `mire/dist-workspace.toml`: cargo-dist 0.32, the shared tap, `CARGO_HOME`,
  and each tool's targets.
- `arclightning/crates/arcl-cli/src/app/skill.rs`: `arcl skill install` writes
  the skill into `.claude/skills` and `.agents/skills`; `--check` writes
  nothing and exits 4 when a file is missing or differs.
- `fab init --help` in fab 0.8.0: `--owner`, `--trailer` and `--gates` are
  required.
- The assets of arcl's v0.3.0 release: a `.sha256` file beside each archive,
  none beside `arcl-cli-installer.sh`, and a `sha256.sum` that lists no
  installer.
- `.github/fabrial.toml` in each repository: the fab version each pins.
- `dalil/.claude/hooks/session-start.sh`,
  `fabrial/.claude/hooks/session-start.sh` and `cloud/scripts/dev-setup.sh`:
  the hand-copied pins.
- formulae.brew.sh and crates.io, checked 2026-11-03: `stk` is a Homebrew
  formula and a crate; `sltk` is neither.
