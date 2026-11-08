//! Runs the built `sltk install` on the machine the tests run on. None of
//! these tests reaches the network.

use std::net::TcpListener;
use std::process::{Command, Output};

use sltk_core::platform::Host;

fn sltk(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sltk")).args(args).output().unwrap()
}

/// Without `--yes`, `sltk install` prints the plan, installs nothing and exits
/// 3 on a supported machine, and exits 1 on any other. It never exits 0, so
/// `sltk install && fab` cannot go on without fab.
#[test]
fn install_without_yes_installs_nothing() {
    let output = sltk(&["install"]);
    if Host::detect().platform().is_some() {
        assert_eq!(output.status.code(), Some(3));
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            stdout.lines().last(),
            Some("sltk: nothing was installed; run sltk install --yes to install")
        );
    } else {
        assert_eq!(output.status.code(), Some(1));
    }
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

/// With `--yes` and a proxy that refuses connections, the download fails:
/// `sltk install` exits 1 naming the tool, and creates no bin directory.
#[test]
fn a_failed_download_exits_1_and_writes_nothing() {
    if Host::detect().platform().is_none() {
        return;
    }
    // A port that was free a moment ago, so nothing listens on it.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let proxy = format!("http://127.0.0.1:{port}");
    let cargo_home = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sltk"))
        .args(["install", "--yes"])
        .env("CARGO_HOME", cargo_home.path())
        .env("HTTPS_PROXY", &proxy)
        .env("https_proxy", &proxy)
        .env("ALL_PROXY", &proxy)
        .env("NO_PROXY", "")
        .env("no_proxy", "")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.starts_with("sltk: could not download fab from https://"),
        "{stderr}"
    );
    assert!(!cargo_home.path().join("bin").exists());
}
