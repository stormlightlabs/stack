//! Runs the built `sltk doctor` against fake tools in a temporary CARGO_HOME,
//! with a PATH the test builds.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

use sltk_core::train::{Tier, Train};

/// Writes a fake of each essential tool into `bin`, printing the train's
/// version.
fn fake_essentials(bin: &Path) {
    for tool in Train::embedded()
        .unwrap()
        .tools
        .iter()
        .filter(|t| t.tier == Tier::Essential)
    {
        let path = bin.join(&tool.binary);
        std::fs::write(&path, format!("#!/bin/sh\necho '{} {}+abc'\n", tool.name, tool.version)).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// Runs `sltk doctor` in `cwd` with `cargo_home` and a PATH of `path`.
fn doctor(cwd: &Path, cargo_home: &Path, path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sltk"))
        .arg("doctor")
        .current_dir(cwd)
        .env("CARGO_HOME", cargo_home)
        .env("PATH", path)
        .env_remove("HOMEBREW_PREFIX")
        .output()
        .unwrap()
}

/// A temporary repository root with a `.git`, so doctor looks no higher for
/// a pin, and a CARGO_HOME with its bin directory.
fn setup() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let cargo_home = root.path().join("cargo");
    std::fs::create_dir_all(cargo_home.join("bin")).unwrap();
    (root, repo, cargo_home)
}

#[test]
fn a_machine_on_the_train_exits_0() {
    let (_root, repo, cargo_home) = setup();
    let bin = cargo_home.join("bin");
    fake_essentials(&bin);
    let output = doctor(&repo, &cargo_home, &bin);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert_eq!(
        stdout,
        format!("sltk doctor: checked fab, arcl in {}: no problems\n", bin.display())
    );
}

#[test]
fn a_missing_tool_exits_1() {
    let (_root, repo, cargo_home) = setup();
    let bin = cargo_home.join("bin");
    fake_essentials(&bin);
    std::fs::remove_file(bin.join("arcl")).unwrap();
    let output = doctor(&repo, &cargo_home, &bin);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert_eq!(
        stdout,
        format!(
            "arcl: {} is not installed\nsltk doctor: checked fab, arcl in {}: 1 problem\n",
            bin.join("arcl").display(),
            bin.display()
        )
    );
}

#[test]
fn an_unreadable_pin_exits_2() {
    let (_root, repo, cargo_home) = setup();
    let bin = cargo_home.join("bin");
    fake_essentials(&bin);
    std::fs::create_dir_all(repo.join(".github")).unwrap();
    std::fs::write(repo.join(".github/sltk.toml"), "not a pin\n").unwrap();
    let output = doctor(&repo, &cargo_home, &bin);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.starts_with("sltk: .github/sltk.toml: "), "{stderr}");
}
