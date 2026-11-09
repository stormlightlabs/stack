//! Runs the real fab the train pins against the `[tasks]` table sltk appends
//! to `.github/fabrial.toml`. If fab renders the table another way, every
//! repository sltk adopts shows `differs: .github/fabrial.toml`.
//!
//! The test is ignored by default, since it needs that fab first on `PATH`.
//! Run it whenever the train changes fab's version:
//!
//! ```sh
//! cargo test -p sltk-core --test pinned_fab -- --ignored
//! ```

#![cfg(unix)]

use std::path::Path;
use std::process::{Command, Output};

use sltk_core::doctor::parse_version;
use sltk_core::glue::{self, FAB_TASKS};
use sltk_core::setup::FAB_SETTINGS;
use sltk_core::train::Train;

/// Runs `program` with `args` in `dir`, and panics if it cannot start.
fn run(dir: &Path, program: &str, args: &[&str]) -> Output {
    Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|err| panic!("{program} {}: {err}", args.join(" ")))
}

/// Panics with the command's output unless it exited 0.
fn assert_ok(output: &Output, what: &str) {
    assert!(
        output.status.success(),
        "{what} exited {}:\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "needs the train's fab first on PATH"]
fn pinned_fab_accepts_the_tasks_table() {
    let train = Train::embedded().unwrap();
    let pinned = &train.tools.iter().find(|tool| tool.name == "fab").unwrap().version;
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path();

    let version = run(root, "fab", &["--version"]);
    let version = String::from_utf8_lossy(&version.stdout);
    assert_eq!(
        parse_version(&version, "fab"),
        Some(pinned.as_str()),
        "the fab on PATH is not the train's"
    );

    assert_ok(&run(root, "git", &["init", "-q"]), "git init");
    let init = run(
        root,
        "fab",
        &[
            "init",
            "--owner",
            "o",
            "--app",
            "x",
            "--trailer",
            "O <o@example.com>",
            "--gates",
            "true",
        ],
    );
    assert_ok(&init, "fab init");

    let settings = root.join(FAB_SETTINGS);
    let rendered = std::fs::read_to_string(&settings).unwrap();
    let with_tasks = glue::fab_tasks(Some(&rendered))
        .unwrap()
        .expect("fab init wrote a [tasks] table");
    assert!(with_tasks.ends_with(FAB_TASKS));
    std::fs::write(&settings, &with_tasks).unwrap();

    assert_ok(&run(root, "fab", &["check"]), "fab check");
    assert_ok(&run(root, "fab", &["update"]), "fab update");
    assert_eq!(
        std::fs::read_to_string(&settings).unwrap(),
        with_tasks,
        "fab update changed the table"
    );
}
