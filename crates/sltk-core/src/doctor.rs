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
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::pin::{self, Pin};
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
                "{}: pins sltk {pinned}, but this is sltk {running}, whose train doctor checks",
                pin::PATH
            ),
        }
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
}

/// Checks the tools `train` lists on `machine`.
///
/// The essentials are always checked. With a `pin`, so do the optional tools
/// it lists, and a pin of another release than `running` is a finding.
/// Without one, an optional tool is checked only when the bin directory has
/// it.
pub fn check(train: &Train, pin: Option<&Pin>, running: &str, machine: &Machine) -> Report {
    let mut report = Report { checked: Vec::new(), findings: Vec::new() };
    if let Some(pin) = pin.filter(|pin| pin.version != running) {
        report
            .findings
            .push(Finding::PinVersion { pinned: pin.version.clone(), running: running.to_owned() });
    }
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

/// The repository root at or above `start` that holds `.github/sltk.toml`.
/// The search stops at the first directory holding `.git`, so a repository
/// never takes the pin of one it sits inside.
pub fn find_pin_root(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        if dir.join(pin::PATH).is_file() {
            return Some(dir.to_owned());
        }
        if dir.join(".git").exists() {
            return None;
        }
    }
    None
}

/// The first `x.y.z` in `text`: three runs of digits joined by dots, not
/// preceded by a digit or a dot. `fab 0.8.0+c5fa630` gives `0.8.0`.
pub fn parse_version(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let digits = |from: usize| bytes[from..].iter().take_while(|b| b.is_ascii_digit()).count();
    for start in 0..bytes.len() {
        let after_other = start == 0 || !(bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'.');
        if !after_other || !bytes[start].is_ascii_digit() {
            continue;
        }
        let mut end = start;
        let mut runs = 0;
        while runs < 3 {
            let n = digits(end);
            if n == 0 {
                break;
            }
            end += n;
            runs += 1;
            if runs < 3 {
                if bytes.get(end) != Some(&b'.') {
                    break;
                }
                end += 1;
            }
        }
        if runs == 3 {
            return Some(&text[start..end]);
        }
    }
    None
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
            Ok(output) => match parse_version(&output) {
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
                    reason: "no x.y.z in its output".to_owned(),
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

/// Runs `<binary> --version` and returns what it wrote to stdout, up to
/// [`MAX_VERSION_OUTPUT`] bytes. Stops the binary after `timeout`.
fn run_version(binary: &Path, timeout: Duration) -> Result<String, String> {
    let mut child = spawn_version(binary).map_err(|err| format!("could not run it: {err}"))?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let (send, receive) = mpsc::channel();
    // The thread ends when the binary closes stdout or the cap is reached. A
    // binary that never does leaves it blocked on the pipe until sltk exits.
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.take(MAX_VERSION_OUTPUT).read_to_end(&mut bytes).map(|_| bytes);
        let _ = send.send(result);
    });
    let deadline = Instant::now() + timeout;
    let output = receive.recv_timeout(timeout);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("it did not exit within {} s", timeout.as_secs_f32()));
            }
        }
    }
    match output {
        Ok(Ok(bytes)) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Ok(Err(err)) => Err(format!("could not read its output: {err}")),
        Err(_) => Err(format!("it did not exit within {} s", timeout.as_secs_f32())),
    }
}

/// Starts `<binary> --version` with stdout piped, retrying while the OS
/// reports the file busy.
fn spawn_version(binary: &Path) -> io::Result<std::process::Child> {
    let mut attempt = 1;
    loop {
        let result = Command::new(binary)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
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

    use super::{Finding, Machine, Report, check, find_pin_root, parse_version};
    use crate::pin::Pin;
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

        fn check(&self, pin: Option<&Pin>, path: &[&str]) -> Report {
            check(&self.train, pin, "0.2.0", &self.machine(path))
        }
    }

    fn pin(version: &str, optional: &[&str]) -> Pin {
        Pin {
            version: version.to_owned(),
            sha256: Default::default(),
            optional: optional.iter().map(|s| s.to_string()).collect(),
        }
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
            [Finding::NoVersion { tool: "arcl".into(), path: arcl, reason: "no x.y.z in its output".into() }]
        );
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
        // Homebrew links bin/fab to its Cellar; doctor resolves the link.
        let cellar = fixture.dir("brew/Cellar/fab/0.7.0/bin");
        std::fs::create_dir_all(&cellar).unwrap();
        let real = fixture.fake("brew/Cellar/fab/0.7.0/bin", "fab", "fab 0.7.0");
        let link = fixture.dir("brew/bin/fab");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let report = fixture.check(None, &["brew/bin", "cargo/bin"]);
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
        let report = fixture.check(Some(&pin("0.1.0", &[])), &["cargo/bin"]);
        let finding = Finding::PinVersion { pinned: "0.1.0".into(), running: "0.2.0".into() };
        assert_eq!(report.findings, [finding]);
        assert_eq!(
            report.findings[0].to_string(),
            ".github/sltk.toml: pins sltk 0.1.0, but this is sltk 0.2.0, whose train doctor checks"
        );
        assert_eq!(fixture.check(Some(&pin("0.2.0", &[])), &["cargo/bin"]).findings, []);
    }

    #[test]
    fn a_pinned_optional_tool_is_checked() {
        let fixture = Fixture::new();
        fixture.good();
        let report = fixture.check(Some(&pin("0.2.0", &["mire"])), &["cargo/bin"]);
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
    fn the_first_x_y_z_is_the_version() {
        assert_eq!(parse_version("fab 0.8.0+c5fa630"), Some("0.8.0"));
        assert_eq!(parse_version("arcl 0.3.0\n"), Some("0.3.0"));
        assert_eq!(parse_version("tool v10.20.30-rc.1"), Some("10.20.30"));
        assert_eq!(parse_version("build 2 of 1.2 then 4.5.6"), Some("4.5.6"));
        assert_eq!(parse_version("1.2.3.4"), Some("1.2.3"));
        assert_eq!(parse_version("x 1.2."), None);
        assert_eq!(parse_version(""), None);
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
    }
}
