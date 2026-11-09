//! `sltk check`: where a repository has drifted from what `sltk init` sets
//! up. It compares the pin with the installed tools through [`doctor`], runs
//! `fab check` and `arcl skill install --check`, and compares the glue with
//! what [`glue::plan`] would write. It writes nothing, fixes nothing and asks
//! nothing.
//!
//! Every input comes in through the repository root and a [`Machine`], so
//! tests build a repository, a bin directory and a `PATH` of their own.

use std::fmt;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::adopt::{FAB_CHECK, FAB_DRIFT, SKILL_CHECK, SKILL_DIFFERS};
use crate::doctor::{self, Finding, Machine, RepoPin};
use crate::glue::{self, Change, Existing, GlueError};
use crate::pin;
use crate::process::run_capped;
use crate::setup::{self, FAB_SETTINGS};
use crate::train::Train;

/// How long `fab check` or `arcl skill install --check` may run.
pub const TOOL_TIMEOUT: Duration = Duration::from_secs(120);

/// The most bytes read from each of a tool check's stdout and stderr.
const MAX_OUTPUT: u64 = 1 << 20;

/// One way the repository differs from what `sltk init` sets up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drift {
    /// The repository has no `.github/sltk.toml`.
    NoPin,
    /// The pin names another `sltk` release, and so another train.
    PinVersion {
        /// The release the pin names.
        pinned: String,
        /// This binary's release.
        running: String,
    },
    /// The pin is a symlink, not a regular file, or too large, so it was not
    /// read.
    PinRefused {
        /// Why, as ".github/sltk.toml is a symlink".
        reason: String,
    },
    /// A tool on this machine differs from the pin, as [`doctor::check`]
    /// finds it.
    Tool(Finding),
    /// A line `fab check` printed when it exited with [`FAB_DRIFT`].
    Fab(String),
    /// A skill file `arcl skill install --check` named when it exited with
    /// [`SKILL_DIFFERS`], as "missing .claude/skills/arclightning/SKILL.md".
    Skill(String),
    /// A glue file differs from what `sltk init` writes.
    Glue {
        /// The file, relative to the repository root.
        path: &'static str,
        /// How it differs, as "is missing".
        how: &'static str,
    },
    /// A glue file holds something `sltk init` would refuse to replace.
    GlueRefused {
        /// The file and the reason, as `glue` words them.
        reason: String,
    },
}

impl fmt::Display for Drift {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Drift::NoPin => write!(f, "{}: missing; sltk init writes it", pin::PATH),
            Drift::PinVersion { pinned, running } => write!(
                f,
                "{}: pins sltk {pinned}, but this is sltk {running}; sltk check compares the tools sltk {running} pins",
                pin::PATH
            ),
            Drift::PinRefused { reason } => write!(f, "{reason}; sltk check read no pin"),
            Drift::Tool(finding) => write!(f, "{finding}"),
            Drift::Fab(line) => write!(f, "fab check: {line}"),
            Drift::Skill(line) => write!(f, "arcl skill install --check: {line}"),
            Drift::Glue { path, how } => write!(f, "{path}: {how}"),
            Drift::GlueRefused { reason } => write!(f, "{reason}"),
        }
    }
}

/// What `sltk check` found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Each difference, in the order found: the pin and tools, `fab check`,
    /// the arcl skill, then the glue.
    pub drift: Vec<Drift>,
    /// Checks that could not run or finish, as "could not run fab check:
    /// ...". Each one may hide drift.
    pub errors: Vec<String>,
}

/// Checks the repository at `root` against `train`, with `running` as this
/// `sltk` release and the tools as `machine` finds them. `fab` and `arcl`
/// run in `root`, found on `machine.path` and given it as `PATH`.
pub fn check(train: &Train, running: &str, root: &Path, machine: &Machine) -> Report {
    let mut report = Report::default();
    let pin = read_pin(train, running, root, &mut report);
    report.drift.extend(
        doctor::check(train, pin.as_ref(), running, machine)
            .findings
            .into_iter()
            .map(Drift::Tool),
    );
    check_fab(root, machine, &mut report);
    check_skill(root, machine, &mut report);
    check_glue(root, &mut report);
    report
}

/// How a glue change differs from the file: what `sltk init` would do to
/// the file at `change.path`, from the file's point of view.
pub fn describe(change: &Change) -> &'static str {
    let mode_only = change.old.as_deref() == Some(change.new.as_str());
    match change.path {
        _ if change.old.is_none() => "is missing",
        glue::HOOK if mode_only => "is not executable",
        glue::HOOK => "differs from the hook sltk init writes",
        glue::SETTINGS => "has no SessionStart entry that runs the hook as sltk init writes it",
        FAB_SETTINGS => "has no [tasks] table for arcl",
        _ => "has a Tasks section that differs from the one sltk init writes, or none",
    }
}

/// Reads the pin under `root` for [`doctor::check`]: a pin of this release,
/// or `None` after recording why there is none to compare with.
fn read_pin(train: &Train, running: &str, root: &Path, report: &mut Report) -> Option<RepoPin> {
    if std::fs::symlink_metadata(root.join(pin::PATH)).is_err() {
        report.drift.push(Drift::NoPin);
        return None;
    }
    match doctor::read_pin(root, train, running) {
        Ok(pin @ RepoPin::Current(_)) => Some(pin),
        Ok(RepoPin::Other { version }) => {
            report
                .drift
                .push(Drift::PinVersion { pinned: version, running: running.to_owned() });
            None
        }
        Ok(RepoPin::Refused { reason }) => {
            report.drift.push(Drift::PinRefused { reason });
            None
        }
        Err(err) => {
            report.errors.push(format!("could not read the pin: {err}"));
            None
        }
    }
}

/// Runs [`FAB_CHECK`] and records each line it prints on drift.
fn check_fab(root: &Path, machine: &Machine, report: &mut Report) {
    let Some(output) = run_tool(&FAB_CHECK, root, machine, report) else {
        return;
    };
    match output.status.code() {
        Some(0) => {}
        Some(FAB_DRIFT) => report.drift.extend(
            output
                .stdout
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(|line| Drift::Fab(line.to_owned())),
        ),
        _ => report.errors.push(failed(&FAB_CHECK, &output)),
    }
}

/// Runs [`SKILL_CHECK`] and records each file it names on drift. arcl lists
/// them indented under a heading; when it lists none, one entry says so.
fn check_skill(root: &Path, machine: &Machine, report: &mut Report) {
    let Some(output) = run_tool(&SKILL_CHECK, root, machine, report) else {
        return;
    };
    match output.status.code() {
        Some(0) => {}
        Some(SKILL_DIFFERS) => {
            let files: Vec<Drift> = output
                .stdout
                .lines()
                .filter(|line| line.starts_with([' ', '\t']) && !line.trim().is_empty())
                .map(|line| Drift::Skill(line.trim().to_owned()))
                .collect();
            if files.is_empty() {
                report
                    .drift
                    .push(Drift::Skill("a skill file is missing or differs".to_owned()));
            }
            report.drift.extend(files);
        }
        _ => report.errors.push(failed(&SKILL_CHECK, &output)),
    }
}

/// Compares the glue with what `sltk init` writes when it adopts the
/// repository, which replaces a hook or Tasks section it did not write.
fn check_glue(root: &Path, report: &mut Report) {
    let relative = |path: &Path| path.strip_prefix(root).unwrap_or(path).display().to_string();
    match glue::plan(root, Existing::Replace) {
        Ok(changes) => report.drift.extend(
            changes
                .iter()
                .map(|change| Drift::Glue { path: change.path, how: describe(change) }),
        ),
        Err(GlueError::Content { path, reason }) => {
            report
                .drift
                .push(Drift::GlueRefused { reason: format!("{}: {reason}", relative(&path)) });
        }
        Err(GlueError::NotAFile { path, reason }) => {
            report
                .drift
                .push(Drift::GlueRefused { reason: format!("{} {reason}", relative(&path)) });
        }
        Err(GlueError::Io { path, source }) => {
            report
                .errors
                .push(format!("could not read {}: {source}", relative(&path)));
        }
    }
}

/// Runs `command` in `root` with [`run_capped`], or records why it could not.
fn run_tool(command: &[&str], root: &Path, machine: &Machine, report: &mut Report) -> Option<crate::process::Captured> {
    let line = command.join(" ");
    let Some(program) = setup::find_program(command[0], machine.path.as_deref()) else {
        report.errors.push(format!(
            "could not run {line}: no executable file named {} in an absolute PATH directory",
            command[0]
        ));
        return None;
    };
    let mut process = Command::new(program);
    process.args(&command[1..]).current_dir(root);
    if let Some(path) = &machine.path {
        process.env("PATH", path);
    }
    match run_capped(&mut process, TOOL_TIMEOUT, MAX_OUTPUT) {
        Ok(output) => Some(output),
        Err(reason) => {
            report.errors.push(format!("could not run {line}: {reason}"));
            None
        }
    }
}

/// The error for a tool check that exited with neither success nor drift,
/// with the first line it wrote to stderr.
fn failed(command: &[&str], output: &crate::process::Captured) -> String {
    let line = command.join(" ");
    match output.stderr.lines().map(str::trim).find(|l| !l.is_empty()) {
        Some(said) => format!("{line} failed with {}: {said}", output.status),
        None => format!("{line} failed with {}", output.status),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use super::{Drift, Report, check};
    use crate::doctor::{Finding, Machine};
    use crate::glue;
    use crate::pin::{self, Pin, TARGETS};
    use crate::setup::FAB_SETTINGS;
    use crate::train::Train;

    /// The release the tests run as.
    const RUNNING: &str = "1.2.3";

    /// fab's settings as fab 0.9.1 writes them, before sltk adds `[tasks]`.
    const FAB_TOML: &str = "owner = \"octo\"\napp = \"octo-bot\"\nsecret_prefix = \"OCTO\"\n\
                            trailer = \"Ada <ada@example.com>\"\ngates = \"just check\"\nversion = \"0.9.1\"\n";

    /// A repository set up as `sltk init` leaves it, and a bin directory of
    /// fake `fab` and `arcl` that report the train's versions and no drift.
    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        bin: PathBuf,
        train: Train,
    }

    impl Fixture {
        fn new() -> Fixture {
            let dir = tempfile::tempdir().unwrap();
            let (root, bin) = (dir.path().join("repo"), dir.path().join("bin"));
            std::fs::create_dir_all(root.join(".git")).unwrap();
            std::fs::create_dir_all(root.join(".github")).unwrap();
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::write(root.join(FAB_SETTINGS), FAB_TOML).unwrap();
            glue::write(&root).unwrap();
            let train = Train::embedded().unwrap();
            let sha256 = TARGETS.into_iter().map(|t| (t, "ab".repeat(32))).collect();
            Pin { version: RUNNING.to_owned(), sha256, optional: Vec::new() }
                .write(&root, &train)
                .unwrap();
            let fixture = Fixture { _dir: dir, root, bin, train };
            fixture.fab("echo 'fabrial 0.9.1 is installed with no drift.'");
            fixture.arcl("echo 'The Arc Lightning skill matches this arcl.'");
            fixture
        }

        /// The version the train pins for `tool`.
        fn version(&self, tool: &str) -> String {
            self.train
                .tools
                .iter()
                .find(|t| t.name == tool)
                .unwrap()
                .version
                .clone()
        }

        /// Writes a fake `name` that prints `version` for `--version` and
        /// runs `rest` otherwise. Builtins only: `PATH` holds nothing else.
        fn fake(&self, name: &str, version: &str, rest: &str) {
            let path = self.bin.join(name);
            let text = format!("#!/bin/sh\n[ \"$1\" = --version ] && {{ echo '{name} {version}'; exit 0; }}\n{rest}\n");
            std::fs::write(&path, text).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        /// A fake `fab` at the train's version that runs `check` for `fab check`.
        fn fab(&self, check: &str) {
            self.fake(
                "fab",
                &self.version("fab"),
                &format!("[ \"$*\" = check ] || exit 9\n{check}"),
            );
        }

        /// A fake `arcl` at the train's version that runs `check` for `arcl
        /// skill install --check`.
        fn arcl(&self, check: &str) {
            self.fake(
                "arcl",
                &self.version("arcl"),
                &format!("[ \"$*\" = 'skill install --check' ] || exit 9\n{check}"),
            );
        }

        fn check(&self) -> Report {
            let machine = Machine {
                path: Some(self.bin.clone().into_os_string()),
                bin: self.bin.clone(),
                brew_prefixes: Vec::new(),
                timeout: Duration::from_secs(5),
            };
            check(&self.train, RUNNING, &self.root, &machine)
        }

        fn lines(&self) -> Vec<String> {
            let report = self.check();
            assert_eq!(report.errors, Vec::<String>::new());
            report.drift.iter().map(|d| d.to_string()).collect()
        }
    }

    /// Every path under `dir` with its bytes, in order.
    fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(path) = stack.pop() {
            if path.is_dir() {
                stack.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
            } else {
                out.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
        out.sort();
        out
    }

    #[test]
    fn a_repository_as_sltk_init_leaves_it_has_no_drift_and_is_not_changed() {
        let fixture = Fixture::new();
        let before = tree(&fixture.root);
        assert_eq!(fixture.check(), Report::default());
        assert_eq!(tree(&fixture.root), before);
    }

    #[test]
    fn a_missing_pin_or_one_of_another_release_is_drift() {
        let fixture = Fixture::new();
        std::fs::write(fixture.root.join(pin::PATH), "version = \"9.9.9\"\n").unwrap();
        assert_eq!(
            fixture.lines(),
            [
                ".github/sltk.toml: pins sltk 9.9.9, but this is sltk 1.2.3; sltk check compares the tools sltk 1.2.3 pins"
            ]
        );
        std::fs::remove_file(fixture.root.join(pin::PATH)).unwrap();
        assert_eq!(fixture.lines(), [".github/sltk.toml: missing; sltk init writes it"]);
    }

    #[test]
    fn a_tool_at_another_version_than_the_pin_is_drift() {
        let fixture = Fixture::new();
        fixture.fake("arcl", "0.2.0", "exit 0");
        let report = fixture.check();
        assert_eq!(
            report.drift,
            [Drift::Tool(Finding::WrongVersion {
                tool: "arcl".into(),
                path: fixture.bin.join("arcl"),
                found: "0.2.0".into(),
                pinned: fixture.version("arcl"),
            })]
        );
    }

    #[test]
    fn each_line_fab_check_prints_on_drift_is_listed() {
        let fixture = Fixture::new();
        fixture
            .fab("echo 'differs: .github/workflows/review.yml'\necho 'retired: .claude/skills/storm/SKILL.md'\nexit 1");
        assert_eq!(
            fixture.lines(),
            [
                "fab check: differs: .github/workflows/review.yml",
                "fab check: retired: .claude/skills/storm/SKILL.md"
            ]
        );
    }

    #[test]
    fn each_skill_file_arcl_names_is_listed() {
        let fixture = Fixture::new();
        fixture.arcl(
            "echo 'The Arc Lightning skill in /r differs from this arcl:'\n\
             echo '  changed .claude/skills/arclightning/SKILL.md'\nexit 4",
        );
        assert_eq!(
            fixture.lines(),
            ["arcl skill install --check: changed .claude/skills/arclightning/SKILL.md"]
        );
        fixture.arcl("exit 4");
        assert_eq!(
            fixture.lines(),
            ["arcl skill install --check: a skill file is missing or differs"]
        );
    }

    #[test]
    fn a_changed_hook_is_drift_and_so_is_one_that_cannot_run() {
        let fixture = Fixture::new();
        let hook = fixture.root.join(glue::HOOK);
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(fixture.lines(), [".claude/hooks/session-start.sh: is not executable"]);
        std::fs::write(&hook, "#!/bin/sh\nversion=0.3.0\n").unwrap();
        assert_eq!(
            fixture.lines(),
            [".claude/hooks/session-start.sh: differs from the hook sltk init writes"]
        );
        std::fs::remove_file(&hook).unwrap();
        assert_eq!(fixture.lines(), [".claude/hooks/session-start.sh: is missing"]);
    }

    #[test]
    fn a_settings_file_without_the_hook_entry_is_drift() {
        let fixture = Fixture::new();
        std::fs::write(fixture.root.join(glue::SETTINGS), "{\"hooks\": {}}\n").unwrap();
        assert_eq!(
            fixture.lines(),
            [".claude/settings.json: has no SessionStart entry that runs the hook as sltk init writes it"]
        );
    }

    #[test]
    fn a_changed_tasks_section_or_a_missing_tasks_table_is_drift() {
        let fixture = Fixture::new();
        let agents = fixture.root.join(glue::AGENTS);
        let text = std::fs::read_to_string(&agents).unwrap();
        std::fs::write(&agents, text.replace("needs `arcl init`", "needs arcl")).unwrap();
        assert_eq!(
            fixture.lines(),
            ["AGENTS.md: has a Tasks section that differs from the one sltk init writes, or none"]
        );
        std::fs::write(&agents, text).unwrap();
        std::fs::write(fixture.root.join(FAB_SETTINGS), FAB_TOML).unwrap();
        assert_eq!(fixture.lines(), [".github/fabrial.toml: has no [tasks] table for arcl"]);
    }

    #[test]
    fn glue_sltk_init_would_refuse_is_drift() {
        let fixture = Fixture::new();
        let agents = fixture.root.join(glue::AGENTS);
        std::fs::write(&agents, format!("{}\n", glue::TASKS_BEGIN)).unwrap();
        let lines = fixture.lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("AGENTS.md: needs one <!-- sltk:tasks:begin --> line"),
            "{lines:?}"
        );
    }

    #[test]
    fn a_tool_check_that_cannot_run_or_fails_is_an_error() {
        let fixture = Fixture::new();
        fixture.fab("echo 'fab: not installed here' >&2\nexit 2");
        std::fs::remove_file(fixture.bin.join("arcl")).unwrap();
        let report = fixture.check();
        assert_eq!(
            report.errors,
            [
                "fab check failed with exit status: 2: fab: not installed here",
                "could not run arcl skill install --check: no executable file named arcl in an absolute PATH directory",
            ]
        );
        assert!(
            matches!(&report.drift[..], [Drift::Tool(Finding::Missing { tool, .. })] if tool == "arcl"),
            "{:?}",
            report.drift
        );
    }
}
