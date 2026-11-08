//! Runs the built `sltk install` on the machine the tests run on. None of
//! these tests reaches the network.

use std::net::TcpListener;
use std::process::{Command, Output};

use sltk_core::platform::Host;

fn sltk(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sltk")).args(args).output().unwrap()
}

#[test]
fn with_an_essential_tool_exits_1() {
    let output = sltk(&["install", "--yes", "--with", "fab"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "sltk: fab is always installed, so it needs no --with\n"
    );
}

#[test]
fn with_a_tool_the_train_lacks_exits_1() {
    let output = sltk(&["install", "--yes", "--with", "nope"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "sltk: the train has no tool named nope\n"
    );
}

/// Runs `sltk install` with `args` and a proxy that refuses connections, so
/// any download fails. Returns the output and the CARGO_HOME it used.
fn install_through_a_refused_proxy(args: &[&str]) -> (Output, tempfile::TempDir) {
    // A port that was free a moment ago, so nothing listens on it.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let proxy = format!("http://127.0.0.1:{port}");
    let cargo_home = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sltk"))
        .args(args)
        .env("CARGO_HOME", cargo_home.path())
        .env("HTTPS_PROXY", &proxy)
        .env("https_proxy", &proxy)
        .env("ALL_PROXY", &proxy)
        .env("NO_PROXY", "")
        .env("no_proxy", "")
        .output()
        .unwrap();
    (output, cargo_home)
}

/// With `--yes` and a proxy that refuses connections, the download fails:
/// `sltk install` exits 1 naming the tool, and creates no bin directory.
#[test]
fn a_failed_download_exits_1_and_writes_nothing() {
    if Host::detect().platform().is_none() {
        return;
    }
    let (output, cargo_home) = install_through_a_refused_proxy(&["install", "--yes"]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.starts_with("sltk: could not download fab from https://"),
        "{stderr}"
    );
    assert!(!cargo_home.path().join("bin").exists());
}

/// Off a terminal (here stdout is a pipe and stdin is closed), `sltk install`
/// without `--yes` installs without asking: it gets as far as the download,
/// which the refused proxy fails.
#[test]
fn off_a_terminal_install_goes_ahead_without_asking() {
    if Host::detect().platform().is_none() {
        return;
    }
    let (output, cargo_home) = install_through_a_refused_proxy(&["install"]);
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("sltk plan for "), "{stdout}");
    assert!(!stdout.contains("Install these?"), "{stdout}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.starts_with("sltk: could not download fab from https://"),
        "{stderr}"
    );
    assert!(!cargo_home.path().join("bin").exists());
}
