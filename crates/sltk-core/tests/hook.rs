//! Runs the session hook `sltk init` writes, with bash, in a temporary
//! project and home. A `.tar.xz` archive built here holds a stub `sltk` and
//! stands in for the release download through `SLTK_HOOK_ARCHIVE`. The stub
//! records its calls and, on `install --yes`, puts a stub `arcl` where
//! `sltk install` would. A fake `uname` picks the target. Nothing here
//! reaches the network.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sha2::{Digest, Sha256};
use sltk_core::glue;
use sltk_core::setup::find_program;

const VERSION: &str = "9.8.7";
const TARGET: &str = "x86_64-unknown-linux-gnu";
const OTHER: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A project with the written hook, a home, a fake `uname` and a built
/// archive.
struct Session {
    dir: tempfile::TempDir,
    project: PathBuf,
    home: PathBuf,
    fakes: PathBuf,
    archive: PathBuf,
    sha256: String,
}

impl Session {
    /// Makes the session, with `uname -sm` printing `platform`.
    fn new(platform: &str) -> Session {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        let home = dir.path().join("home");
        let fakes = dir.path().join("fakes");
        for d in [&project, &home, &fakes] {
            std::fs::create_dir_all(d).unwrap();
        }
        glue::write(&project).unwrap();
        write_script(&fakes.join("uname"), &format!("#!/bin/sh\necho '{platform}'\n"));
        let archive = build_archive(dir.path());
        let sha256 = hex(&Sha256::digest(std::fs::read(&archive).unwrap()));
        let session = Session { dir, project, home, fakes, archive, sha256 };
        session.pin(&session.sha256.clone());
        session
    }

    /// Writes the pin with `linux` as the Linux archive's sha256.
    fn pin(&self, linux: &str) {
        std::fs::create_dir_all(self.project.join(".github")).unwrap();
        std::fs::write(
            self.project.join(".github/sltk.toml"),
            format!(
                "version = \"{VERSION}\"\nsha256-aarch64-apple-darwin = \"{OTHER}\"\nsha256-{TARGET} = \"{linux}\"\noptional = []\n"
            ),
        )
        .unwrap();
    }

    /// Runs the hook with bash, the fakes first on `path`, and `archive` as
    /// `SLTK_HOOK_ARCHIVE`.
    fn run_on(&self, path: &str, archive: &Path) -> Output {
        let bash = find_program("bash", std::env::var_os("PATH").as_deref()).expect("bash on PATH");
        Command::new(bash)
            .arg(self.project.join(glue::HOOK))
            .env_clear()
            .env("PATH", format!("{}:{path}", self.fakes.display()))
            .env("HOME", &self.home)
            .env("CLAUDE_CODE_REMOTE", "true")
            .env("CLAUDE_PROJECT_DIR", &self.project)
            .env("CLAUDE_ENV_FILE", self.env_file())
            .env("SLTK_HOOK_ARCHIVE", archive)
            .env("CALLS", self.calls())
            .output()
            .unwrap()
    }

    /// Runs the hook with the system's tools on `PATH`.
    fn run(&self) -> Output {
        self.run_on(&system_path(), &self.archive)
    }

    fn env_file(&self) -> PathBuf {
        self.dir.path().join("env")
    }

    fn calls(&self) -> PathBuf {
        self.dir.path().join("calls")
    }

    /// Where the hook installs `sltk`.
    fn sltk(&self) -> PathBuf {
        self.home.join(format!(".local/share/sltk/sltk-{VERSION}/bin/sltk"))
    }

    /// Where the stub `sltk install` puts `arcl`.
    fn arcl(&self) -> PathBuf {
        self.home.join(".cargo/bin/arcl")
    }

    fn read_calls(&self) -> String {
        std::fs::read_to_string(self.calls()).unwrap_or_default()
    }
}

/// The stub `sltk`: answers `--version` with [`VERSION`], and on any other
/// call records it in `$CALLS` and, for `install`, writes a stub `arcl` that
/// records its calls the same way.
const STUB_SLTK: &str = r#"#!/bin/sh
if [ "$1" = --version ]; then
  echo "sltk 9.8.7"
  exit 0
fi
echo "sltk $*" >> "$CALLS"
if [ "$1" = install ]; then
  mkdir -p "$HOME/.cargo/bin"
  printf '#!/bin/sh\necho "arcl $*" >> "$CALLS"\n' > "$HOME/.cargo/bin/arcl"
  chmod 755 "$HOME/.cargo/bin/arcl"
fi
"#;

/// Builds `sltk-cli-<target>.tar.xz` under `dir` the way dist lays it out:
/// one top-level directory holding the binary and a README.
fn build_archive(dir: &Path) -> PathBuf {
    let name = format!("sltk-cli-{TARGET}");
    let src = dir.join("src").join(&name);
    std::fs::create_dir_all(&src).unwrap();
    write_script(&src.join("sltk"), STUB_SLTK);
    std::fs::write(src.join("README.md"), "stub\n").unwrap();
    let archive = dir.join(format!("{name}.tar.xz"));
    let status = Command::new("tar")
        .arg("-cJf")
        .arg(&archive)
        .arg("-C")
        .arg(dir.join("src"))
        .arg(&name)
        .status()
        .unwrap();
    assert!(status.success());
    archive
}

fn write_script(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The test's own `PATH`, which has the system's tools.
fn system_path() -> String {
    std::env::var("PATH").unwrap_or_default()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn the_hook_installs_the_checked_sltk_and_then_arcl() {
    let session = Session::new("Linux x86_64");
    let output = session.run();
    assert!(output.status.success(), "{}", stderr(&output));
    let sltk = session.sltk();
    assert_eq!(std::fs::read_to_string(&sltk).unwrap(), STUB_SLTK);
    assert_eq!(std::fs::metadata(&sltk).unwrap().permissions().mode() & 0o777, 0o755);
    assert!(session.arcl().is_file());
    assert_eq!(
        session.read_calls(),
        "sltk install --yes\narcl init --quiet\narcl --version\n"
    );
    let path_line = format!(
        "export PATH=\"{}:{}:$PATH\"\n",
        sltk.parent().unwrap().display(),
        session.home.join(".cargo/bin").display()
    );
    assert_eq!(std::fs::read_to_string(session.env_file()).unwrap(), path_line);

    // A second session finds the pinned sltk and downloads nothing: the
    // archive it names does not exist.
    std::fs::create_dir_all(session.project.join(".arcl")).unwrap();
    std::fs::write(session.project.join(".arcl/arcl.db"), "").unwrap();
    let output = session.run_on(&system_path(), &session.dir.path().join("absent.tar.xz"));
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(std::fs::read_to_string(session.env_file()).unwrap(), path_line);
    assert!(
        session
            .read_calls()
            .ends_with("--version\nsltk install --yes\narcl tree\narcl --version\n")
    );
}

#[test]
fn an_archive_with_another_hash_fails_and_installs_nothing() {
    let session = Session::new("Linux x86_64");
    session.pin(OTHER);
    let output = session.run();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stderr(&output),
        format!(
            "session-start: sltk-cli-{TARGET}.tar.xz has sha256 {}, but .github/sltk.toml pins {OTHER}\n",
            session.sha256
        )
    );
    assert!(!session.home.join(".local/share/sltk").exists());
    assert!(!session.arcl().exists());
    assert_eq!(session.read_calls(), "");
}

/// A bin directory under `session` linking only the hook's tools, plus
/// `extra`, from the system's `PATH`.
fn bin_with(session: &Session, extra: &[&str]) -> String {
    let bin = session.dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let tools = [
        "sed", "tar", "xz", "mktemp", "cp", "mkdir", "chmod", "mv", "rm", "grep", "date",
    ];
    for tool in tools.iter().chain(extra) {
        let real = find_program(tool, Some(system_path().as_ref())).unwrap_or_else(|| panic!("{tool} on PATH"));
        std::os::unix::fs::symlink(real, bin.join(tool)).unwrap();
    }
    bin.display().to_string()
}

#[test]
fn without_sha256sum_the_hook_hashes_with_shasum() {
    let session = Session::new("Linux x86_64");
    let output = session.run_on(&bin_with(&session, &["shasum", "perl"]), &session.archive);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(std::fs::read_to_string(session.sltk()).unwrap(), STUB_SLTK);
    assert!(session.arcl().is_file());

    let session = Session::new("Linux x86_64");
    session.pin(OTHER);
    let output = session.run_on(&bin_with(&session, &["shasum", "perl"]), &session.archive);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains(&format!("has sha256 {}", session.sha256)),
        "{}",
        stderr(&output)
    );
}

#[test]
fn without_a_hashing_tool_the_hook_fails_naming_both() {
    let session = Session::new("Linux x86_64");
    let output = session.run_on(&bin_with(&session, &[]), &session.archive);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stderr(&output),
        "session-start: found neither sha256sum nor shasum on PATH, so the sltk archive cannot be checked\n"
    );
    assert!(!session.home.join(".local/share/sltk").exists());
    assert_eq!(session.read_calls(), "");
}

#[test]
fn a_missing_hash_line_or_platform_fails_naming_it() {
    let session = Session::new("Linux x86_64");
    std::fs::write(
        session.project.join(".github/sltk.toml"),
        format!("version = \"{VERSION}\"\nsha256-aarch64-apple-darwin = \"{OTHER}\"\n"),
    )
    .unwrap();
    let output = session.run();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stderr(&output),
        format!("session-start: .github/sltk.toml needs one line that reads sha256-{TARGET} = \"<sha256>\"\n")
    );

    let session = Session::new("Linux aarch64");
    let output = session.run();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stderr(&output),
        "session-start: sltk has no build for Linux aarch64; it supports Darwin arm64 and Linux x86_64\n"
    );
    assert_eq!(session.read_calls(), "");
}

#[test]
fn a_local_session_is_left_alone() {
    let session = Session::new("Linux x86_64");
    let bash = find_program("bash", std::env::var_os("PATH").as_deref()).unwrap();
    let output = Command::new(bash)
        .arg(session.project.join(glue::HOOK))
        .env_clear()
        .env("PATH", system_path())
        .env("HOME", &session.home)
        .env("CLAUDE_PROJECT_DIR", &session.project)
        .env("SLTK_HOOK_ARCHIVE", &session.archive)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!session.home.join(".local").exists());
}
