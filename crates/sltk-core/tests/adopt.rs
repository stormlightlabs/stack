//! Runs adoption against a repository set up by hand, with fake `fab` and
//! `arcl` scripts that record each call, and fixed archive hashes in place
//! of a published release. The repository holds what fab and a hand-copied
//! hook leave: fab's settings, an AGENTS.md with an unmarked Tasks section,
//! a hook that installs arcl itself, and a SessionStart entry without quotes.

#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use sltk_core::adopt::{self, Adopt, Adoption};
use sltk_core::glue;
use sltk_core::pin::{self, Pin};
use sltk_core::setup::{FAB_SETTINGS, Flag, HashSource, SetupError};
use sltk_core::train::{Platform, Train};

const DARWIN: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
const LINUX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// fab's settings as fab 0.9.1 writes them, with a list and no `[tasks]`.
const FAB_TOML: &str = "# fabrial's settings for this repository.\n\
owner = \"octo\"\n\
app = \"octo-bot\"\n\
secret_prefix = \"OCTO\"\n\
trailer = \"Ada Lovelace <ada@example.com>\"\n\
gates = \"just check\"\n\
planning = [\".arcl/workspace/\"]\n\
version = \"0.9.1\"\n";

/// fab's settings as fab 0.8.0 wrote them, naming no App.
const OLD_FAB_TOML: &str = "owner = \"octo\"\n\
trailer = \"Ada Lovelace <ada@example.com>\"\n\
gates = \"just check\"\n\
version = \"0.8.0\"\n";

/// The part of AGENTS.md before the Tasks section.
const BEFORE: &str = "# AGENTS.md\n\n## Gates\n\n```sh\njust check\n```\n\n";

/// A Tasks section copied by hand, with a pinned arcl install line and a
/// `##` line inside a code fence that must not end the section.
const OLD_TASKS: &str = "## Tasks\n\nTasks live in arcl. Install arcl 0.3.0:\n\n\
```sh\n## not a heading\ncurl -LsSf https://example.test/arcl-installer.sh | sh\n```\n\n\
### Steps\n\n- Run `arcl init` first.\n\n";

/// The part of AGENTS.md after the Tasks section, which fab writes.
const AFTER: &str = "## Branches and pull requests\n\nThis repository uses fabrial.\n";

/// A hook copied by hand that installs arcl itself.
const OLD_HOOK: &str = "#!/usr/bin/env bash\n# Prepares a Claude Code cloud session.\nset -euo pipefail\n\n\
version=0.3.0\n\nif [ \"${CLAUDE_CODE_REMOTE:-}\" != \"true\" ]; then\n  exit 0\nfi\n";

/// The SessionStart entry a hand-copied hook comes with.
const OLD_SETTINGS: &str = "{\n  \"hooks\": {\n    \"SessionStart\": [\n      {\n        \"hooks\": [\n          \
{\n            \"type\": \"command\",\n            \
\"command\": \"$CLAUDE_PROJECT_DIR/.claude/hooks/session-start.sh\"\n          }\n        ]\n      }\n    ]\n  }\n}\n";

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

/// A fake tool set and a repository set up by hand.
struct Fakes {
    _dir: tempfile::TempDir,
    bin: PathBuf,
    calls: PathBuf,
    root: PathBuf,
}

impl Fakes {
    /// Makes the fakes. `arcl skill install --check` exits `check`, and
    /// `fab update` exits `update`.
    fn new(check: i32, update: i32) -> Fakes {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&bin).unwrap();
        let status = std::process::Command::new("git")
            .args(["init", "-q"])
            .arg(&root)
            .status()
            .unwrap();
        assert!(status.success());
        let calls = dir.path().join("calls");
        let record = format!(
            "#!/bin/sh\n{{ printf '%s' \"$(pwd -P)\"; for arg; do printf '\\t%s' \"$arg\"; done; echo; }} >> '{}'\n",
            calls.display()
        );
        write_script(
            &bin.join("fab"),
            &format!("{record}[ \"$1\" = update ] && exit {update}\nexit 0\n"),
        );
        write_script(
            &bin.join("arcl"),
            &format!("{record}[ \"$1 $2 $3 $4\" = 'skill install --check ' ] && exit {check}\nexit 0\n"),
        );
        std::fs::create_dir_all(root.join(".github")).unwrap();
        std::fs::create_dir_all(root.join(".claude/hooks")).unwrap();
        std::fs::create_dir_all(root.join(".arcl/workspace")).unwrap();
        std::fs::write(root.join(FAB_SETTINGS), FAB_TOML).unwrap();
        std::fs::write(root.join(glue::AGENTS), format!("{BEFORE}{OLD_TASKS}{AFTER}")).unwrap();
        write_script(&root.join(glue::HOOK), OLD_HOOK);
        std::fs::write(root.join(glue::SETTINGS), OLD_SETTINGS).unwrap();
        Fakes { _dir: dir, bin, calls, root }
    }

    /// Runs `f` with an [`Adopt`] for the repository, the fakes as `PATH`
    /// and `given` as the flags.
    fn with<T>(&self, given: &[(Flag, String)], f: impl FnOnce(&Adopt) -> T) -> T {
        let path = OsString::from(&self.bin);
        let adopt = Adopt { root: &self.root, given, with: &[], hashes: &Fixed, path: Some(&path) };
        f(&adopt)
    }

    /// Plans the adoption with no flags.
    fn plan(&self) -> Result<Adoption, SetupError> {
        self.with(&[], |adopt| adopt::plan(&train(), "1.2.3", adopt))
    }

    /// Plans and applies the adoption with no flags, and returns the result
    /// and what it wrote.
    fn adopt(&self) -> (Result<Pin, SetupError>, String) {
        self.with(&[], |adopt| {
            let adoption = adopt::plan(&train(), "1.2.3", adopt).unwrap();
            let mut output = Vec::new();
            let result = adopt::apply(&train(), adopt, &adoption, &mut output);
            (result, String::from_utf8(output).unwrap())
        })
    }

    /// Each recorded call's arguments, without the working directory, which
    /// is checked to be the repository root.
    fn calls(&self) -> Vec<String> {
        let root = self.root.canonicalize().unwrap().display().to_string();
        let text = std::fs::read_to_string(&self.calls).unwrap_or_default();
        text.lines()
            .map(|line| {
                let (dir, args) = line.split_once('\t').unwrap();
                assert_eq!(dir, root);
                args.replace('\t', " ")
            })
            .collect()
    }

    /// The text of `relative` under the root.
    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.root.join(relative)).unwrap()
    }
}

fn train() -> Train {
    Train::embedded().unwrap()
}

fn write_script(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Every path under `dir`, `.git` included, with its mode and its bytes or
/// link target, in order.
fn tree(dir: &Path) -> Vec<(PathBuf, u32, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let meta = std::fs::symlink_metadata(&path).unwrap();
        let mode = meta.permissions().mode();
        let bytes = if meta.is_dir() {
            stack.extend(std::fs::read_dir(&path).unwrap().map(|entry| entry.unwrap().path()));
            Vec::new()
        } else if meta.file_type().is_symlink() {
            std::fs::read_link(&path).unwrap().into_os_string().into_encoded_bytes()
        } else {
            std::fs::read(&path).unwrap()
        };
        out.push((path, mode, bytes));
    }
    out.sort();
    out
}

#[test]
fn a_repository_with_fab_settings_and_no_pin_or_record_is_adopted() {
    let fakes = Fakes::new(0, 0);
    assert!(adopt::adopting(&fakes.root).unwrap());
    std::fs::write(fakes.root.join(".git/sltk-init"), "").unwrap();
    assert!(!adopt::adopting(&fakes.root).unwrap(), "a record resumes a fresh init");
    std::fs::remove_file(fakes.root.join(".git/sltk-init")).unwrap();
    std::fs::write(fakes.root.join(pin::PATH), "").unwrap();
    assert!(!adopt::adopting(&fakes.root).unwrap(), "a pinned repository is set up");
    std::fs::remove_file(fakes.root.join(pin::PATH)).unwrap();
    std::fs::remove_file(fakes.root.join(FAB_SETTINGS)).unwrap();
    assert!(
        !adopt::adopting(&fakes.root).unwrap(),
        "a fresh repository gets fab init"
    );
}

#[test]
fn planning_shows_the_diff_and_changes_no_byte() {
    let fakes = Fakes::new(adopt::SKILL_DIFFERS, 0);
    let before = tree(&fakes.root);
    let adoption = fakes.plan().unwrap();
    let text = adoption.describe();
    assert_eq!(tree(&fakes.root), before);
    assert_eq!(fakes.calls(), ["check", "skill install --check"]);

    assert!(
        text.starts_with(
            "sltk init adopts this repository and keeps the settings in .github/fabrial.toml.\n\
             It runs these commands, which write their own files:\n  fab update\n  arcl skill install\n\
             It writes these files:\n--- a/AGENTS.md\n+++ b/AGENTS.md\n"
        ),
        "{text}"
    );
    assert!(text.contains("\n-Tasks live in arcl. Install arcl 0.3.0:\n"), "{text}");
    assert!(
        text.contains(&format!("\n+{}\n ## Tasks\n", glue::TASKS_BEGIN)),
        "{text}"
    );
    assert!(text.contains("\n-## not a heading\n"), "{text}");
    assert!(
        text.contains(&format!("\n+{}\n \n ## Branches", glue::TASKS_END)),
        "{text}"
    );
    assert!(text.contains("\n ## Branches and pull requests\n"), "{text}");
    assert!(text.contains("\n--- a/.claude/hooks/session-start.sh\n+++ b/.claude/hooks/session-start.sh\n"));
    assert!(text.contains("\n-version=0.3.0\n"), "{text}");
    assert!(text.contains("\n+pin=.github/sltk.toml\n"), "{text}");
    assert!(
        text.contains(
            "\n-            \"command\": \"$CLAUDE_PROJECT_DIR/.claude/hooks/session-start.sh\"\n\
             +            \"command\": \"\\\"$CLAUDE_PROJECT_DIR\\\"/.claude/hooks/session-start.sh\"\n"
        ),
        "{text}"
    );
    assert!(text.contains("\n--- /dev/null\n+++ b/.github/sltk.toml\n"), "{text}");
    assert!(text.ends_with("\n+optional = []\n"), "{text}");
    assert!(!text.contains("mode "), "the copied hook is executable already: {text}");
}

#[test]
fn applying_replaces_the_hook_and_tasks_section_then_runs_fab_update_and_the_skill_install() {
    let fakes = Fakes::new(adopt::SKILL_DIFFERS, 0);
    let (result, output) = fakes.adopt();
    let pin = result.unwrap();
    assert_eq!(
        output,
        "wrote AGENTS.md\n\
         wrote .claude/hooks/session-start.sh\n\
         wrote .claude/settings.json\n\
         wrote .github/fabrial.toml\n\
         running fab update\n\
         running arcl skill install\n\
         wrote .github/sltk.toml\n"
    );
    assert_eq!(
        fakes.calls(),
        ["check", "skill install --check", "update", "skill install"]
    );
    assert_eq!(
        fakes.read(glue::AGENTS),
        format!(
            "{BEFORE}{}\n{}{}\n\n{AFTER}",
            glue::TASKS_BEGIN,
            glue::TASKS,
            glue::TASKS_END
        )
    );
    assert_eq!(fakes.read(glue::HOOK), glue::HOOK_SCRIPT);
    let mode = std::fs::metadata(fakes.root.join(glue::HOOK))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755);
    let settings = fakes.read(glue::SETTINGS);
    let quoted = serde_json::to_string(glue::HOOK_COMMAND).unwrap();
    assert_eq!(settings.matches(&quoted).count(), 1, "{settings}");
    assert!(!settings.contains("\"$CLAUDE_PROJECT_DIR/"), "{settings}");
    assert_eq!(fakes.read(FAB_SETTINGS), format!("{FAB_TOML}{}", glue::FAB_TASKS));
    assert_eq!(Pin::read(&fakes.root, &train()).unwrap(), pin);
    assert_eq!(pin.version, "1.2.3");

    // Adopted, the repository is pinned, so init neither adopts nor refreshes it.
    assert!(!adopt::adopting(&fakes.root).unwrap());
    assert!(matches!(fakes.plan(), Err(SetupError::PinExists { .. })));
    assert_eq!(glue::write(&fakes.root).unwrap(), Vec::<&str>::new());
}

#[test]
fn skill_files_that_match_are_not_installed_again() {
    let fakes = Fakes::new(0, 0);
    let adoption = fakes.plan().unwrap();
    assert_eq!(adoption.commands, [vec!["fab".to_owned(), "update".to_owned()]]);
    fakes.adopt().0.unwrap();
    assert_eq!(
        fakes.calls(),
        [
            "check",
            "skill install --check",
            "check",
            "skill install --check",
            "update"
        ]
    );
}

#[test]
fn a_skill_check_that_fails_another_way_stops_before_any_change() {
    let fakes = Fakes::new(7, 0);
    let before = tree(&fakes.root);
    let err = fakes.plan().unwrap_err();
    assert_eq!(
        err.to_string(),
        "sltk: arcl skill install --check failed with exit status: 7; sltk changed nothing"
    );
    assert_eq!(tree(&fakes.root), before);
}

#[test]
fn a_repository_without_arcl_gets_its_workspace() {
    let fakes = Fakes::new(0, 0);
    std::fs::remove_dir_all(fakes.root.join(".arcl")).unwrap();
    let adoption = fakes.plan().unwrap();
    let lines: Vec<String> = adoption.commands.iter().map(|c| c.join(" ")).collect();
    assert_eq!(lines, ["fab update", "arcl init --workspace"]);
}

#[test]
fn flags_that_match_fab_settings_are_accepted_and_others_refused() {
    let fakes = Fakes::new(0, 0);
    let same = [(Flag::Owner, "octo".to_owned()), (Flag::Gates, "just check".to_owned())];
    fakes
        .with(&same, |adopt| adopt::plan(&train(), "1.2.3", adopt))
        .unwrap();
    let other = [
        (Flag::Owner, "hubot".to_owned()),
        (Flag::Gates, "just check".to_owned()),
    ];
    let calls = fakes.calls().len();
    let err = fakes
        .with(&other, |adopt| adopt::plan(&train(), "1.2.3", adopt))
        .unwrap_err()
        .to_string();
    assert!(
        err.ends_with(
            "/.github/fabrial.toml holds other values for --owner. Adoption keeps the settings in this file; \
             leave out --owner, or change the file with fab update first"
        ),
        "{err}"
    );
    assert_eq!(fakes.calls().len(), calls, "a refused plan runs nothing");
}

#[test]
fn a_claude_md_without_agents_md_gets_the_tasks_section() {
    let fakes = Fakes::new(0, 0);
    std::fs::rename(fakes.root.join(glue::AGENTS), fakes.root.join(glue::CLAUDE)).unwrap();
    fakes.adopt().0.unwrap();
    assert!(!fakes.root.join(glue::AGENTS).exists());
    let text = fakes.read(glue::CLAUDE);
    assert!(text.starts_with(&format!("{BEFORE}{}\n", glue::TASKS_BEGIN)), "{text}");
    assert!(text.ends_with(&format!("{}\n\n{AFTER}", glue::TASKS_END)), "{text}");
}

#[test]
fn a_rerun_keeps_the_tasks_section_in_claude_md_after_fab_update_writes_agents_md() {
    // fab update creates AGENTS.md, then the skill install fails.
    let fakes = Fakes::new(4, 0);
    std::fs::rename(fakes.root.join(glue::AGENTS), fakes.root.join(glue::CLAUDE)).unwrap();
    let agents = format!("# AGENTS.md\n\n{AFTER}");
    write_script(
        &fakes.bin.join("fab"),
        &format!("#!/bin/sh\n[ \"$1\" = update ] && printf '%s' '{agents}' > AGENTS.md\nexit 0\n"),
    );
    write_script(
        &fakes.bin.join("arcl"),
        "#!/bin/sh\n[ \"$1 $2 $3\" = 'skill install --check' ] && exit 4\nexit 5\n",
    );
    assert!(fakes.adopt().0.is_err());
    let claude = fakes.read(glue::CLAUDE);
    assert_eq!(fakes.read(glue::AGENTS), agents);

    // The rerun finds the markers in CLAUDE.md and leaves AGENTS.md alone.
    write_script(
        &fakes.bin.join("arcl"),
        "#!/bin/sh\n[ \"$1 $2 $3\" = 'skill install --check' ] && exit 4\nexit 0\n",
    );
    assert_eq!(glue::tasks_file(&fakes.root).unwrap(), glue::CLAUDE);
    let adoption = fakes.plan().unwrap();
    assert!(adoption.glue.is_empty(), "{:?}", adoption.glue);
    fakes.adopt().0.unwrap();
    assert_eq!(fakes.read(glue::CLAUDE), claude);
    assert_eq!(fakes.read(glue::AGENTS), agents);
}

#[test]
fn a_pin_written_after_the_diff_is_refused_as_existing() {
    let fakes = Fakes::new(0, 0);
    fakes.with(&[], |adopt| {
        let adoption = adopt::plan(&train(), "1.2.3", adopt).unwrap();
        std::fs::write(fakes.root.join(pin::PATH), "").unwrap();
        let err = adopt::apply(&train(), adopt, &adoption, &mut Vec::new()).unwrap_err();
        assert!(matches!(err, SetupError::PinExists { .. }), "{err:?}");
    });
}

#[test]
fn a_failed_fab_update_leaves_no_pin_and_a_rerun_finishes() {
    let fakes = Fakes::new(0, 3);
    let (result, _) = fakes.adopt();
    let err = result.unwrap_err().to_string();
    assert_eq!(
        err,
        "sltk: fab update failed with exit status: 3. Ran: none. Not run: none. \
         sltk did not write the pin; rerun sltk init with the same flags to finish"
    );
    assert!(!fakes.root.join(pin::PATH).exists());
    assert!(adopt::adopting(&fakes.root).unwrap());

    write_script(&fakes.bin.join("fab"), "#!/bin/sh\nexit 0\n");
    let adoption = fakes.plan().unwrap();
    assert!(adoption.glue.is_empty(), "{:?}", adoption.glue);
    let (result, output) = fakes.adopt();
    result.unwrap();
    assert_eq!(output, "running fab update\nwrote .github/sltk.toml\n");
}

#[test]
fn a_file_changed_after_the_diff_stops_the_write() {
    let fakes = Fakes::new(0, 0);
    let path = OsString::from(&fakes.bin);
    let adopt = Adopt { root: &fakes.root, given: &[], with: &[], hashes: &Fixed, path: Some(&path) };
    let adoption = adopt::plan(&train(), "1.2.3", &adopt).unwrap();
    std::fs::write(fakes.root.join(glue::HOOK), "#!/bin/sh\necho edited\n").unwrap();
    let before = tree(&fakes.root);
    let err = adopt::apply(&train(), &adopt, &adoption, &mut Vec::new()).unwrap_err();
    assert!(
        matches!(&err, SetupError::Changed { path } if path.ends_with(glue::HOOK)),
        "{err:?}"
    );
    assert_eq!(tree(&fakes.root), before);
}

#[test]
fn two_unmarked_tasks_sections_are_refused() {
    let fakes = Fakes::new(0, 0);
    std::fs::write(fakes.root.join(glue::AGENTS), format!("{OLD_TASKS}{OLD_TASKS}")).unwrap();
    let before = tree(&fakes.root);
    let err = fakes.plan().unwrap_err().to_string();
    assert!(err.contains("AGENTS.md: has 2 ## Tasks headings"), "{err}");
    assert_eq!(tree(&fakes.root), before);
}

#[test]
fn a_non_executable_hook_shows_its_mode_change() {
    let fakes = Fakes::new(0, 0);
    std::fs::set_permissions(fakes.root.join(glue::HOOK), std::fs::Permissions::from_mode(0o644)).unwrap();
    let text = fakes.plan().unwrap().describe();
    assert!(text.contains("\nmode 644 -> 755\n"), "{text}");
}

#[test]
fn a_missing_fab_or_arcl_stops_the_plan_before_any_change() {
    for program in ["fab", "arcl"] {
        let fakes = Fakes::new(0, 0);
        std::fs::remove_file(fakes.bin.join(program)).unwrap();
        let before = tree(&fakes.root);
        let err = fakes.plan().unwrap_err();
        assert!(
            matches!(&err, SetupError::NoProgram { program: p } if p == program),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            format!(
                "sltk: {program} is not in an absolute PATH directory, and sltk init runs it to adopt this \
                 repository. Install it with sltk install and rerun sltk init; sltk changed nothing"
            )
        );
        assert_eq!(tree(&fakes.root), before);
        assert!(fakes.calls().is_empty(), "{:?}", fakes.calls());
    }
}

#[test]
fn the_plan_lists_the_files_fab_check_reports_for_fab_update_to_rewrite() {
    let fakes = Fakes::new(0, 0);
    write_script(
        &fakes.bin.join("fab"),
        "#!/bin/sh\n[ \"$1\" = check ] || exit 9\n\
         echo 'differs: .github/workflows/branch-pr.yml'\necho 'missing: .claude/agents/reviser.md'\nexit 1\n",
    );
    let before = tree(&fakes.root);
    let text = fakes.plan().unwrap().describe();
    assert_eq!(tree(&fakes.root), before);
    assert!(
        text.contains(
            "  fab update\n\
             fab update will rewrite or remove these files, which fab check reports as drifted:\n  \
             .github/workflows/branch-pr.yml\n  .claude/agents/reviser.md\nIt writes these files:\n"
        ),
        "{text}"
    );

    // No drift lists nothing, and a failure stops the plan.
    write_script(
        &fakes.bin.join("fab"),
        "#!/bin/sh\necho 'fabrial 0.9.1 is installed with no drift.'\n",
    );
    let text = fakes.plan().unwrap().describe();
    assert!(!text.contains("fab check"), "{text}");
    write_script(&fakes.bin.join("fab"), "#!/bin/sh\nexit 2\n");
    let err = fakes.plan().unwrap_err().to_string();
    assert_eq!(err, "sltk: fab check failed with exit status: 2; sltk changed nothing");
    assert_eq!(tree(&fakes.root), before);
}

#[test]
fn an_agents_md_linked_to_claude_md_has_the_section_written_to_claude_md() {
    let fakes = Fakes::new(0, 0);
    std::fs::rename(fakes.root.join(glue::AGENTS), fakes.root.join(glue::CLAUDE)).unwrap();
    std::os::unix::fs::symlink(glue::CLAUDE, fakes.root.join(glue::AGENTS)).unwrap();
    fakes.adopt().0.unwrap();
    let link = std::fs::read_link(fakes.root.join(glue::AGENTS)).unwrap();
    assert_eq!(link, Path::new(glue::CLAUDE));
    let text = fakes.read(glue::CLAUDE);
    assert!(text.starts_with(&format!("{BEFORE}{}\n", glue::TASKS_BEGIN)), "{text}");
    assert!(text.ends_with(&format!("{}\n\n{AFTER}", glue::TASKS_END)), "{text}");
}

#[test]
fn a_tasks_heading_only_in_a_code_block_leaves_the_file_whole() {
    let fakes = Fakes::new(0, 0);
    let text = format!("{BEFORE}Example:\n\n~~~md\n## Tasks\n```\nnot closed by that\n~~~\n\n{AFTER}");
    std::fs::write(fakes.root.join(glue::AGENTS), &text).unwrap();
    fakes.adopt().0.unwrap();
    assert_eq!(
        fakes.read(glue::AGENTS),
        format!("{text}\n{}\n{}{}\n", glue::TASKS_BEGIN, glue::TASKS, glue::TASKS_END)
    );
}

#[test]
fn settings_without_an_app_need_app_and_fab_update_gets_it() {
    let fakes = Fakes::new(0, 0);
    std::fs::write(fakes.root.join(FAB_SETTINGS), OLD_FAB_TOML).unwrap();
    assert!(adopt::needs_app(&fakes.root).unwrap());
    let before = tree(&fakes.root);
    let err = fakes.plan().unwrap_err().to_string();
    assert!(
        err.ends_with(
            "/.github/fabrial.toml names no app, and fab update needs one. Pass --app with the slug of the \
             repository's GitHub App"
        ),
        "{err}"
    );
    assert_eq!(tree(&fakes.root), before);
    assert!(fakes.calls().is_empty());

    // The values the file lacks go to fab update; the ones it has must match.
    let given = [
        (Flag::App, "octo-bot".to_owned()),
        (Flag::SecretPrefix, "OCTO".to_owned()),
    ];
    let adoption = fakes
        .with(&given, |adopt| adopt::plan(&train(), "1.2.3", adopt))
        .unwrap();
    let lines: Vec<String> = adoption.commands.iter().map(|c| c.join(" ")).collect();
    assert_eq!(lines, ["fab update --app octo-bot --secret-prefix OCTO"]);
    let given = [(Flag::App, "octo-bot".to_owned())];
    let adoption = fakes
        .with(&given, |adopt| adopt::plan(&train(), "1.2.3", adopt))
        .unwrap();
    assert_eq!(
        adoption.commands,
        [["fab", "update", "--app", "octo-bot"].map(String::from)]
    );

    std::fs::write(fakes.root.join(FAB_SETTINGS), FAB_TOML).unwrap();
    assert!(!adopt::needs_app(&fakes.root).unwrap());
    let adoption = fakes
        .with(&given, |adopt| adopt::plan(&train(), "1.2.3", adopt))
        .unwrap();
    assert_eq!(adoption.commands, [["fab", "update"].map(String::from)]);
    let other = [(Flag::App, "hubot".to_owned()), (Flag::SecretPrefix, "OCTO".to_owned())];
    let err = fakes
        .with(&other, |adopt| adopt::plan(&train(), "1.2.3", adopt))
        .unwrap_err()
        .to_string();
    assert!(err.contains("holds other values for --app. "), "{err}");
}
