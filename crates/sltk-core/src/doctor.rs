//! `sltk doctor`: what is wrong with the tools on this machine. It finds each
//! tool's binary in the bin directory and on `PATH`, runs `<binary> --version`,
//! and reports what differs from the train. It changes nothing.
//!
//! Every input comes in through [`Machine`], so tests build a `PATH`, a bin
//! directory and Homebrew prefixes of their own.

use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use crate::pin::{self, Pin, PinError};
use crate::train::{Tier, Tool, Train};

/// How long `<binary> --version` may run before doctor stops it.
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// The most bytes doctor reads from `<binary> --version`.
const MAX_VERSION_OUTPUT: u64 = 4096;

/// Where Homebrew keeps what it installs, on Apple silicon, Intel macOS and
/// Linux. `$HOMEBREW_PREFIX` joins these in [`Machine::from_env`].
const BREW_PREFIXES: [&str; 3] = ["/opt/homebrew", "/usr/local/Cellar", "/home/linuxbrew/.linuxbrew"];

/// How often doctor checks whether `<binary> --version` has exited.
const POLL: Duration = Duration::from_millis(10);

/// How long doctor keeps reading `<binary> --version` once the binary has
/// exited. A background process it started may hold stdout open; doctor stops
/// reading after this and uses what it has.
const EXIT_GRACE: Duration = Duration::from_millis(200);

/// The largest `.github/sltk.toml` doctor reads.
const MAX_PIN_BYTES: u64 = 64 * 1024;

/// How many times doctor starts a binary that the OS reports busy, as when an
/// install is still writing it.
const BUSY_ATTEMPTS: u32 = 5;

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
}

/// Checks the tools `train` lists on `machine`.
///
/// The essentials are always checked. With a pin of this release, so are the
/// optional tools it lists. A pin of another release than `running`, or one
/// doctor refused to read, is a finding, and doctor then treats the
/// repository as unpinned: an optional tool is checked only when the bin
/// directory has it.
pub fn check(train: &Train, pin: Option<&RepoPin>, running: &str, machine: &Machine) -> Report {
    let mut report = Report { checked: Vec::new(), findings: Vec::new() };
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

/// The nearest directory at or above `start` that has a `.github/sltk.toml`,
/// searching no higher than the root of the git repository `start` is in.
/// That root is the nearest directory holding a `.git` entry, a directory in
/// a clone and a file in a worktree, so a repository never takes the pin of
/// one it sits inside. Outside a repository there is no pin. The pin is found
/// without following a symlink at its own path; [`read_pin`] refuses one.
pub fn find_pin_root(start: &Path) -> Option<PathBuf> {
    let repo = start.ancestors().find(|dir| exists(&dir.join(".git")))?;
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
/// [`MAX_VERSION_OUTPUT`] bytes.
///
/// The binary runs in a process group of its own. Once it exits, doctor reads
/// for up to [`EXIT_GRACE`] more and uses what it has, since a background
/// process the binary started may hold stdout open. If it has not exited
/// after `timeout`, that is the error. Either way doctor then kills the whole
/// group. A process that left the group, as with `setsid`, can still hold
/// the pipe; the reading thread then waits on it until sltk exits.
fn run_version(binary: &Path, timeout: Duration) -> Result<String, String> {
    let mut child = spawn_version(binary).map_err(|err| format!("could not run it: {err}"))?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let read = Arc::new(Mutex::new(Vec::new()));
    let (send, done) = mpsc::channel();
    let shared = Arc::clone(&read);
    std::thread::spawn(move || {
        let _ = send.send(read_capped(stdout, &shared));
    });
    let exited = wait_for_exit(&mut child, Instant::now() + timeout);
    let mut result = done.recv_timeout(if exited { EXIT_GRACE } else { Duration::ZERO });
    kill_group(&mut child);
    let _ = child.wait();
    if !exited {
        return Err(format!("it did not exit within {} s", timeout.as_secs_f32()));
    }
    if result.is_err() {
        // The kill closes the pipe for every process in the group, so the
        // thread may now finish what is buffered.
        result = done.recv_timeout(EXIT_GRACE);
    }
    if let Ok(Err(err)) = result {
        return Err(format!("could not read its output: {err}"));
    }
    let bytes = read.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Reads `from` into `into` until end of file or [`MAX_VERSION_OUTPUT`]
/// bytes, appending as it goes so a caller that stops waiting still has what
/// was read.
fn read_capped(mut from: impl Read, into: &Mutex<Vec<u8>>) -> io::Result<()> {
    let mut buf = [0; 512];
    let mut total = 0;
    while total < MAX_VERSION_OUTPUT {
        let want = buf.len().min((MAX_VERSION_OUTPUT - total) as usize);
        let n = match from.read(&mut buf[..want]) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        };
        into.lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend_from_slice(&buf[..n]);
        total += n as u64;
    }
    Ok(())
}

/// Waits until `child` exits or `deadline` passes, and says whether it
/// exited. On Unix the child is left unreaped, so its process group ID cannot
/// be reused before [`kill_group`] signals it.
fn wait_for_exit(child: &mut std::process::Child, deadline: Instant) -> bool {
    loop {
        #[cfg(unix)]
        let status = {
            use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
            let options = WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT;
            waitid(WaitId::Pid(Pid::from_child(child)), options).map(|status| status.is_some())
        };
        #[cfg(not(unix))]
        let status = child.try_wait().map(|status| status.is_some());
        match status {
            Ok(true) => return true,
            Ok(false) if Instant::now() < deadline => std::thread::sleep(POLL),
            _ => return false,
        }
    }
}

/// Kills every process in `child`'s process group on Unix, then `child`
/// itself, in case it moved to another group.
fn kill_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        use rustix::process::{Pid, Signal, kill_process_group};
        let _ = kill_process_group(Pid::from_child(child), Signal::KILL);
    }
    let _ = child.kill();
}

/// Starts `<binary> --version` with stdout piped, in a process group of its
/// own on Unix, retrying while the OS reports the file busy.
fn spawn_version(binary: &Path) -> io::Result<std::process::Child> {
    let mut attempt = 1;
    loop {
        let mut command = Command::new(binary);
        command
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        let result = command.spawn();
        match result {
            Err(err) if err.kind() == io::ErrorKind::ExecutableFileBusy && attempt < BUSY_ATTEMPTS => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            result => return result,
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{Finding, Machine, RepoPin, Report, check, find_pin_root, parse_version, read_pin};
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
}
