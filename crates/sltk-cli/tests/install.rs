//! Runs the built `sltk install` on the machine the tests run on.

use std::process::Command;

use sltk_core::platform::Host;

/// `sltk install` exits 3 after a plan for a supported machine, ending with a
/// line saying nothing was installed, and 1 on any other machine. It never
/// exits 0, so `sltk install && fab` cannot go on without fab.
#[test]
fn install_never_exits_zero() {
    let output = Command::new(env!("CARGO_BIN_EXE_sltk"))
        .arg("install")
        .output()
        .unwrap();
    if Host::detect().platform().is_some() {
        assert_eq!(output.status.code(), Some(3));
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            stdout.lines().last(),
            Some("sltk: nothing was installed, because downloading is not built yet")
        );
    } else {
        assert_eq!(output.status.code(), Some(1));
    }
}
