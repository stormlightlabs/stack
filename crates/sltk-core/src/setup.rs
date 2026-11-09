//! Repository setup: `sltk init` runs each tool's own setup commands from the
//! train, passing fab's settings to `fab init`, writes the glue in [`glue`],
//! and then writes the pin.
//!
//! The pin records the sha256 of each platform's `sltk` archive, copied from
//! the `.sha256` files on this release, so `init` reads those before it runs
//! anything. It writes the pin only when every command succeeded and the
//! glue is written.
//!
//! Before the first command it writes a record of the run, [`RECORD`], in
//! the repository's git directory, and removes it after the pin. A rerun
//! skips `fab init` only when that record and fab's settings both match.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};

use crate::glue::{self, GlueError};
use crate::install::Fetch;
use crate::pin::{self, Pin, PinError};
use crate::train::{Platform, Tier, Train};

/// Where each `sltk` archive's `.sha256` file is published, with `{version}`
/// and `{target}` to replace.
pub const RELEASE_HASH_URL: &str =
    "https://github.com/stormlightlabs/stack/releases/download/v{version}/sltk-cli-{target}.tar.xz.sha256";

/// Where `fab init` writes fab's settings, relative to the repository root.
pub const FAB_SETTINGS: &str = ".github/fabrial.toml";

/// The secret prefix `fab init` writes when `sltk init` is given none: the
/// workflows then read `FABRIAL_APP_ID` and `FABRIAL_PRIVATE_KEY`.
pub const DEFAULT_SECRET_PREFIX: &str = "FABRIAL";

/// The secret prefix fab's workflows read when its settings name none, as
/// an install from fab 0.9.0 or earlier does until `fab update` writes it.
pub const LEGACY_SECRET_PREFIX: &str = "THUNDERBOT";

/// The file in the repository's git directory where `init` records a run
/// that has not written the pin yet, as `git rev-parse --git-path` names it.
pub const RECORD: &str = "sltk-init";

/// What to do about fab's settings that fresh setup refuses. `sltk init`
/// adopts a repository with fab's settings only when git holds no record of
/// an earlier fresh run, since that record means a fresh run is unfinished.
const ADOPT: &str = "to adopt the repository with the settings it has, \
     remove sltk-init from the git directory and rerun sltk init without --owner, --app, --secret-prefix, \
     --trailer and --gates";

/// Why `init` refuses a repository that already has a pin.
const PINNED: &str = "sltk init sets up a repository once and never replaces the pin";

/// Why a download of a `.sha256` file can fail even when the network works.
const UNPUBLISHED: &str =
    "sltk init pins a published sltk release, and stormlightlabs/stack publishes none before 0.1.0";

/// The longest `.sha256` file [`ReleaseHashes`] reads, in bytes. dist writes
/// the digest and the archive name on one line.
const MAX_HASH_FILE: u64 = 4096;

/// The longest fab settings file or run record `init` reads, in bytes.
const MAX_SETTINGS_FILE: u64 = 64 * 1024;

/// A setting `sltk init` passes to `fab init`, by its flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    /// `--owner`: the GitHub login of the person who approves pull requests.
    Owner,
    /// `--app`: the slug of the GitHub App the workflows act as.
    App,
    /// `--secret-prefix`: the prefix of the secrets holding the App's ID and
    /// key.
    SecretPrefix,
    /// `--trailer`: the squash commit's Co-authored-by value, `Name <email>`.
    Trailer,
    /// `--gates`: the command agents run before pushing.
    Gates,
}

impl Flag {
    /// Every flag, in the order `fab init` takes them.
    pub const ALL: [Flag; 5] = [Flag::Owner, Flag::App, Flag::SecretPrefix, Flag::Trailer, Flag::Gates];

    /// The flags `fab init` requires, which `sltk init` asks for on a
    /// terminal. [`Flag::SecretPrefix`] has a default,
    /// [`DEFAULT_SECRET_PREFIX`].
    pub const REQUIRED: [Flag; 4] = [Flag::Owner, Flag::App, Flag::Trailer, Flag::Gates];

    /// The flag as typed, such as `--owner`.
    pub fn name(self) -> &'static str {
        match self {
            Flag::Owner => "--owner",
            Flag::App => "--app",
            Flag::SecretPrefix => "--secret-prefix",
            Flag::Trailer => "--trailer",
            Flag::Gates => "--gates",
        }
    }

    /// Checks a value for this flag, so a value `fab init` would refuse fails
    /// before any download or answer is spent on it.
    ///
    /// fab's own rules come from fab 0.9.1's `Settings::validate`,
    /// `is_login`, `is_app_slug` and `is_secret_prefix` in
    /// `crates/fab-core/src/settings.rs`: no value is empty or holds a quote,
    /// a backslash or a line break, the owner is a GitHub login without the
    /// `@`, the App is a slug and the prefix can start a secret name. sltk adds that a value holds no control
    /// character and no space at either end, and that a trailer reads
    /// `Name <email>`, the form a Co-authored-by line takes.
    pub fn check(self, value: &str) -> Result<(), String> {
        if value.is_empty() {
            return Err("is empty".to_owned());
        }
        if value.chars().any(char::is_control) {
            return Err("holds a control character or line break".to_owned());
        }
        if value.contains(['"', '\\']) {
            return Err("holds a quote or a backslash, which fab cannot write".to_owned());
        }
        if value.trim() != value {
            return Err("starts or ends with a space".to_owned());
        }
        match self {
            Flag::Owner if value.starts_with('@') => Err("is a login without the @".to_owned()),
            Flag::Owner if !is_login(value) => Err(
                "is not a GitHub login: letters, digits and single hyphens, not at either end, up to 39 characters"
                    .to_owned(),
            ),
            Flag::App if !is_app_slug(value) => Err(
                "is not a GitHub App slug: lowercase letters, digits and hyphens, not at either end, up to 34 \
                 characters"
                    .to_owned(),
            ),
            Flag::SecretPrefix if !is_secret_prefix(value) => Err(
                "is not a secret name prefix: uppercase letters, digits and underscores, not starting with a \
                 digit or GITHUB_"
                    .to_owned(),
            ),
            Flag::Trailer if !is_trailer(value) => Err("is not Name <email>".to_owned()),
            _ => Ok(()),
        }
    }
}

/// Why `sltk init` stopped.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    /// The repository already has a pin, and `init` never overwrites it.
    #[error("sltk: {} already exists; {PINNED}", path.display())]
    PinExists { path: PathBuf },
    /// The repository has fab's settings file, and it is not the one an
    /// earlier `sltk init` with the same flags wrote.
    #[error("sltk: {} {reason}; {ADOPT}", path.display())]
    FabInstalled { path: PathBuf, reason: String },
    /// The record of an earlier run could not be read, or the record of this
    /// one could not be written.
    #[error("sltk: {} {reason}", path.display())]
    Record { path: PathBuf, reason: String },
    /// The repository's git directory, where `init` records its run, could
    /// not be found.
    #[error("sltk: could not find the git directory of {}: {reason}; sltk init runs in a git repository", root.display())]
    NoGitDir { root: PathBuf, reason: String },
    /// A fab setting is not valid.
    #[error("sltk: {flag} {reason}")]
    BadFlag { flag: &'static str, reason: String },
    /// The sha256 of this release's archive for a platform could not be read.
    #[error("sltk: could not read the sha256 of the sltk {version} archive for {platform}: {message}")]
    Hash {
        version: String,
        platform: Platform,
        message: String,
    },
    /// The pin could not be checked or written.
    #[error("sltk: {0}")]
    Pin(#[from] PinError),
    /// The Tasks section, the session hook or its settings entry could not
    /// be written.
    #[error(transparent)]
    Glue(#[from] GlueError),
    /// Adoption could not use fab's settings: the file cannot be read, or a
    /// flag given differs from it.
    #[error("sltk: {} {reason}", path.display())]
    Adopt { path: PathBuf, reason: String },
    /// `arcl skill install --check` failed with a status other than the one
    /// that means a skill file is missing or differs.
    #[error("sltk: {command} failed with {status}; sltk changed nothing")]
    Check { command: String, status: ExitStatus },
    /// A program adoption runs is not on `PATH`.
    #[error(
        "sltk: {program} is not in an absolute PATH directory, and sltk init runs it to adopt this repository. \
             Install it with sltk install and rerun sltk init; sltk changed nothing"
    )]
    NoProgram { program: String },
    /// A file changed between the diff adoption showed and the write.
    #[error("sltk: {} changed after sltk showed the diff; sltk changed nothing, so rerun sltk init", path.display())]
    Changed { path: PathBuf },
    /// A setup command could not be started.
    #[error("sltk: could not run {command} in {}: {source}. {progress}", dir.display())]
    Spawn {
        command: String,
        dir: PathBuf,
        source: std::io::Error,
        progress: Progress,
    },
    /// A setup command exited with a failure.
    #[error("sltk: {command} failed with {status}. {progress}")]
    Failed {
        command: String,
        status: ExitStatus,
        progress: Progress,
    },
}

/// Gives the sha256 of this `sltk` release's archive for a platform.
/// [`ReleaseHashes`] is the real one; tests supply fixed hashes.
pub trait HashSource {
    /// The digest for `platform` in lowercase hexadecimal, or what went wrong.
    fn sha256(&self, platform: Platform) -> Result<String, String>;
}

/// The settings `sltk init` passes to `fab init`, each checked by
/// [`Flag::check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FabSettings {
    /// The GitHub login of the person who approves pull requests.
    pub owner: String,
    /// The slug of the GitHub App the workflows act as.
    pub app: String,
    /// The prefix of the secrets holding the App's ID and key.
    pub secret_prefix: String,
    /// The squash commit's Co-authored-by value, as `Name <email>`.
    pub trailer: String,
    /// The command agents run before pushing.
    pub gates: String,
}

impl FabSettings {
    /// Checks each value and returns the settings.
    pub fn new(
        owner: String, app: String, secret_prefix: String, trailer: String, gates: String,
    ) -> Result<FabSettings, SetupError> {
        let settings = FabSettings { owner, app, secret_prefix, trailer, gates };
        for flag in Flag::ALL {
            flag.check(settings.get(flag))
                .map_err(|reason| SetupError::BadFlag { flag: flag.name(), reason })?;
        }
        Ok(settings)
    }

    /// The value for `flag`.
    pub fn get(&self, flag: Flag) -> &str {
        match flag {
            Flag::Owner => &self.owner,
            Flag::App => &self.app,
            Flag::SecretPrefix => &self.secret_prefix,
            Flag::Trailer => &self.trailer,
            Flag::Gates => &self.gates,
        }
    }
}

/// What `sltk init` works on.
pub struct Init<'a> {
    /// The repository root, where every setup command runs.
    pub root: &'a Path,
    /// The settings for `fab init`.
    pub settings: &'a FabSettings,
    /// The optional tools the pin lists.
    pub with: &'a [String],
    /// Where the `sltk` archive hashes come from.
    pub hashes: &'a dyn HashSource,
    /// The `PATH` to find the setup commands on and run them with, or `None`
    /// for the one `sltk` runs with. Only its absolute entries are searched.
    pub path: Option<&'a OsStr>,
}

/// Where a failed `sltk init` stopped: the setup commands it ran and the
/// ones it did not reach. A rerun with the same flags finishes the setup,
/// since it skips a `fab init` that already ran and the arcl commands are
/// safe to run again.
#[derive(Debug)]
pub struct Progress {
    /// The commands that succeeded, as command lines.
    pub ran: Vec<String>,
    /// The commands after the failed one, as command lines.
    pub not_run: Vec<String>,
}

impl std::fmt::Display for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let list = |lines: &[String]| if lines.is_empty() { "none".to_owned() } else { lines.join("; ") };
        write!(
            f,
            "Ran: {}. Not run: {}. sltk did not write the pin; rerun sltk init with the same flags to finish",
            list(&self.ran),
            list(&self.not_run)
        )
    }
}

/// Reads each hash from the `.sha256` file published beside the archive, at
/// [`RELEASE_HASH_URL`].
pub struct ReleaseHashes<'a> {
    /// The release, such as `0.1.0`.
    pub version: &'a str,
    /// How to download the file.
    pub fetch: &'a dyn Fetch,
}

impl HashSource for ReleaseHashes<'_> {
    fn sha256(&self, platform: Platform) -> Result<String, String> {
        let url = RELEASE_HASH_URL
            .replace("{version}", self.version)
            .replace("{target}", platform.as_str());
        let reader = self
            .fetch
            .open(&url)
            .map_err(|err| format!("{url}: {err}; {UNPUBLISHED}"))?;
        let mut text = String::new();
        reader
            .take(MAX_HASH_FILE)
            .read_to_string(&mut text)
            .map_err(|err| format!("{url}: {err}"))?;
        parse_digest(&text).ok_or_else(|| format!("{url} does not start with a 64-digit hexadecimal sha256"))
    }
}

/// What `init` records in [`RECORD`] before its first command: the flags it
/// passes to `fab init` and the train's fab version, which that fab writes as
/// the settings' `version`.
#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
struct Record {
    owner: String,
    app: String,
    secret_prefix: String,
    trailer: String,
    gates: String,
    fab: String,
}

impl Record {
    /// The value for `flag`.
    fn get(&self, flag: Flag) -> &str {
        match flag {
            Flag::Owner => &self.owner,
            Flag::App => &self.app,
            Flag::SecretPrefix => &self.secret_prefix,
            Flag::Trailer => &self.trailer,
            Flag::Gates => &self.gates,
        }
    }

    /// The record as TOML. [`Flag::check`] keeps quotes, backslashes and
    /// line breaks out of the flags, so each goes between quotes as it is.
    fn render(&self) -> String {
        format!(
            "# sltk init passed these settings to fab init and has not written the pin yet.\n\
             owner = \"{}\"\napp = \"{}\"\nsecret_prefix = \"{}\"\ntrailer = \"{}\"\ngates = \"{}\"\n\
             fab = \"{}\"\n",
            self.owner, self.app, self.secret_prefix, self.trailer, self.gates, self.fab
        )
    }
}

/// The setup commands `init` runs, in train order: those of every essential
/// tool and of each tool in `with`. `fab init` gets every [`Flag`].
pub fn commands(train: &Train, settings: &FabSettings, with: &[String]) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    for tool in &train.tools {
        if tool.tier != Tier::Essential && !with.contains(&tool.name) {
            continue;
        }
        for command in &tool.setup {
            let mut command = command.clone();
            if tool.name == "fab" && is_fab_init(&command) {
                for flag in Flag::ALL {
                    command.extend([flag.name().to_owned(), settings.get(flag).to_owned()]);
                }
            }
            commands.push(command);
        }
    }
    commands
}

/// Runs `sltk init`: checks the repository, reads this release's archive
/// hashes and checks the pin, records the run in the git directory, runs
/// each of [`commands`] in the repository root, stopping at the first
/// failure, writes the glue, and then writes the pin and removes the record.
/// Writes the command lines and the paths it wrote to `output`.
///
/// Refuses a repository that already has a pin, or fab's settings file that
/// an earlier `init` with the same flags and the train's fab did not leave.
/// When it did, `init` skips `fab init`, which refuses to run twice, and runs
/// the rest.
pub fn init(train: &Train, version: &str, init: Init, output: &mut dyn Write) -> Result<Pin, SetupError> {
    let path = init.root.join(pin::PATH);
    if path.exists() {
        return Err(SetupError::PinExists { path });
    }
    let fab_version = train
        .tools
        .iter()
        .find(|tool| tool.name == "fab")
        .map_or("", |tool| tool.version.as_str());
    let record = Record {
        owner: init.settings.owner.clone(),
        app: init.settings.app.clone(),
        secret_prefix: init.settings.secret_prefix.clone(),
        trailer: init.settings.trailer.clone(),
        gates: init.settings.gates.clone(),
        fab: fab_version.to_owned(),
    };
    let record_path = record_path(init.root)?;
    let fab_done = fab_installed(init.root, &record_path, &record)?;
    let mut sha256 = std::collections::BTreeMap::new();
    for platform in pin::TARGETS {
        let hash = init.hashes.sha256(platform).map_err(|message| SetupError::Hash {
            version: version.to_owned(),
            platform,
            message,
        })?;
        sha256.insert(platform, hash);
    }
    let pin = Pin { version: version.to_owned(), sha256, optional: init.with.to_vec() };
    // Checks the pin before any command changes the repository.
    pin.render(train)?;
    let commands = commands(train, init.settings, init.with);
    let lines: Vec<String> = commands.iter().map(|command| command_line(command)).collect();
    // Removing an earlier record first replaces a symlink rather than
    // writing through it.
    let write_record = || match std::fs::remove_file(&record_path) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
        _ => std::fs::write(&record_path, record.render()),
    };
    write_record().map_err(|err| SetupError::Record {
        path: record_path.clone(),
        reason: format!("could not be written: {err}"),
    })?;
    let mut ran = Vec::new();
    for (i, command) in commands.iter().enumerate() {
        let line = &lines[i];
        // The commands still run when stdout is closed, so writes to it may fail.
        if fab_done && is_fab_init(command) {
            let _ = writeln!(output, "skipping fab init: {FAB_SETTINGS} already holds these settings");
            continue;
        }
        let _ = writeln!(output, "running {line}");
        let progress = || Progress { ran: ran.clone(), not_run: lines[i + 1..].to_vec() };
        let spawn = |source| SetupError::Spawn {
            command: line.clone(),
            dir: init.root.to_owned(),
            source,
            progress: progress(),
        };
        let status = run(command, init.root, init.path).map_err(spawn)?;
        if !status.success() {
            return Err(SetupError::Failed { command: line.clone(), status, progress: progress() });
        }
        ran.push(line.clone());
    }
    for path in glue::write(init.root)? {
        let _ = writeln!(output, "wrote {path}");
    }
    pin.write(init.root, train)?;
    let _ = writeln!(output, "wrote {}", pin::PATH);
    if let Err(err) = std::fs::remove_file(&record_path) {
        let _ = writeln!(output, "could not remove {}: {err}", record_path.display());
    }
    Ok(pin)
}

/// The absolute path of `name` on `path`: the first absolute directory in it
/// that holds a regular file named `name` with an execute bit. Empty and
/// relative entries are skipped, so a program in the repository is never run
/// for a bare name. A name holding a `/` is not searched.
pub fn find_program(name: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    if name.is_empty() || name.contains('/') {
        return None;
    }
    std::env::split_paths(path?)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

/// Runs `command` in `root` and waits for it. The program is found with
/// [`find_program`] on `path`, or on the `PATH` `sltk` runs with when `path`
/// is `None`, and the command gets `path` as its `PATH` when given.
pub fn run(command: &[String], root: &Path, path: Option<&OsStr>) -> std::io::Result<ExitStatus> {
    prepare(command, root, path)?.status()
}

/// [`run`], with the command's standard output captured and returned with
/// its status. Its standard error still goes to `sltk`'s.
pub fn run_captured(command: &[String], root: &Path, path: Option<&OsStr>) -> std::io::Result<Output> {
    prepare(command, root, path)?
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
}

/// A path git printed on its own line, from its raw bytes. On Unix any bytes
/// make a path; elsewhere the path must be UTF-8.
pub fn path_from_git(mut stdout: Vec<u8>) -> Result<PathBuf, String> {
    while stdout.last().is_some_and(|b| matches!(b, b'\n' | b'\r')) {
        stdout.pop();
    }
    if stdout.is_empty() {
        return Err("git printed no path".to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(PathBuf::from(std::ffi::OsString::from_vec(stdout)))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(stdout)
            .map(PathBuf::from)
            .map_err(|_| "git printed a path that is not UTF-8".to_owned())
    }
}

/// The process [`run`] and [`run_captured`] start, not yet started.
fn prepare(command: &[String], root: &Path, path: Option<&OsStr>) -> std::io::Result<Command> {
    let search = path.map(OsStr::to_owned).or_else(|| std::env::var_os("PATH"));
    let program = find_program(&command[0], search.as_deref()).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("no executable file named {} in an absolute PATH directory", command[0]),
        )
    })?;
    let mut process = Command::new(program);
    process.args(&command[1..]).current_dir(root);
    if let Some(path) = path {
        process.env("PATH", path);
    }
    Ok(process)
}

/// Whether `command` is the train's `fab init`.
fn is_fab_init(command: &[String]) -> bool {
    command.len() >= 2 && command[0] == "fab" && command[1] == "init"
}

/// Whether fab is already installed in `root` by an earlier `init` that
/// stopped before the pin: `false` when there is no [`FAB_SETTINGS`], `true`
/// when it holds `record`'s value for each [`Flag`] and `record`'s fab as
/// its version, and the record at `record_path` matches `record` too, and an
/// error otherwise. Reads the settings as fab does, as TOML.
fn fab_installed(root: &Path, record_path: &Path, record: &Record) -> Result<bool, SetupError> {
    /// The keys of fab's settings file that `init` compares.
    #[derive(serde::Deserialize)]
    struct Installed {
        owner: String,
        app: Option<String>,
        secret_prefix: Option<String>,
        trailer: String,
        gates: String,
        version: String,
    }
    let path = root.join(FAB_SETTINGS);
    let refuse = |reason: String| SetupError::FabInstalled { path: path.clone(), reason };
    let Some(text) = read_small(&path).map_err(refuse)? else {
        return Ok(false);
    };
    let installed: Installed = toml::from_str(&text)
        .map_err(|err| refuse(format!("could not be read as fab's settings: {}", err.message())))?;
    let differ: Vec<&str> = [
        (Flag::Owner, Some(installed.owner)),
        (Flag::App, installed.app),
        (Flag::SecretPrefix, installed.secret_prefix),
        (Flag::Trailer, Some(installed.trailer)),
        (Flag::Gates, Some(installed.gates)),
    ]
    .into_iter()
    .filter(|(flag, value)| value.as_deref() != Some(record.get(*flag)))
    .map(|(flag, _)| flag.name())
    .collect();
    if !differ.is_empty() {
        return Err(refuse(format!("holds other values for {}", differ.join(", "))));
    }
    if installed.version != record.fab {
        return Err(refuse(format!(
            "was written by fab {}, not by the train's fab {}",
            installed.version, record.fab
        )));
    }
    let not_ours = || refuse("was not written by an earlier sltk init".to_owned());
    let bad_record = |reason: String| SetupError::Record { path: record_path.to_owned(), reason };
    let Some(text) = read_small(record_path).map_err(bad_record)? else {
        return Err(not_ours());
    };
    let earlier: Record =
        toml::from_str(&text).map_err(|err| bad_record(format!("could not be read: {}", err.message())))?;
    if &earlier != record {
        return Err(not_ours());
    }
    Ok(true)
}

/// Where `init` records its run for `root`: [`RECORD`] in the git directory,
/// as `git rev-parse --git-path` gives it, so a linked worktree gets its own.
pub(crate) fn record_path(root: &Path) -> Result<PathBuf, SetupError> {
    let fail = |reason: String| SetupError::NoGitDir { root: root.to_owned(), reason };
    let git = find_program("git", std::env::var_os("PATH").as_deref())
        .ok_or_else(|| fail("no executable file named git in an absolute PATH directory".to_owned()))?;
    let output = Command::new(git)
        .args(["rev-parse", "--git-path", RECORD])
        .current_dir(root)
        .output()
        .map_err(|err| fail(format!("could not run git: {err}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(fail(stderr.trim().to_owned()));
    }
    Ok(root.join(path_from_git(output.stdout).map_err(fail)?))
}

/// The text of the regular file at `path`, or `None` when there is none.
/// Refuses a symlink, anything but a regular file, and a file longer than
/// [`MAX_SETTINGS_FILE`], with the reason as the error.
pub(crate) fn read_small(path: &Path) -> Result<Option<String>, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("could not be read: {err}")),
    };
    if meta.file_type().is_symlink() {
        return Err("is a symlink, which sltk does not follow".to_owned());
    }
    if !meta.is_file() {
        return Err("is not a regular file".to_owned());
    }
    let too_long = || format!("is longer than {} KiB", MAX_SETTINGS_FILE / 1024);
    if meta.len() > MAX_SETTINGS_FILE {
        return Err(too_long());
    }
    let file = std::fs::File::open(path).map_err(|err| format!("could not be read: {err}"))?;
    let mut text = String::new();
    file.take(MAX_SETTINGS_FILE + 1)
        .read_to_string(&mut text)
        .map_err(|err| format!("could not be read: {err}"))?;
    if text.len() as u64 > MAX_SETTINGS_FILE {
        return Err(too_long());
    }
    Ok(Some(text))
}

/// Whether `path` is a regular file, after symlinks, with an execute bit.
fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
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

/// The digest at the start of a `.sha256` file, lowercased, when it is 64
/// hexadecimal digits.
fn parse_digest(text: &str) -> Option<String> {
    let digest = text.split_whitespace().next()?.to_ascii_lowercase();
    (digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())).then_some(digest)
}

/// Whether `owner` is a GitHub login: ASCII letters, digits and single
/// hyphens, not starting or ending with a hyphen, up to 39 characters. A copy
/// of `is_login` in fab 0.9.1's `crates/fab-core/src/settings.rs`.
fn is_login(owner: &str) -> bool {
    !owner.is_empty()
        && owner.len() <= 39
        && owner.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !owner.starts_with('-')
        && !owner.ends_with('-')
        && !owner.contains("--")
}

/// Whether `app` is a GitHub App slug: lowercase ASCII letters, digits and
/// hyphens, not starting or ending with a hyphen, up to 34 characters. A copy
/// of `is_app_slug` in fab 0.9.1's `crates/fab-core/src/settings.rs`.
fn is_app_slug(app: &str) -> bool {
    !app.is_empty()
        && app.len() <= 34
        && app
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !app.starts_with('-')
        && !app.ends_with('-')
}

/// Whether `prefix` can start a secret name GitHub accepts: uppercase ASCII
/// letters, digits and underscores, not starting with a digit or `GITHUB_`.
/// A copy of `is_secret_prefix` in fab 0.9.1's
/// `crates/fab-core/src/settings.rs`.
fn is_secret_prefix(prefix: &str) -> bool {
    !prefix.is_empty()
        && prefix
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        && !prefix.starts_with(|c: char| c.is_ascii_digit())
        && prefix != "GITHUB"
        && !prefix.starts_with("GITHUB_")
}

/// Whether `value` reads `Name <email>`: a name, a space, and an address with
/// text on both sides of one `@`, in angle brackets that end the value.
fn is_trailer(value: &str) -> bool {
    let Some((name, rest)) = value.split_once(" <") else {
        return false;
    };
    let Some(email) = rest.strip_suffix('>') else {
        return false;
    };
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !name.trim().is_empty()
        && !name.contains(['<', '>'])
        && !local.is_empty()
        && !domain.is_empty()
        && !local.contains(['<', '>', ' '])
        && !domain.contains(['<', '>', ' ', '@'])
}

/// A command as a person would type it, with each argument that holds
/// anything but letters, digits and `-_./=:@` in single quotes.
pub(crate) fn command_line(command: &[String]) -> String {
    let plain = |arg: &str| !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@".contains(c));
    command
        .iter()
        .map(
            |arg| {
                if plain(arg) { arg.clone() } else { format!("'{}'", arg.replace('\'', r"'\''")) }
            },
        )
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::{FabSettings, Flag, HashSource, ReleaseHashes, command_line, commands, parse_digest};
    use crate::install::Fetch;
    use crate::train::{Platform, Train};

    /// Serves `body` for one URL and fails for any other.
    struct OneFile {
        url: &'static str,
        body: String,
    }

    impl Fetch for OneFile {
        fn open(&self, url: &str) -> Result<Box<dyn Read>, String> {
            if url == self.url {
                Ok(Box::new(std::io::Cursor::new(self.body.clone().into_bytes())))
            } else {
                Err("404 Not Found".to_owned())
            }
        }
    }

    #[test]
    fn a_release_hash_comes_from_the_sha256_file_beside_the_archive() {
        let url = "https://github.com/stormlightlabs/stack/releases/download/v0.1.0/sltk-cli-x86_64-unknown-linux-gnu.tar.xz.sha256";
        let fetch = OneFile { url, body: format!("{DIGEST}  sltk-cli-x86_64-unknown-linux-gnu.tar.xz\n") };
        let hashes = ReleaseHashes { version: "0.1.0", fetch: &fetch };
        assert_eq!(hashes.sha256(Platform::LinuxX86_64).as_deref(), Ok(DIGEST));
        let err = hashes.sha256(Platform::AppleSilicon).unwrap_err();
        assert!(
            err.contains("v0.1.0/sltk-cli-aarch64-apple-darwin.tar.xz.sha256: 404 Not Found"),
            "{err}"
        );
        assert!(err.ends_with("publishes none before 0.1.0"), "{err}");
        let garbled = OneFile { url, body: "<html>".to_owned() };
        let err = ReleaseHashes { version: "0.1.0", fetch: &garbled }
            .sha256(Platform::LinuxX86_64)
            .unwrap_err();
        assert!(
            err.ends_with("does not start with a 64-digit hexadecimal sha256"),
            "{err}"
        );
    }

    const DIGEST: &str = "bd8c707af7c6088c1afdf57ff5b9e9f1c4f4830d0dabf1432c4b5ebaced33fe0";

    fn settings() -> FabSettings {
        FabSettings::new(
            "octo".into(),
            "octo-bot".into(),
            "OCTO".into(),
            "Ada Lovelace <ada@example.com>".into(),
            "just check".into(),
        )
        .unwrap()
    }

    #[test]
    fn an_app_must_be_a_slug_and_a_prefix_must_start_a_secret_name() {
        let long = "x".repeat(34);
        for good in ["stormlight-thunderbot", "a", "bot2", long.as_str()] {
            assert_eq!(Flag::App.check(good), Ok(()), "{good}");
        }
        let longer = "x".repeat(35);
        for bad in ["Bot", "-bot", "bot-", "my_bot", "bot[bot]", longer.as_str()] {
            assert!(
                Flag::App
                    .check(bad)
                    .unwrap_err()
                    .starts_with("is not a GitHub App slug"),
                "{bad}"
            );
        }
        for good in ["FABRIAL", "THUNDERBOT", "MY_BOT2", "_X", "GITHUBX"] {
            assert_eq!(Flag::SecretPrefix.check(good), Ok(()), "{good}");
        }
        for bad in ["fabrial", "2BOT", "GITHUB", "GITHUB_BOT", "MY-BOT"] {
            assert!(
                Flag::SecretPrefix
                    .check(bad)
                    .unwrap_err()
                    .starts_with("is not a secret name prefix"),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_trailer_must_read_name_and_email() {
        for good in ["Ada <ada@example.com>", "Ada Lovelace <a.b+c@mail.example>"] {
            assert_eq!(Flag::Trailer.check(good), Ok(()), "{good}");
        }
        for bad in [
            "ada@example.com",
            "<ada@example.com>",
            "Ada <ada@example.com",
            "Ada <adaexample.com>",
            "Ada <@example.com>",
            "Ada <ada@>",
            "Ada <ada@x@y>",
            "Ada <ada@example.com> extra",
            "Ada <a b@example.com>",
        ] {
            assert_eq!(Flag::Trailer.check(bad), Err("is not Name <email>".to_owned()), "{bad}");
        }
    }

    #[test]
    fn every_flag_takes_one_trimmed_line() {
        for flag in Flag::ALL {
            assert_eq!(flag.check(""), Err("is empty".to_owned()));
            assert!(flag.check("a\nb").is_err());
            assert_eq!(flag.check(" a"), Err("starts or ends with a space".to_owned()));
        }
        assert_eq!(Flag::Gates.check("cargo test --locked"), Ok(()));
    }

    #[test]
    fn no_flag_takes_a_quote_or_a_backslash() {
        let reason = Err("holds a quote or a backslash, which fab cannot write".to_owned());
        assert_eq!(Flag::Gates.check(r#"sh -c "just check""#), reason);
        assert_eq!(Flag::Gates.check(r"just check\"), reason);
        assert_eq!(Flag::Trailer.check(r#"Ada "A" <ada@example.com>"#), reason);
        assert_eq!(Flag::Owner.check(r"oc\to"), reason);
    }

    #[test]
    fn an_owner_must_be_a_github_login() {
        let long = "x".repeat(39);
        for good in ["desertthunder", "a", "a-b-c", "A1", long.as_str()] {
            assert_eq!(Flag::Owner.check(good), Ok(()), "{good}");
        }
        let longer = "x".repeat(40);
        for bad in ["a b", "-a", "a-", "a--b", "a_b", "a.b", "ä", longer.as_str()] {
            assert_eq!(
                Flag::Owner.check(bad),
                Err(
                    "is not a GitHub login: letters, digits and single hyphens, not at either end, up to 39 characters"
                        .to_owned()
                ),
                "{bad}"
            );
        }
        assert_eq!(Flag::Owner.check("@octo"), Err("is a login without the @".to_owned()));
    }

    #[test]
    fn a_bad_setting_names_its_flag() {
        let err = FabSettings::new(
            "octo".into(),
            "octo-bot".into(),
            "OCTO".into(),
            "nobody".into(),
            "just check".into(),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "sltk: --trailer is not Name <email>");
    }

    #[test]
    fn a_digest_is_the_first_word_of_the_file() {
        let upper = DIGEST.to_ascii_uppercase();
        assert_eq!(
            parse_digest(&format!("{DIGEST}  sltk-cli-x.tar.xz\n")).as_deref(),
            Some(DIGEST)
        );
        assert_eq!(parse_digest(&format!("{upper}\n")).as_deref(), Some(DIGEST));
        assert_eq!(parse_digest(""), None);
        assert_eq!(parse_digest("Not Found"), None);
        assert_eq!(parse_digest(&DIGEST[1..]), None);
    }

    #[test]
    fn a_command_line_quotes_what_a_shell_would_split() {
        let command = ["fab", "--trailer", "Ada <a@b.c>", "it's", "--owner=octo"].map(String::from);
        assert_eq!(
            command_line(&command),
            r"fab --trailer 'Ada <a@b.c>' 'it'\''s' --owner=octo"
        );
    }

    #[test]
    fn fab_init_gets_every_flag_and_arcl_its_own_commands() {
        let train = Train::embedded().unwrap();
        let got = commands(&train, &settings(), &[]);
        let want: Vec<Vec<String>> = [
            &[
                "fab",
                "init",
                "--owner",
                "octo",
                "--app",
                "octo-bot",
                "--secret-prefix",
                "OCTO",
                "--trailer",
                "Ada Lovelace <ada@example.com>",
                "--gates",
                "just check",
            ][..],
            &["arcl", "init", "--workspace"],
            &["arcl", "skill", "install"],
        ]
        .iter()
        .map(|c| c.iter().map(|s| s.to_string()).collect())
        .collect();
        assert_eq!(got, want);
    }
}
