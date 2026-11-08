//! Runs `init` against fake `fab` and `arcl` scripts that record each call,
//! with fixed archive hashes in place of a published release.

#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use sltk_core::pin::{self, Pin};
use sltk_core::setup::{FabSettings, HashSource, Init, SetupError, init};
use sltk_core::train::{Platform, Train};

const DARWIN: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
const LINUX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// The hashes a release would publish.
struct Fixed;

impl HashSource for Fixed {
    fn sha256(&self, platform: Platform) -> Result<String, String> {
        Ok(match platform {
            Platform::AppleSilicon => DARWIN,
            Platform::LinuxX86_64 => LINUX,
        }
        .to_owned())
    }
}

/// A source with no release behind it.
struct Missing;

impl HashSource for Missing {
    fn sha256(&self, _: Platform) -> Result<String, String> {
        Err("404 Not Found".to_owned())
    }
}

/// A fake tool and repository: a bin directory holding `fab` and `arcl`
/// scripts that append their working directory and arguments, tab-separated,
/// to `calls`, and an empty repository root.
struct Fakes {
    _dir: tempfile::TempDir,
    bin: PathBuf,
    calls: PathBuf,
    root: PathBuf,
}

impl Fakes {
    /// Makes the fakes. `arcl skill` exits 7 when `arcl_skill_fails` is set.
    fn new(arcl_skill_fails: bool) -> Fakes {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        let calls = dir.path().join("calls");
        // Builtins only: PATH holds nothing but the fakes.
        let record = format!(
            "#!/bin/sh\n{{ printf '%s' \"$(pwd -P)\"; for arg; do printf '\\t%s' \"$arg\"; done; echo; }} >> '{}'\n",
            calls.display()
        );
        let fail = if arcl_skill_fails { "[ \"$1\" = skill ] && exit 7\n" } else { "" };
        write_script(&bin.join("fab"), &record);
        write_script(&bin.join("arcl"), &format!("{record}{fail}exit 0\n"));
        Fakes { _dir: dir, bin, calls, root }
    }

    /// Runs `init` with the fakes first on `PATH`.
    fn init(&self, with: &[String], hashes: &dyn HashSource) -> (Result<Pin, SetupError>, String) {
        let train = Train::embedded().unwrap();
        let settings = settings();
        let path = OsString::from(&self.bin);
        let mut output = Vec::new();
        let result = init(
            &train,
            "1.2.3",
            Init { root: &self.root, settings: &settings, with, hashes, path: Some(&path) },
            &mut output,
        );
        (result, String::from_utf8(output).unwrap())
    }

    /// Each recorded call as its working directory and arguments.
    fn calls(&self) -> Vec<Vec<String>> {
        let text = std::fs::read_to_string(&self.calls).unwrap_or_default();
        text.lines()
            .map(|line| line.split('\t').map(String::from).collect())
            .collect()
    }

    /// A call recorded in the repository root.
    fn call(&self, args: &[&str]) -> Vec<String> {
        let root = self.root.canonicalize().unwrap().display().to_string();
        std::iter::once(root)
            .chain(args.iter().map(|arg| arg.to_string()))
            .collect()
    }
}

fn write_script(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn settings() -> FabSettings {
    FabSettings::new(
        "octo".into(),
        "Ada Lovelace <ada@example.com>".into(),
        "just check".into(),
    )
    .unwrap()
}

#[test]
fn init_runs_fab_then_arcl_in_the_root_and_writes_the_pin() {
    let fakes = Fakes::new(false);
    let (result, output) = fakes.init(&[], &Fixed);
    let pin = result.unwrap();
    assert_eq!(
        fakes.calls(),
        [
            fakes.call(&[
                "init",
                "--owner",
                "octo",
                "--trailer",
                "Ada Lovelace <ada@example.com>",
                "--gates",
                "just check"
            ]),
            fakes.call(&["init", "--workspace"]),
            fakes.call(&["skill", "install"]),
        ]
    );
    assert_eq!(
        output,
        "running fab init --owner octo --trailer 'Ada Lovelace <ada@example.com>' --gates 'just check'\n\
         running arcl init --workspace\n\
         running arcl skill install\n\
         wrote .github/sltk.toml\n"
    );
    let text = std::fs::read_to_string(fakes.root.join(pin::PATH)).unwrap();
    assert!(text.contains("\nversion = \"1.2.3\"\n"), "{text}");
    assert!(
        text.contains(&format!("\nsha256-aarch64-apple-darwin = \"{DARWIN}\"\n")),
        "{text}"
    );
    assert!(
        text.contains(&format!("\nsha256-x86_64-unknown-linux-gnu = \"{LINUX}\"\n")),
        "{text}"
    );
    assert!(text.ends_with("\noptional = []\n"), "{text}");
    assert_eq!(Pin::read(&fakes.root, &Train::embedded().unwrap()).unwrap(), pin);
}

#[test]
fn a_failing_command_stops_init_before_the_pin() {
    let fakes = Fakes::new(true);
    let (result, _) = fakes.init(&[], &Fixed);
    let err = result.unwrap_err().to_string();
    assert!(
        err.starts_with("sltk: arcl skill install failed with exit status: 7"),
        "{err}"
    );
    assert_eq!(fakes.calls().len(), 3);
    assert!(!fakes.root.join(".github").exists());
}

#[test]
fn an_existing_pin_is_refused_and_kept() {
    let fakes = Fakes::new(false);
    std::fs::create_dir_all(fakes.root.join(".github")).unwrap();
    std::fs::write(fakes.root.join(pin::PATH), "kept\n").unwrap();
    let (result, _) = fakes.init(&[], &Fixed);
    assert!(matches!(result, Err(SetupError::PinExists { .. })));
    assert!(fakes.calls().is_empty());
    assert_eq!(std::fs::read_to_string(fakes.root.join(pin::PATH)).unwrap(), "kept\n");
}

#[test]
fn a_missing_release_hash_stops_init_before_any_command() {
    let fakes = Fakes::new(false);
    let (result, _) = fakes.init(&[], &Missing);
    assert_eq!(
        result.unwrap_err().to_string(),
        "sltk: could not read the sha256 of the sltk 1.2.3 archive for aarch64-apple-darwin: 404 Not Found"
    );
    assert!(fakes.calls().is_empty());
}

#[test]
fn a_with_name_the_train_lacks_stops_init_before_any_command() {
    let fakes = Fakes::new(false);
    let (result, _) = fakes.init(&["nope".to_owned()], &Fixed);
    assert_eq!(
        result.unwrap_err().to_string(),
        "sltk: .github/sltk.toml: the train has no tool named nope"
    );
    assert!(fakes.calls().is_empty());
}
