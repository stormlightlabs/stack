//! Runs the built `sltk init` in a temporary git repository, with fake `fab`
//! and `arcl` first on `PATH` that record any call. None of these tests
//! reaches the network.

#![cfg(unix)]

use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};

/// Makes a git repository and a bin directory of fake `fab` and `arcl` under
/// `dir`, runs `sltk init` in the repository with `args` and the fakes first
/// on `PATH`, and returns its output. Downloads go to a proxy that refuses
/// connections.
fn sltk_init(dir: &Path, args: &[&str]) -> Output {
    sltk_init_in(dir, args, |_| {})
}

/// [`sltk_init`], with `prepare` run on the new repository first.
fn sltk_init_in(dir: &Path, args: &[&str], prepare: impl FnOnce(&Path)) -> Output {
    let (repo, bin) = (dir.join("repo"), dir.join("bin"));
    std::fs::create_dir_all(&bin).unwrap();
    let status = Command::new("git").args(["init", "-q"]).arg(&repo).status().unwrap();
    assert!(status.success());
    prepare(&repo);
    for tool in ["fab", "arcl"] {
        let script = bin.join(tool);
        std::fs::write(
            &script,
            format!("#!/bin/sh\necho {tool} >> '{}'\n", dir.join("calls").display()),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    // A port that was free a moment ago, so nothing listens on it.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let proxy = format!("http://127.0.0.1:{port}");
    Command::new(env!("CARGO_BIN_EXE_sltk"))
        .arg("init")
        .args(args)
        .current_dir(&repo)
        .env("PATH", path)
        .env("HTTPS_PROXY", &proxy)
        .env("https_proxy", &proxy)
        .env("ALL_PROXY", &proxy)
        .env("NO_PROXY", "")
        .env("no_proxy", "")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// Off a terminal, a missing setting is an error naming the flag, and nothing
/// runs.
#[test]
fn off_a_terminal_a_missing_flag_is_named_and_nothing_runs() {
    let dir = tempfile::tempdir().unwrap();
    let output = sltk_init(dir.path(), &["--owner", "octo", "--gates", "just check"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "sltk: init needs --trailer; sltk asks for missing settings only on a terminal\n"
    );
    assert!(!dir.path().join("calls").exists());
    assert!(!dir.path().join("repo/.github").exists());
}

/// With every flag but no release hashes to read, `sltk init` exits 1 naming
/// the missing release, before any setup command runs.
#[test]
fn without_release_hashes_nothing_runs_and_no_pin_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let args = [
        "--owner",
        "octo",
        "--trailer",
        "Ada <ada@example.com>",
        "--gates",
        "just check",
    ];
    let output = sltk_init(dir.path(), &args);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.starts_with(&format!(
            "sltk: could not read the sha256 of the sltk {} archive for aarch64-apple-darwin: https://github.com/stormlightlabs/stack/releases/download/v",
            env!("CARGO_PKG_VERSION")
        )),
        "{stderr}"
    );
    assert!(stderr.contains("publishes none before 0.1.0"), "{stderr}");
    assert!(!dir.path().join("calls").exists());
    assert!(!dir.path().join("repo/.github").exists());
}

/// A repository with fab's settings and no pin is adopted, so no flag is
/// needed. Without release hashes the plan stops before any command or write.
#[test]
fn a_repository_set_up_by_hand_is_adopted_without_flags() {
    let dir = tempfile::tempdir().unwrap();
    let settings = "owner = \"octo\"\ntrailer = \"Ada <ada@example.com>\"\ngates = \"make\"\n";
    let output = sltk_init_in(dir.path(), &[], |repo| {
        std::fs::create_dir_all(repo.join(".github")).unwrap();
        std::fs::write(repo.join(".github/fabrial.toml"), settings).unwrap();
    });
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.starts_with("sltk: could not read the sha256 of the sltk "),
        "{stderr}"
    );
    assert!(!dir.path().join("calls").exists());
    let entries: Vec<_> = std::fs::read_dir(dir.path().join("repo/.github")).unwrap().collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("repo/.github/fabrial.toml")).unwrap(),
        settings
    );
}
