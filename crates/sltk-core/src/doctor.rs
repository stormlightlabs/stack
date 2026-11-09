//! `sltk doctor`: what is wrong with the tools on this machine. It finds each
//! tool's binary in the bin directory and on `PATH`, runs `<binary> --version`,
//! and reports what differs from the train. It changes nothing.
//!
//! Inside a git repository it also asks the repository's [`Host`] whether
//! the App `.github/fabrial.toml` names is installed, its secrets are
//! available and `storm/review` is required on `main`.
//!
//! Every input comes in through [`Machine`] and [`Host`], so tests build a
//! `PATH`, a bin directory, Homebrew prefixes and a host of their own.

use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::host::{Answer, Host};
use crate::pin::{self, Pin, PinError};
use crate::process::run_capped;
use crate::setup::{self, FAB_SETTINGS, Flag, LEGACY_SECRET_PREFIX};
use crate::train::{Tier, Tool, Train};

/// The endings of the two secrets fabrial's workflows read to act as the
/// App, after its secret prefix and an underscore.
pub const SECRET_SUFFIXES: [&str; 2] = ["APP_ID", "PRIVATE_KEY"];

/// The check that must be required before a pull request merges.
pub const REVIEW_CHECK: &str = "storm/review";

/// The branch that must require [`REVIEW_CHECK`].
pub const BRANCH: &str = "main";

/// How long `<binary> --version` may run before doctor stops it.
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// The most bytes doctor reads from `<binary> --version`.
const MAX_VERSION_OUTPUT: u64 = 4096;

/// Where Homebrew keeps what it installs, on Apple silicon, Intel macOS and
/// Linux. `$HOMEBREW_PREFIX` joins these in [`Machine::from_env`].
const BREW_PREFIXES: [&str; 3] = ["/opt/homebrew", "/usr/local/Cellar", "/home/linuxbrew/.linuxbrew"];

/// The largest `.github/sltk.toml` doctor reads.
const MAX_PIN_BYTES: u64 = 64 * 1024;

/// One thing doctor found wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// The bin directory has no executable file for the tool.
    Missing {
        /// The tool's name.
        tool: String,
        /// The path the binary should be at.
        path: PathBuf,
    },
    /// The binary reports a version other than the train's.
    WrongVersion {
        /// The tool's name.
        tool: String,
        /// The binary that was run.
        path: PathBuf,
        /// The version it reported.
        found: String,
        /// The version the train pins.
        pinned: String,
    },
    /// Running `<binary> --version` gave no version.
    NoVersion {
        /// The tool's name.
        tool: String,
        /// The binary that was run.
        path: PathBuf,
        /// What went wrong.
        reason: String,
    },
    /// The first match for the binary on `PATH` is not the bin directory's.
    Shadowed {
        /// The tool's name.
        tool: String,
        /// The binary `PATH` finds first.
        found: PathBuf,
        /// Whether that binary resolves to a path under a Homebrew prefix.
        homebrew: bool,
        /// The binary `sltk install` puts in the bin directory.
        pinned: PathBuf,
    },
    /// No absolute `PATH` entry is the bin directory.
    BinNotOnPath {
        /// The bin directory.
        bin: PathBuf,
    },
    /// The repository pins another `sltk` release than this one, and so
    /// another train.
    PinVersion {
        /// The release `.github/sltk.toml` pins.
        pinned: String,
        /// This binary's release.
        running: String,
    },
    /// The repository's pin is a symlink, not a regular file, or too large,
    /// so doctor did not read it.
    PinRefused {
        /// Why, as ".github/sltk.toml is a symlink".
        reason: String,
    },
    /// The host says the App is not installed.
    AppMissing {
        /// The App's slug.
        app: String,
    },
    /// The host says the App is installed, but its installation is
    /// suspended, so it cannot act.
    AppSuspended {
        /// The App's slug.
        app: String,
        /// How, as "its installation on owner was suspended at ...".
        reason: String,
    },
    /// The host says the repository's workflows cannot read a secret.
    SecretMissing {
        /// The secret's name.
        name: String,
    },
    /// The host says a branch does not require a check.
    CheckNotRequired {
        /// The check.
        check: String,
        /// The branch.
        branch: String,
    },
    /// A key of fab's settings that names the App holds a value fab
    /// refuses, so every fabrial workflow fails. Doctor then asks the host
    /// nothing about the App or its secrets.
    BadSetting {
        /// The key, `app` or `secret_prefix`.
        key: String,
        /// What is wrong with its value, as "is not a string".
        reason: String,
    },
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Finding::Missing { tool, path } => write!(f, "{tool}: {} is not installed", path.display()),
            Finding::WrongVersion { tool, path, found, pinned } => {
                write!(f, "{tool}: {} is {found}; the train pins {pinned}", path.display())
            }
            Finding::NoVersion { tool, path, reason } => {
                write!(f, "{tool}: {} --version gave no version: {reason}", path.display())
            }
            Finding::Shadowed { tool, found, homebrew, pinned } => {
                let from = if *homebrew { ", from Homebrew," } else { "" };
                write!(
                    f,
                    "{tool}: PATH runs {}{from} instead of {}",
                    found.display(),
                    pinned.display()
                )
            }
            Finding::BinNotOnPath { bin } => write!(f, "PATH does not include {}", bin.display()),
            Finding::PinVersion { pinned, running } => write!(
                f,
                "{}: pins sltk {pinned}, but this is sltk {running}; doctor checks the tools sltk {running} pins",
                pin::PATH
            ),
            Finding::PinRefused { reason } => write!(f, "{reason}; doctor read no pin"),
            Finding::AppMissing { app } => write!(f, "{app} is not installed"),
            Finding::AppSuspended { app, reason } => write!(f, "{app} cannot act: {reason}"),
            Finding::SecretMissing { name } => write!(f, "secret {name} is not available to this repository"),
            Finding::CheckNotRequired { check, branch } => write!(f, "{check} is not required on {branch}"),
            Finding::BadSetting { key, reason } => {
                write!(
                    f,
                    "{FAB_SETTINGS}: {key} {reason}; doctor did not check the App or its secrets"
                )
            }
        }
    }
}

/// A repository check doctor did not finish. Neither kind is a problem on its
/// own: the setting may well be right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// Doctor ran no repository checks.
    Skipped {
        /// Why, as "gh not found".
        reason: String,
    },
    /// Doctor did not check the App or its secrets, since fab's settings
    /// name no App.
    SkippedApp {
        /// Why, as ".github/fabrial.toml names no app".
        reason: String,
    },
    /// The host could not answer one check.
    CouldNotCheck {
        /// What was asked, as "secret FABRIAL_APP_ID".
        what: String,
        /// The host's reason.
        reason: String,
    },
}

impl fmt::Display for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Note::Skipped { reason } => write!(f, "skipped repository checks: {reason}"),
            Note::SkippedApp { reason } => write!(f, "skipped the App and secret checks: {reason}"),
            Note::CouldNotCheck { what, reason } => write!(f, "could not check {what}: {reason}"),
        }
    }
}

/// Why [`read_app`] found no App to check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoApp {
    /// fab's settings are missing, are not TOML or name no App. Doctor
    /// skips the App and its secrets with a [`Note::SkippedApp`].
    Absent(String),
    /// `app` or `secret_prefix` holds a value fab refuses: each key and what
    /// is wrong with it. Each is a [`Finding::BadSetting`].
    Invalid(Vec<(String, String)>),
}

impl NoApp {
    /// A [`Finding::BadSetting`] for each invalid key. Doctor reports them
    /// whether or not it can reach the host.
    pub fn findings(&self) -> Vec<Finding> {
        match self {
            NoApp::Absent(_) => Vec::new(),
            NoApp::Invalid(keys) => keys
                .iter()
                .map(|(key, reason)| Finding::BadSetting { key: key.clone(), reason: reason.clone() })
                .collect(),
        }
    }
}

/// The repository's pin, as [`read_pin`] finds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoPin {
    /// A pin of this `sltk` release, read whole.
    Current(Pin),
    /// A pin of another `sltk` release. Its other keys may name tools or
    /// targets this train lacks, so only its version is read.
    Other {
        /// The release `.github/sltk.toml` pins.
        version: String,
    },
    /// A pin doctor would not read, and why.
    Refused {
        /// Why, as ".github/sltk.toml is a symlink".
        reason: String,
    },
}

/// What [`read_pin_text`] found at `.github/sltk.toml`.
enum PinText {
    /// The pin's text.
    Read(String),
    /// Why doctor would not read it.
    Refused(String),
}

/// The App fabrial's workflows act as, from `.github/fabrial.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    /// The App's slug, as in `github.com/apps/<slug>`.
    pub slug: String,
    /// The prefix of the secrets holding its ID and key.
    pub secret_prefix: String,
}

impl App {
    /// The secrets the workflows read, `<prefix>_APP_ID` and
    /// `<prefix>_PRIVATE_KEY`.
    pub fn secrets(&self) -> [String; 2] {
        SECRET_SUFFIXES.map(|suffix| format!("{}_{suffix}", self.secret_prefix))
    }
}

/// What doctor reads from the machine.
#[derive(Debug, Clone)]
pub struct Machine {
    /// The `PATH` to search. Relative entries are skipped.
    pub path: Option<OsString>,
    /// The directory `sltk install` puts tools in.
    pub bin: PathBuf,
    /// Directories under which a binary counts as Homebrew's.
    pub brew_prefixes: Vec<PathBuf>,
    /// How long `<binary> --version` may run.
    pub timeout: Duration,
}

impl Machine {
    /// The machine as this process sees it: `PATH`, the bin directory from
    /// `CARGO_HOME` and `HOME`, and the Homebrew prefixes with
    /// `$HOMEBREW_PREFIX` added when it is set.
    pub fn from_env() -> Result<Machine, crate::install::InstallError> {
        let var = std::env::var_os;
        let bin = crate::install::bin_dir(var("CARGO_HOME"), var("HOME"))?;
        let mut brew_prefixes: Vec<PathBuf> = BREW_PREFIXES.iter().map(PathBuf::from).collect();
        if let Some(prefix) = var("HOMEBREW_PREFIX").filter(|p| Path::new(p).is_absolute()) {
            brew_prefixes.push(prefix.into());
        }
        Ok(Machine { path: var("PATH"), bin, brew_prefixes, timeout: VERSION_TIMEOUT })
    }
}

/// What doctor checked and what it found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The tools checked, in train order.
    pub checked: Vec<String>,
    /// What is wrong, in the order found.
    pub findings: Vec<Finding>,
    /// The repository checked, as [`Host::repository`] names it.
    pub repository: Option<String>,
    /// Repository checks doctor skipped or could not finish.
    pub notes: Vec<Note>,
}

/// Checks the tools `train` lists on `machine`.
///
/// The essentials are always checked. With a pin of this release, so are the
/// optional tools it lists. A pin of another release than `running`, or one
/// doctor refused to read, is a finding, and doctor then treats the
/// repository as unpinned: an optional tool is checked only when the bin
/// directory has it.
pub fn check(train: &Train, pin: Option<&RepoPin>, running: &str, machine: &Machine) -> Report {
    let mut report = Report { checked: Vec::new(), findings: Vec::new(), repository: None, notes: Vec::new() };
    let pin = match pin {
        Some(RepoPin::Current(pin)) => Some(pin),
        Some(RepoPin::Other { version }) => {
            report
                .findings
                .push(Finding::PinVersion { pinned: version.clone(), running: running.to_owned() });
            None
        }
        Some(RepoPin::Refused { reason }) => {
            report.findings.push(Finding::PinRefused { reason: reason.clone() });
            None
        }
        None => None,
    };
    let bin_on_path = path_dirs(machine.path.as_ref()).any(|dir| same_file(&dir, &machine.bin));
    if !bin_on_path {
        report.findings.push(Finding::BinNotOnPath { bin: machine.bin.clone() });
    }
    for tool in train.tools.iter().filter(|tool| wanted(tool, pin, &machine.bin)) {
        report.checked.push(tool.name.clone());
        check_tool(tool, machine, &mut report.findings);
    }
    report
}

/// Asks `host` whether `app` is installed, each of its secrets is available
/// and [`BRANCH`] requires [`REVIEW_CHECK`]. A no or a suspension is a
/// finding; an answer the host could not give is a [`Note::CouldNotCheck`].
/// When `app` is the reason there is none, from [`read_app`], doctor skips
/// the App and its secrets: with a [`Note::SkippedApp`] when the settings
/// name none, and with a [`Finding::BadSetting`] per key fab would refuse.
pub fn check_repository(host: &dyn Host, app: Result<&App, &NoApp>, report: &mut Report) {
    report.repository = Some(host.repository());
    let mut skipped = None;
    let mut invalid = Vec::new();
    let mut record = |answer: Answer, finding: Finding, what: String| match answer {
        Answer::Yes => {}
        Answer::No => report.findings.push(finding),
        Answer::Suspended(reason) => report.findings.push(match finding {
            Finding::AppMissing { app } => Finding::AppSuspended { app, reason },
            other => other,
        }),
        Answer::Unknown(reason) => report.notes.push(Note::CouldNotCheck { what, reason }),
    };
    match app {
        Ok(app) => {
            let slug = &app.slug;
            record(
                host.app_installed(slug),
                Finding::AppMissing { app: slug.clone() },
                format!("whether {slug} is installed"),
            );
            for name in app.secrets() {
                record(
                    host.secret_available(&name),
                    Finding::SecretMissing { name: name.clone() },
                    format!("secret {name}"),
                );
            }
        }
        Err(NoApp::Absent(reason)) => skipped = Some(Note::SkippedApp { reason: reason.clone() }),
        Err(no_app) => invalid = no_app.findings(),
    }
    record(
        host.check_required(BRANCH, REVIEW_CHECK),
        Finding::CheckNotRequired { check: REVIEW_CHECK.to_owned(), branch: BRANCH.to_owned() },
        format!("whether {REVIEW_CHECK} is required on {BRANCH}"),
    );
    report.findings.extend(invalid);
    report.notes.extend(skipped);
}

/// The App `.github/fabrial.toml` under `root` names, with its secret
/// prefix, or why there is none. No file, no `app` key, or a file that is
/// not TOML is [`NoApp::Absent`]. An `app` or `secret_prefix` that is not a
/// string, or that [`Flag::check`] refuses as `sltk init` would, is
/// [`NoApp::Invalid`], since fab refuses it too. Settings with no
/// `secret_prefix` read the secrets fab 0.9.0 and earlier named,
/// [`LEGACY_SECRET_PREFIX`].
pub fn read_app(root: &Path) -> Result<App, NoApp> {
    let text = setup::read_small(&root.join(FAB_SETTINGS))
        .map_err(|reason| NoApp::Absent(format!("{FAB_SETTINGS} {reason}")))?
        .ok_or_else(|| NoApp::Absent(format!("there is no {FAB_SETTINGS}")))?;
    let settings: toml::Table = toml::from_str(&text)
        .map_err(|err| NoApp::Absent(format!("{FAB_SETTINGS} could not be read as TOML: {}", err.message())))?;
    let mut invalid = Vec::new();
    let mut value = |key: &str, flag: Flag| match settings.get(key) {
        None => None,
        Some(toml::Value::String(value)) => match flag.check(value) {
            Ok(()) => Some(value.clone()),
            Err(reason) => {
                invalid.push((key.to_owned(), reason));
                None
            }
        },
        Some(_) => {
            invalid.push((key.to_owned(), "is not a string".to_owned()));
            None
        }
    };
    let slug = value("app", Flag::App);
    let secret_prefix = value("secret_prefix", Flag::SecretPrefix);
    if !invalid.is_empty() {
        return Err(NoApp::Invalid(invalid));
    }
    let slug = slug.ok_or_else(|| NoApp::Absent(format!("{FAB_SETTINGS} names no app")))?;
    let secret_prefix = secret_prefix.unwrap_or_else(|| LEGACY_SECRET_PREFIX.to_owned());
    Ok(App { slug, secret_prefix })
}

/// The root of the git repository `start` is in: the nearest directory at or
/// above it holding a `.git` entry, a directory in a clone and a file in a
/// worktree. Outside a repository there is none.
pub fn find_repo_root(start: &Path) -> Option<&Path> {
    start.ancestors().find(|dir| exists(&dir.join(".git")))
}

/// The nearest directory at or above `start` that has a `.github/sltk.toml`,
/// searching no higher than [`find_repo_root`], so a repository never takes
/// the pin of one it sits inside. Outside a repository there is no pin. The
/// pin is found without following a symlink at its own path; [`read_pin`]
/// refuses one.
pub fn find_pin_root(start: &Path) -> Option<PathBuf> {
    let repo = find_repo_root(start)?;
    start
        .ancestors()
        .take(start.ancestors().position(|dir| dir == repo)? + 1)
        .find(|dir| exists(&dir.join(pin::PATH)))
        .map(Path::to_owned)
}

/// Reads `.github/sltk.toml` under `root`. When `.github` or the pin is a
/// symlink, the pin is not a regular file, or it is larger than 64 KiB, the
/// pin is [`RepoPin::Refused`]. When its version line names a release other
/// than `running`, only that version is read, since the rest follows another
/// train. Otherwise the whole pin is read against `train`, and a pin without a
/// version line fails as [`Pin::parse`] says.
pub fn read_pin(root: &Path, train: &Train, running: &str) -> Result<RepoPin, PinError> {
    let text = match read_pin_text(root)? {
        PinText::Read(text) => text,
        PinText::Refused(reason) => return Ok(RepoPin::Refused { reason }),
    };
    match pin::read_version(&text) {
        Some(version) if version != running => Ok(RepoPin::Other { version: version.to_owned() }),
        _ => Pin::parse(&text, train).map(RepoPin::Current),
    }
}

/// The version in `<binary> --version` output, in the form fab and arcl print
/// it: a first line of two words, `binary` then `x.y.z`, with optional build
/// metadata after a `+`. `fab 0.8.0+c5fa630` gives `0.8.0`. A prerelease such
/// as `0.8.0-rc.1`, a fourth part, another first word, or more words is not a
/// version.
pub fn parse_version<'a>(text: &'a str, binary: &str) -> Option<&'a str> {
    let mut words = text.lines().next()?.split_whitespace();
    if words.next()? != binary {
        return None;
    }
    let word = words.next()?;
    if words.next().is_some() {
        return None;
    }
    let (version, build) = match word.split_once('+') {
        Some((version, build)) => (version, Some(build)),
        None => (word, None),
    };
    let numeric = |part: &str| {
        !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) && (part == "0" || !part.starts_with('0'))
    };
    let identifier = |id: &str| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    let parts: Vec<&str> = version.split('.').collect();
    let core = parts.len() == 3 && parts.iter().all(|part| numeric(part));
    let build_ok = build.is_none_or(|build| build.split('.').all(identifier));
    (core && build_ok).then_some(version)
}

/// Whether doctor checks `tool`: always for an essential, else when the pin
/// lists it, or, without a pin, when `bin` has it.
fn wanted(tool: &Tool, pin: Option<&Pin>, bin: &Path) -> bool {
    match (tool.tier, pin) {
        (Tier::Essential, _) => true,
        (Tier::Optional, Some(pin)) => pin.optional.contains(&tool.name),
        (Tier::Optional, None) => is_executable(&bin.join(&tool.binary)),
    }
}

/// Adds what is wrong with one tool to `findings`.
fn check_tool(tool: &Tool, machine: &Machine, findings: &mut Vec<Finding>) {
    let pinned = machine.bin.join(&tool.binary);
    if !is_executable(&pinned) {
        findings.push(Finding::Missing { tool: tool.name.clone(), path: pinned.clone() });
    } else {
        match run_version(&pinned, machine.timeout) {
            Ok(output) if output.trim().is_empty() => findings.push(Finding::NoVersion {
                tool: tool.name.clone(),
                path: pinned.clone(),
                reason: "it exited without printing a version".to_owned(),
            }),
            Ok(output) => match parse_version(&output, &tool.binary) {
                Some(found) if found == tool.version => {}
                Some(found) => findings.push(Finding::WrongVersion {
                    tool: tool.name.clone(),
                    path: pinned.clone(),
                    found: found.to_owned(),
                    pinned: tool.version.clone(),
                }),
                None => findings.push(Finding::NoVersion {
                    tool: tool.name.clone(),
                    path: pinned.clone(),
                    reason: "unrecognised version output".to_owned(),
                }),
            },
            Err(reason) => findings.push(Finding::NoVersion { tool: tool.name.clone(), path: pinned.clone(), reason }),
        }
    }
    if let Some(found) = find_on_path(machine.path.as_ref(), &tool.binary)
        && !same_file(&found, &pinned)
    {
        let homebrew = is_homebrew(&found, &machine.brew_prefixes);
        findings.push(Finding::Shadowed { tool: tool.name.clone(), found, homebrew, pinned });
    }
}

/// The absolute entries of `path`, in order. A relative or empty entry would
/// resolve against the current directory, so it is skipped.
fn path_dirs(path: Option<&OsString>) -> impl Iterator<Item = PathBuf> + '_ {
    path.into_iter()
        .flat_map(std::env::split_paths)
        .filter(|dir| dir.is_absolute())
}

/// The first executable file named `binary` in an absolute entry of `path`.
fn find_on_path(path: Option<&OsString>, binary: &str) -> Option<PathBuf> {
    path_dirs(path)
        .map(|dir| dir.join(binary))
        .find(|candidate| is_executable(candidate))
}

/// Whether `path` is a regular file, after symlinks, that someone may run.
fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// Whether `a` and `b` name the same file once symlinks are resolved, or are
/// the same path when either cannot be resolved.
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Whether `path`, as given or with symlinks resolved, is under one of
/// `prefixes`.
fn is_homebrew(path: &Path, prefixes: &[PathBuf]) -> bool {
    let resolved = path.canonicalize().ok();
    prefixes.iter().any(|prefix| {
        let resolved_prefix = prefix.canonicalize().ok();
        [Some(path), resolved.as_deref()]
            .into_iter()
            .flatten()
            .any(|p| p.starts_with(prefix) || resolved_prefix.as_deref().is_some_and(|rp| p.starts_with(rp)))
    })
}

/// Whether anything, a symlink included, is at `path`.
fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// The text of `.github/sltk.toml` under `root`, or why doctor refuses it:
/// `.github` or the pin is a symlink, the pin is not a regular file, or it is
/// larger than [`MAX_PIN_BYTES`]. An error reading it is a [`PinError::Io`].
fn read_pin_text(root: &Path) -> Result<PinText, PinError> {
    let io = |path: PathBuf| move |source| PinError::Io { path, source };
    let github = root.join(".github");
    let path = root.join(pin::PATH);
    for (entry, dir) in [(&github, true), (&path, false)] {
        let meta = std::fs::symlink_metadata(entry).map_err(io(entry.clone()))?;
        let name = entry.strip_prefix(root).unwrap_or(entry).display();
        if meta.file_type().is_symlink() {
            return Ok(PinText::Refused(format!("{name} is a symlink")));
        }
        if dir && !meta.is_dir() {
            return Ok(PinText::Refused(format!("{name} is not a directory")));
        }
        if !dir && !meta.is_file() {
            return Ok(PinText::Refused(format!("{name} is not a regular file")));
        }
    }
    let file = open_no_follow(&path).map_err(io(path.clone()))?;
    let meta = file.metadata().map_err(io(path.clone()))?;
    if !meta.is_file() {
        return Ok(PinText::Refused(format!("{} is not a regular file", pin::PATH)));
    }
    let mut text = String::new();
    file.take(MAX_PIN_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(io(path.clone()))?;
    if text.len() as u64 > MAX_PIN_BYTES {
        return Ok(PinText::Refused(format!("{} is larger than 64 KiB", pin::PATH)));
    }
    Ok(PinText::Read(text))
}

/// Opens `path` to read, failing if it is a symlink, and without blocking if
/// it has become a FIFO since it was checked.
fn open_no_follow(path: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use rustix::fs::OFlags;
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags((OFlags::NOFOLLOW | OFlags::NONBLOCK).bits() as i32);
    }
    options.open(path)
}

/// Runs `<binary> --version` and returns what it wrote to stdout, up to
/// [`MAX_VERSION_OUTPUT`] bytes, as [`run_capped`] runs it.
fn run_version(binary: &Path, timeout: Duration) -> Result<String, String> {
    let mut command = Command::new(binary);
    command.arg("--version");
    run_capped(&mut command, timeout, MAX_VERSION_OUTPUT).map(|output| output.stdout)
}

#[cfg(all(test, unix))]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{
        App, Finding, Machine, NoApp, Note, RepoPin, Report, check, check_repository, find_pin_root, parse_version,
        read_app, read_pin,
    };
    use crate::host::{Answer, Host};
    use crate::pin::{Pin, PinError};
    use crate::train::Train;

    /// fab and arcl essential, mire optional.
    const TRAIN: &str = r#"
[[tool]]
name = "fab"
tier = "essential"
binary = "fab"
version = "0.8.0"
url = "https://example.test/v{version}/{archive}"

[tool.platforms.x86_64-unknown-linux-gnu]
archive = "fab-x86_64-unknown-linux-gnu.tar.xz"
sha256 = "81cf21ffd2140482a4ea315e60ca34ead891c18e9aa65e7f05daf7fda2404498"

[[tool]]
name = "arcl"
tier = "essential"
binary = "arcl"
version = "0.3.0"
url = "https://example.test/v{version}/{archive}"

[tool.platforms.x86_64-unknown-linux-gnu]
archive = "arcl-x86_64-unknown-linux-musl.tar.xz"
sha256 = "80f6bc7e4ce41f7037c91330c814d94e277aed381d192960d00a130a859675e3"

[[tool]]
name = "mire"
tier = "optional"
binary = "mire"
version = "0.1.0"
url = "https://example.test/v{version}/{archive}"

[tool.platforms.x86_64-unknown-linux-gnu]
archive = "mire-x86_64-unknown-linux-gnu.tar.xz"
sha256 = "81cf21ffd2140482a4ea315e60ca34ead891c18e9aa65e7f05daf7fda2404498"
"#;

    /// A temporary machine: a bin directory, a Homebrew prefix and a spare
    /// directory, none on `PATH` until a test puts them there.
    struct Fixture {
        root: tempfile::TempDir,
        train: Train,
    }

    impl Fixture {
        fn new() -> Fixture {
            let root = tempfile::tempdir().unwrap();
            for dir in ["cargo/bin", "brew/bin", "other"] {
                std::fs::create_dir_all(root.path().join(dir)).unwrap();
            }
            Fixture { root, train: Train::parse(TRAIN).unwrap() }
        }

        fn dir(&self, name: &str) -> PathBuf {
            self.root.path().join(name)
        }

        fn bin(&self) -> PathBuf {
            self.dir("cargo/bin")
        }

        /// Writes a script `name` in `dir` that prints `output`.
        fn fake(&self, dir: &str, name: &str, output: &str) -> PathBuf {
            let path = self.dir(dir).join(name);
            std::fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{output}'\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }

        /// fab 0.8.0 and arcl 0.3.0 in the bin directory, as `fab --version`
        /// and `arcl --version` print them.
        fn good(&self) {
            self.fake("cargo/bin", "fab", "fab 0.8.0+c5fa630");
            self.fake("cargo/bin", "arcl", "arcl 0.3.0");
        }

        fn machine(&self, path: &[&str]) -> Machine {
            let entries: Vec<PathBuf> = path
                .iter()
                .map(|p| if p.starts_with('/') { PathBuf::from(p) } else { self.dir(p) })
                .collect();
            Machine {
                path: Some(std::env::join_paths(entries).unwrap()),
                bin: self.bin(),
                brew_prefixes: vec![self.dir("brew")],
                timeout: Duration::from_secs(5),
            }
        }

        fn check(&self, pin: Option<&RepoPin>, path: &[&str]) -> Report {
            check(&self.train, pin, "0.2.0", &self.machine(path))
        }
    }

    /// A host that gives set answers and records what it was asked.
    struct FakeHost {
        app: Answer,
        app_id: Answer,
        private_key: Answer,
        review: Answer,
        asked: std::cell::RefCell<Vec<String>>,
    }

    impl FakeHost {
        /// A host that answers yes to everything.
        fn yes() -> FakeHost {
            FakeHost {
                app: Answer::Yes,
                app_id: Answer::Yes,
                private_key: Answer::Yes,
                review: Answer::Yes,
                asked: Default::default(),
            }
        }

        /// What doctor finds and notes with this host, for the App
        /// `bot` with secrets under `BOT`.
        fn report(&self) -> Report {
            let app = App { slug: "bot".into(), secret_prefix: "BOT".into() };
            self.report_for(Ok(&app))
        }

        /// What doctor finds and notes with this host for `app`.
        fn report_for(&self, app: Result<&App, &NoApp>) -> Report {
            let mut report = Report { checked: Vec::new(), findings: Vec::new(), repository: None, notes: Vec::new() };
            check_repository(self, app, &mut report);
            report
        }
    }

    impl Host for FakeHost {
        fn repository(&self) -> String {
            "owner/repo on Fake".to_owned()
        }

        fn app_installed(&self, app: &str) -> Answer {
            self.asked.borrow_mut().push(format!("app {app}"));
            self.app.clone()
        }

        fn secret_available(&self, name: &str) -> Answer {
            self.asked.borrow_mut().push(format!("secret {name}"));
            match name {
                "BOT_APP_ID" => self.app_id.clone(),
                "BOT_PRIVATE_KEY" => self.private_key.clone(),
                _ => Answer::Unknown(format!("unexpected secret {name}")),
            }
        }

        fn check_required(&self, branch: &str, check: &str) -> Answer {
            self.asked.borrow_mut().push(format!("check {check} on {branch}"));
            self.review.clone()
        }
    }

    /// A pin of the release the fixture runs as, listing `optional`.
    fn pin(optional: &[&str]) -> RepoPin {
        RepoPin::Current(Pin {
            version: "0.2.0".to_owned(),
            sha256: Default::default(),
            optional: optional.iter().map(|s| s.to_string()).collect(),
        })
    }

    /// Writes `text` as `.github/sltk.toml` under `root`.
    fn write_pin(root: &std::path::Path, text: &str) {
        std::fs::create_dir_all(root.join(".github")).unwrap();
        std::fs::write(root.join(".github/sltk.toml"), text).unwrap();
    }

    #[test]
    fn a_machine_on_the_train_has_no_findings() {
        let fixture = Fixture::new();
        fixture.good();
        let report = fixture.check(None, &["cargo/bin", "other"]);
        assert_eq!(report.checked, ["fab", "arcl"]);
        assert_eq!(report.findings, []);
    }

    #[test]
    fn a_missing_tool_is_named() {
        let fixture = Fixture::new();
        fixture.fake("cargo/bin", "fab", "fab 0.8.0");
        let report = fixture.check(None, &["cargo/bin"]);
        let path = fixture.bin().join("arcl");
        assert_eq!(
            report.findings,
            [Finding::Missing { tool: "arcl".into(), path: path.clone() }]
        );
        assert_eq!(
            report.findings[0].to_string(),
            format!("arcl: {} is not installed", path.display())
        );
    }

    #[test]
    fn a_file_without_an_execute_bit_is_missing() {
        let fixture = Fixture::new();
        fixture.good();
        let arcl = fixture.bin().join("arcl");
        std::fs::set_permissions(&arcl, std::fs::Permissions::from_mode(0o644)).unwrap();
        let report = fixture.check(None, &["cargo/bin"]);
        assert_eq!(report.findings, [Finding::Missing { tool: "arcl".into(), path: arcl }]);
    }

    #[test]
    fn a_version_other_than_the_train_is_reported() {
        let fixture = Fixture::new();
        fixture.good();
        let fab = fixture.fake("cargo/bin", "fab", "fab 0.7.1+0000000");
        let report = fixture.check(None, &["cargo/bin"]);
        let finding = Finding::WrongVersion {
            tool: "fab".into(),
            path: fab.clone(),
            found: "0.7.1".into(),
            pinned: "0.8.0".into(),
        };
        assert_eq!(report.findings, [finding]);
        assert_eq!(
            report.findings[0].to_string(),
            format!("fab: {} is 0.7.1; the train pins 0.8.0", fab.display())
        );
    }

    #[test]
    fn output_with_no_version_is_reported() {
        let fixture = Fixture::new();
        fixture.good();
        let arcl = fixture.fake("cargo/bin", "arcl", "arcl, built today");
        let report = fixture.check(None, &["cargo/bin"]);
        assert_eq!(
            report.findings,
            [Finding::NoVersion { tool: "arcl".into(), path: arcl, reason: "unrecognised version output".into() }]
        );
    }

    #[test]
    fn a_binary_that_prints_nothing_is_reported() {
        let fixture = Fixture::new();
        fixture.good();
        let arcl = fixture.bin().join("arcl");
        std::fs::write(&arcl, "#!/bin/sh\nexit 0\n").unwrap();
        let report = fixture.check(None, &["cargo/bin"]);
        assert_eq!(
            report.findings,
            [Finding::NoVersion {
                tool: "arcl".into(),
                path: arcl,
                reason: "it exited without printing a version".into()
            }]
        );
    }

    #[test]
    fn a_background_process_holding_stdout_does_not_hold_doctor() {
        let fixture = Fixture::new();
        fixture.good();
        // fab prints its version, starts a process that keeps stdout open and
        // writes its pid, waits for the pid, then exits.
        let pid_file = fixture.dir("other/pid");
        let script = format!(
            "#!/bin/sh\nprintf 'fab 0.8.0\\n'\nsh -c 'echo $$ > {pid}; exec sleep 3' &\n\
             while [ ! -s {pid} ]; do sleep 0.01; done\nexit 0\n",
            pid = pid_file.display()
        );
        std::fs::write(fixture.bin().join("fab"), script).unwrap();
        let started = std::time::Instant::now();
        let report = fixture.check(None, &["cargo/bin"]);
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        assert_eq!(report.findings, []);
        let pid = std::fs::read_to_string(&pid_file).unwrap();
        let pid = pid.trim();
        // Killed, it is gone, or a zombie until init reaps it.
        let gone = (0..50).any(|_| {
            let ps = std::process::Command::new("ps")
                .args(["-o", "stat=", "-p", pid])
                .output()
                .unwrap();
            let stat = String::from_utf8_lossy(&ps.stdout);
            let stat = stat.trim();
            stat.is_empty() || stat.starts_with('Z') || {
                std::thread::sleep(Duration::from_millis(20));
                false
            }
        });
        assert!(gone, "process {pid} still runs");
    }

    #[test]
    fn a_binary_that_hangs_is_stopped() {
        let fixture = Fixture::new();
        fixture.good();
        let arcl = fixture.bin().join("arcl");
        std::fs::write(&arcl, "#!/bin/sh\nexec sleep 30\n").unwrap();
        let mut machine = fixture.machine(&["cargo/bin"]);
        machine.timeout = Duration::from_millis(200);
        let started = std::time::Instant::now();
        let report = check(&fixture.train, None, "0.2.0", &machine);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(
            report.findings,
            [Finding::NoVersion { tool: "arcl".into(), path: arcl, reason: "it did not exit within 0.2 s".into() }]
        );
    }

    #[test]
    fn a_homebrew_binary_ahead_on_path_is_reported() {
        let fixture = Fixture::new();
        fixture.good();
        // The link sits outside the Homebrew prefix and points into its
        // Cellar, so only the resolved path shows it is Homebrew's.
        let cellar = fixture.dir("brew/Cellar/fab/0.7.0/bin");
        std::fs::create_dir_all(&cellar).unwrap();
        std::fs::create_dir_all(fixture.dir("other/bin")).unwrap();
        let real = fixture.fake("brew/Cellar/fab/0.7.0/bin", "fab", "fab 0.7.0");
        let link = fixture.dir("other/bin/fab");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let report = fixture.check(None, &["other/bin", "cargo/bin"]);
        let finding = Finding::Shadowed {
            tool: "fab".into(),
            found: link.clone(),
            homebrew: true,
            pinned: fixture.bin().join("fab"),
        };
        assert_eq!(report.findings, [finding]);
        assert_eq!(
            report.findings[0].to_string(),
            format!(
                "fab: PATH runs {}, from Homebrew, instead of {}",
                link.display(),
                fixture.bin().join("fab").display()
            )
        );
    }

    #[test]
    fn a_homebrew_binary_behind_the_bin_directory_is_fine() {
        let fixture = Fixture::new();
        fixture.good();
        fixture.fake("brew/bin", "fab", "fab 0.7.0");
        assert_eq!(fixture.check(None, &["cargo/bin", "brew/bin"]).findings, []);
    }

    #[test]
    fn another_binary_ahead_on_path_is_not_called_homebrew() {
        let fixture = Fixture::new();
        fixture.good();
        let other = fixture.fake("other", "arcl", "arcl 0.3.0");
        let report = fixture.check(None, &["other", "cargo/bin"]);
        let finding = Finding::Shadowed {
            tool: "arcl".into(),
            found: other.clone(),
            homebrew: false,
            pinned: fixture.bin().join("arcl"),
        };
        assert_eq!(report.findings, [finding]);
        assert!(!report.findings[0].to_string().contains("Homebrew"));
    }

    #[test]
    fn a_link_to_the_bin_directory_is_the_same_directory() {
        let fixture = Fixture::new();
        fixture.good();
        std::os::unix::fs::symlink(fixture.bin(), fixture.dir("linked")).unwrap();
        assert_eq!(fixture.check(None, &["linked"]).findings, []);
    }

    #[test]
    fn a_bin_directory_off_path_is_reported() {
        let fixture = Fixture::new();
        fixture.good();
        let report = fixture.check(None, &["other"]);
        assert_eq!(report.findings, [Finding::BinNotOnPath { bin: fixture.bin() }]);
        assert_eq!(
            report.findings[0].to_string(),
            format!("PATH does not include {}", fixture.bin().display())
        );
    }

    #[test]
    fn relative_path_entries_are_never_searched() {
        let fixture = Fixture::new();
        fixture.good();
        let mut machine = fixture.machine(&["cargo/bin"]);
        // A relative entry naming the bin directory still does not count, and
        // a fab in it is not found.
        let relative = fixture.bin().strip_prefix("/").unwrap().to_owned();
        machine.path = Some(std::env::join_paths([PathBuf::from("."), relative, fixture.bin()]).unwrap());
        assert_eq!(check(&fixture.train, None, "0.2.0", &machine).findings, []);
        machine.path = Some(OsString::from("cargo/bin"));
        assert_eq!(
            check(&fixture.train, None, "0.2.0", &machine).findings,
            [Finding::BinNotOnPath { bin: fixture.bin() }]
        );
    }

    #[test]
    fn a_pin_of_another_release_is_reported() {
        let fixture = Fixture::new();
        fixture.good();
        let other = RepoPin::Other { version: "0.1.0".into() };
        let report = fixture.check(Some(&other), &["cargo/bin"]);
        let finding = Finding::PinVersion { pinned: "0.1.0".into(), running: "0.2.0".into() };
        assert_eq!(report.findings, [finding]);
        assert_eq!(
            report.findings[0].to_string(),
            ".github/sltk.toml: pins sltk 0.1.0, but this is sltk 0.2.0; doctor checks the tools sltk 0.2.0 pins"
        );
        assert_eq!(fixture.check(Some(&pin(&[])), &["cargo/bin"]).findings, []);
    }

    #[test]
    fn a_pin_of_another_release_is_read_by_its_version_line_alone() {
        let root = tempfile::tempdir().unwrap();
        // A target this train has no hash key for: the full parse rejects it.
        write_pin(
            root.path(),
            "version = \"9.9.9\"\nsha256-aarch64-unknown-linux-gnu = \"00\"\noptional = [\"zed\"]\n",
        );
        let train = Train::parse(TRAIN).unwrap();
        assert_eq!(
            read_pin(root.path(), &train, "0.2.0").unwrap(),
            RepoPin::Other { version: "9.9.9".into() }
        );
        // The same file pinning this release is read whole, and fails.
        let err = read_pin(root.path(), &train, "9.9.9").unwrap_err();
        assert!(matches!(err, PinError::MissingHashLine { .. }), "{err}");
    }

    #[test]
    fn a_pin_without_a_version_line_fails_to_read() {
        let root = tempfile::tempdir().unwrap();
        write_pin(root.path(), "version = \"9.9\"\n");
        let train = Train::parse(TRAIN).unwrap();
        let err = read_pin(root.path(), &train, "0.2.0").unwrap_err();
        assert!(matches!(err, PinError::MissingVersionLine), "{err}");
    }

    #[test]
    fn a_pinned_optional_tool_is_checked() {
        let fixture = Fixture::new();
        fixture.good();
        let report = fixture.check(Some(&pin(&["mire"])), &["cargo/bin"]);
        assert_eq!(report.checked, ["fab", "arcl", "mire"]);
        assert_eq!(
            report.findings,
            [Finding::Missing { tool: "mire".into(), path: fixture.bin().join("mire") }]
        );
    }

    #[test]
    fn without_a_pin_only_an_installed_optional_tool_is_checked() {
        let fixture = Fixture::new();
        fixture.good();
        fixture.fake("brew/bin", "mire", "mire 0.0.9");
        assert_eq!(fixture.check(None, &["brew/bin", "cargo/bin"]).checked, ["fab", "arcl"]);
        fixture.fake("cargo/bin", "mire", "mire 0.1.0");
        let report = fixture.check(None, &["brew/bin", "cargo/bin"]);
        assert_eq!(report.checked, ["fab", "arcl", "mire"]);
        assert!(matches!(&report.findings[..], [Finding::Shadowed { tool, homebrew: true, .. }] if tool == "mire"));
    }

    #[test]
    fn the_version_is_the_word_after_the_binary_name() {
        assert_eq!(parse_version("fab 0.8.0+c5fa630", "fab"), Some("0.8.0"));
        assert_eq!(parse_version("arcl 0.3.0\n", "arcl"), Some("0.3.0"));
        assert_eq!(parse_version("fab 10.20.30+build.1-x\nmore\n", "fab"), Some("10.20.30"));
        for text in [
            "fab 0.8.0-rc.1",
            "fab 0.8.0.1",
            "fab 0.8",
            "fab 0.8.0+",
            "fab 0.8.0+a..b",
            "fab 0.8.0+c5f_a63",
            "fab v0.8.0",
            "fab 0.08.0",
            "fab 0.8.0 (c5fa630)",
            "arcl 0.8.0",
            "built with rustc 1.97.0\nfab 0.8.0",
            "fab\n0.8.0",
            "",
        ] {
            assert_eq!(parse_version(text, "fab"), None, "{text:?}");
        }
    }

    #[test]
    fn a_pin_above_any_repository_is_ignored() {
        let root = tempfile::tempdir().unwrap();
        write_pin(root.path(), "");
        let deep = root.path().join("a/b");
        std::fs::create_dir_all(&deep).unwrap();
        assert_eq!(find_pin_root(&deep), None);
        assert_eq!(find_pin_root(root.path()), None);
        // Nor does a repository below it take it.
        std::fs::create_dir_all(deep.join(".git")).unwrap();
        assert_eq!(find_pin_root(&deep), None);
    }

    #[test]
    fn a_symlinked_pin_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join(".github")).unwrap();
        let elsewhere = root.path().join("pin.toml");
        std::fs::write(&elsewhere, "version = \"9.9.9\"\n").unwrap();
        std::os::unix::fs::symlink(&elsewhere, repo.join(".github/sltk.toml")).unwrap();
        assert_eq!(find_pin_root(&repo).as_deref(), Some(repo.as_path()));
        let train = Train::parse(TRAIN).unwrap();
        let refused = read_pin(&repo, &train, "0.2.0").unwrap();
        assert_eq!(
            refused,
            RepoPin::Refused { reason: ".github/sltk.toml is a symlink".into() }
        );
        let fixture = Fixture::new();
        fixture.good();
        let report = fixture.check(Some(&refused), &["cargo/bin"]);
        assert_eq!(
            report.findings.iter().map(|f| f.to_string()).collect::<Vec<_>>(),
            [".github/sltk.toml is a symlink; doctor read no pin"]
        );
    }

    #[test]
    fn a_pin_that_is_not_a_small_regular_file_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let train = Train::parse(TRAIN).unwrap();
        let read = |text: &str| {
            write_pin(root.path(), text);
            read_pin(root.path(), &train, "0.2.0").unwrap()
        };
        let big = format!("version = \"9.9.9\"\n{}", "#".repeat(64 * 1024));
        assert_eq!(
            read(&big),
            RepoPin::Refused { reason: ".github/sltk.toml is larger than 64 KiB".into() }
        );
        std::fs::remove_file(root.path().join(".github/sltk.toml")).unwrap();
        std::fs::create_dir(root.path().join(".github/sltk.toml")).unwrap();
        assert_eq!(
            read_pin(root.path(), &train, "0.2.0").unwrap(),
            RepoPin::Refused { reason: ".github/sltk.toml is not a regular file".into() }
        );
        // A symlinked .github is refused too.
        let other = tempfile::tempdir().unwrap();
        write_pin(other.path(), "version = \"9.9.9\"\n");
        let repo = root.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        std::os::unix::fs::symlink(other.path().join(".github"), repo.join(".github")).unwrap();
        assert_eq!(
            read_pin(&repo, &train, "0.2.0").unwrap(),
            RepoPin::Refused { reason: ".github is a symlink".into() }
        );
    }

    #[test]
    fn the_pin_root_is_the_nearest_with_a_pin_inside_the_repository() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        let deep = repo.join("a/b");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        assert_eq!(find_pin_root(&deep), None);
        std::fs::create_dir_all(repo.join(".github")).unwrap();
        std::fs::write(repo.join(".github/sltk.toml"), "").unwrap();
        assert_eq!(find_pin_root(&deep).as_deref(), Some(repo.as_path()));
        // A repository inside another does not take the outer pin.
        let inner = repo.join("inner");
        std::fs::create_dir_all(inner.join(".git")).unwrap();
        assert_eq!(find_pin_root(&inner), None);
        // A worktree's .git is a file, and it bounds the search the same way.
        let worktree = repo.join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(worktree.join(".git"), "gitdir: ../.git/worktrees/worktree\n").unwrap();
        assert_eq!(find_pin_root(&worktree), None);
        write_pin(&worktree, "");
        assert_eq!(find_pin_root(&worktree).as_deref(), Some(worktree.as_path()));
    }

    #[test]
    fn a_host_with_everything_set_up_has_no_findings() {
        let host = FakeHost::yes();
        let report = host.report();
        assert_eq!(report.findings, []);
        assert_eq!(report.notes, []);
        assert_eq!(report.repository.as_deref(), Some("owner/repo on Fake"));
        assert_eq!(
            *host.asked.borrow(),
            [
                "app bot",
                "secret BOT_APP_ID",
                "secret BOT_PRIVATE_KEY",
                "check storm/review on main"
            ]
        );
    }

    #[test]
    fn each_no_from_the_host_is_a_finding() {
        let no = FakeHost {
            app: Answer::No,
            app_id: Answer::No,
            private_key: Answer::No,
            review: Answer::No,
            ..FakeHost::yes()
        };
        let report = no.report();
        assert_eq!(report.notes, []);
        assert_eq!(
            report.findings.iter().map(|f| f.to_string()).collect::<Vec<_>>(),
            [
                "bot is not installed",
                "secret BOT_APP_ID is not available to this repository",
                "secret BOT_PRIVATE_KEY is not available to this repository",
                "storm/review is not required on main",
            ]
        );
        // Each answer stands alone.
        let one = |host: FakeHost| host.report().findings;
        assert_eq!(
            one(FakeHost { app: Answer::No, ..FakeHost::yes() }),
            [Finding::AppMissing { app: "bot".into() }]
        );
        let suspended =
            one(FakeHost { app: Answer::Suspended("its installation on o was suspended".into()), ..FakeHost::yes() });
        assert_eq!(
            suspended,
            [Finding::AppSuspended { app: "bot".into(), reason: "its installation on o was suspended".into() }]
        );
        assert_eq!(
            suspended[0].to_string(),
            "bot cannot act: its installation on o was suspended"
        );
        assert_eq!(
            one(FakeHost { app_id: Answer::No, ..FakeHost::yes() }),
            [Finding::SecretMissing { name: "BOT_APP_ID".into() }]
        );
        assert_eq!(
            one(FakeHost { private_key: Answer::No, ..FakeHost::yes() }),
            [Finding::SecretMissing { name: "BOT_PRIVATE_KEY".into() }]
        );
        assert_eq!(
            one(FakeHost { review: Answer::No, ..FakeHost::yes() }),
            [Finding::CheckNotRequired { check: "storm/review".into(), branch: "main".into() }]
        );
    }

    #[test]
    fn the_app_and_its_secrets_come_from_fab_settings() {
        let root = tempfile::tempdir().unwrap();
        let settings = |text: &str| {
            std::fs::create_dir_all(root.path().join(".github")).unwrap();
            std::fs::write(root.path().join(".github/fabrial.toml"), text).unwrap();
            read_app(root.path())
        };
        assert_eq!(
            read_app(root.path()),
            Err(NoApp::Absent("there is no .github/fabrial.toml".to_owned()))
        );
        // The shape fab 0.9.1 writes: lists and a [tasks] table beside the keys.
        let full = "owner = \"o\"\napp = \"my-bot\"\nsecret_prefix = \"MY_BOT\"\n\
                    planning = [\".arcl/workspace/\"]\nversion = \"0.9.1\"\n[tasks]\nshow = \"arcl task show {id}\"\n";
        let app = settings(full).unwrap();
        assert_eq!(app, App { slug: "my-bot".into(), secret_prefix: "MY_BOT".into() });
        assert_eq!(app.secrets(), ["MY_BOT_APP_ID", "MY_BOT_PRIVATE_KEY"]);
        // No prefix reads the secrets fab 0.9.0 and earlier named.
        let legacy = settings("app = \"stormlight-thunderbot\"\n").unwrap();
        assert_eq!(legacy.secrets(), ["THUNDERBOT_APP_ID", "THUNDERBOT_PRIVATE_KEY"]);
        assert_eq!(
            settings("owner = \"o\"\nversion = \"0.8.0\"\n"),
            Err(NoApp::Absent(".github/fabrial.toml names no app".to_owned()))
        );
        assert!(matches!(
            settings("app = "),
            Err(NoApp::Absent(reason)) if reason.starts_with(".github/fabrial.toml could not be read as TOML")
        ));
    }

    #[test]
    fn an_app_or_prefix_fab_refuses_is_invalid() {
        let root = tempfile::tempdir().unwrap();
        let settings = |text: &str| {
            std::fs::create_dir_all(root.path().join(".github")).unwrap();
            std::fs::write(root.path().join(".github/fabrial.toml"), text).unwrap();
            read_app(root.path())
        };
        let invalid = |pairs: &[(&str, &str)]| {
            Err(NoApp::Invalid(
                pairs
                    .iter()
                    .map(|(key, reason)| (key.to_string(), reason.to_string()))
                    .collect(),
            ))
        };
        let slug = "is not a GitHub App slug: lowercase letters, digits and hyphens, not at either end, up to 34 \
                    characters";
        let prefix = "is not a secret name prefix: uppercase letters, digits and underscores, not starting with a \
                      digit or GITHUB_";
        assert_eq!(settings("app = \"Not A Slug\"\n"), invalid(&[("app", slug)]));
        assert_eq!(
            settings("app = \"bot\"\nsecret_prefix = \"lower\"\n"),
            invalid(&[("secret_prefix", prefix)])
        );
        assert_eq!(settings("app = 1\n"), invalid(&[("app", "is not a string")]));
        assert_eq!(
            settings("app = \"bot\"\nsecret_prefix = [\"BOT\"]\n"),
            invalid(&[("secret_prefix", "is not a string")])
        );
        // Both keys are named, even without an app to check.
        assert_eq!(
            settings("app = \"\"\nsecret_prefix = 2\n"),
            invalid(&[("app", "is empty"), ("secret_prefix", "is not a string")])
        );
        assert_eq!(
            settings("secret_prefix = \"GITHUB_BOT\"\n"),
            invalid(&[("secret_prefix", prefix)])
        );
    }

    #[test]
    fn an_invalid_app_is_a_finding_and_doctor_asks_nothing_about_it() {
        let host = FakeHost::yes();
        let app = NoApp::Invalid(vec![
            ("app".into(), "is not a string".into()),
            ("secret_prefix".into(), "is empty".into()),
        ]);
        let report = host.report_for(Err(&app));
        assert_eq!(*host.asked.borrow(), ["check storm/review on main"]);
        assert!(report.notes.is_empty());
        assert_eq!(
            report.findings.iter().map(|f| f.to_string()).collect::<Vec<_>>(),
            [
                ".github/fabrial.toml: app is not a string; doctor did not check the App or its secrets",
                ".github/fabrial.toml: secret_prefix is empty; doctor did not check the App or its secrets",
            ]
        );
    }

    #[test]
    fn without_an_app_doctor_skips_it_and_its_secrets_but_checks_the_branch() {
        let host = FakeHost { review: Answer::No, ..FakeHost::yes() };
        let report = host.report_for(Err(&NoApp::Absent(".github/fabrial.toml names no app".into())));
        assert_eq!(*host.asked.borrow(), ["check storm/review on main"]);
        assert_eq!(
            report.notes.iter().map(|n| n.to_string()).collect::<Vec<_>>(),
            ["skipped the App and secret checks: .github/fabrial.toml names no app"]
        );
        assert_eq!(
            report.findings,
            [Finding::CheckNotRequired { check: "storm/review".into(), branch: "main".into() }]
        );
    }

    #[test]
    fn an_answer_the_host_could_not_give_is_a_note_not_a_finding() {
        let unknown = |reason: &str| Answer::Unknown(reason.to_owned());
        let host = FakeHost {
            app: unknown("HTTP 403"),
            app_id: unknown("no admin access"),
            private_key: Answer::No,
            review: unknown("timed out"),
            ..FakeHost::yes()
        };
        let report = host.report();
        assert_eq!(
            report.findings,
            [Finding::SecretMissing { name: "BOT_PRIVATE_KEY".into() }]
        );
        assert_eq!(
            report.notes.iter().map(|n| n.to_string()).collect::<Vec<_>>(),
            [
                "could not check whether bot is installed: HTTP 403",
                "could not check secret BOT_APP_ID: no admin access",
                "could not check whether storm/review is required on main: timed out",
            ]
        );
        assert_eq!(
            Note::Skipped { reason: "gh not found".into() }.to_string(),
            "skipped repository checks: gh not found"
        );
    }
}
