//! Runs the built `sltk check` on a temporary repository set up as `sltk
//! init` leaves it, with fake `fab` and `arcl` in a temporary CARGO_HOME
//! that is also the whole PATH.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use sltk_core::glue;
use sltk_core::pin::{Pin, TARGETS};
use sltk_core::setup::FAB_SETTINGS;
use sltk_core::train::Train;

/// A repository with a pin of this release, fab's settings and the glue, and
/// a bin directory whose `fab` and `arcl` report the train's versions. `fab
/// check` exits `fab_exit`, printing `differs: .github/CODEOWNERS` when that
/// is 1.
fn setup(fab_exit: i32) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let (repo, cargo_home) = (dir.path().join("repo"), dir.path().join("cargo"));
    let bin = cargo_home.join("bin");
    std::fs::create_dir_all(repo.join(".github")).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        repo.join(FAB_SETTINGS),
        "owner = \"octo\"\napp = \"octo-bot\"\ntrailer = \"Ada <a@b.c>\"\ngates = \"make\"\nversion = \"0.9.1\"\n",
    )
    .unwrap();
    glue::write(&repo).unwrap();
    let train = Train::embedded().unwrap();
    let sha256 = TARGETS.into_iter().map(|t| (t, "cd".repeat(32))).collect();
    Pin { version: sltk_core::VERSION.to_owned(), sha256, optional: Vec::new() }
        .write(&repo, &train)
        .unwrap();
    for tool in &train.tools {
        let check = match tool.name.as_str() {
            "fab" => format!(
                "[ \"$1\" = check ] && {{ [ {fab_exit} = 1 ] && echo 'differs: .github/CODEOWNERS'; exit {fab_exit}; }}"
            ),
            _ => "[ \"$1\" = skill ] && exit 0".to_owned(),
        };
        let path = bin.join(&tool.binary);
        let text = format!(
            "#!/bin/sh\n[ \"$1\" = --version ] && {{ echo '{} {}'; exit 0; }}\n{check}\nexit 9\n",
            tool.binary, tool.version
        );
        std::fs::write(&path, text).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    (dir, repo, cargo_home)
}

/// Runs `sltk check --dir repo` with stdin closed and the fakes as PATH.
fn sltk_check(repo: &Path, cargo_home: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sltk"))
        .args(["check", "--dir"])
        .arg(repo)
        .env("CARGO_HOME", cargo_home)
        .env("PATH", cargo_home.join("bin"))
        .env_remove("HOMEBREW_PREFIX")
        .stdin(Stdio::null())
        .output()
        .unwrap()
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
fn a_clean_repository_exits_0_and_is_not_changed() {
    let (_dir, repo, cargo_home) = setup(0);
    let before = tree(&repo);
    let output = sltk_check(&repo, &cargo_home);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert_eq!(stdout, format!("sltk check: checked {}: no drift\n", repo.display()));
    assert_eq!(tree(&repo), before);
}

#[test]
fn drift_is_listed_and_exits_1_without_changing_anything() {
    let (_dir, repo, cargo_home) = setup(1);
    std::fs::write(repo.join(glue::HOOK), "#!/bin/sh\nversion=0.3.0\n").unwrap();
    let before = tree(&repo);
    let output = sltk_check(&repo, &cargo_home);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert_eq!(
        stdout,
        format!(
            "fab check: differs: .github/CODEOWNERS\n\
             .claude/hooks/session-start.sh: differs from the hook sltk init writes\n\
             sltk check: checked {}: 2 differences\n",
            repo.display()
        )
    );
    assert_eq!(tree(&repo), before);
}

#[test]
fn a_check_that_cannot_run_exits_2() {
    let (_dir, repo, cargo_home) = setup(0);
    std::fs::remove_file(cargo_home.join("bin/arcl")).unwrap();
    let output = sltk_check(&repo, &cargo_home);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(2), "{stdout}");
    assert!(
        stdout.contains(
            "could not run arcl skill install --check: no executable file named arcl in an absolute PATH directory\n"
        ),
        "{stdout}"
    );
    assert!(stdout.ends_with("1 difference; 1 check did not finish\n"), "{stdout}");
}
