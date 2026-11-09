//! Runs `init` against fake `fab` and `arcl` scripts that record each call,
//! with fixed archive hashes in place of a published release. The fake `fab`
//! writes `.github/fabrial.toml` and refuses to run twice, as fab 0.8.0 does.
//! The repository is a real git repository, where `init` records its run.

#![cfg(unix)]

use std::ffi::{OsStr, OsString};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use sltk_core::glue;
use sltk_core::pin::{self, Pin};
use sltk_core::setup::{FAB_SETTINGS, FabSettings, HashSource, Init, SetupError, init};
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

/// A source that must not be read: `init` refuses before it gets here.
struct Unread;

impl HashSource for Unread {
    fn sha256(&self, _: Platform) -> Result<String, String> {
        panic!("init read a release hash before refusing the repository");
    }
}

/// How the fake `arcl skill install` exits.
#[derive(Clone, Copy, PartialEq)]
enum Skill {
    /// Exits 0.
    Works,
    /// Exits 7 on every call.
    Fails,
    /// Exits 7 on the first call and 0 after.
    FailsOnce,
}

/// A fake tool and repository: a bin directory holding `fab` and `arcl`
/// scripts that append their working directory and arguments, tab-separated,
/// to `calls`, and an empty git repository.
struct Fakes {
    _dir: tempfile::TempDir,
    bin: PathBuf,
    calls: PathBuf,
    root: PathBuf,
}

impl Fakes {
    /// Makes the fakes, with `arcl skill` exiting as `skill` says.
    fn new(skill: Skill) -> Fakes {
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
        // Builtins only: PATH holds nothing but the fakes.
        let record = format!(
            "#!/bin/sh\n{{ printf '%s' \"$(pwd -P)\"; for arg; do printf '\\t%s' \"$arg\"; done; echo; }} >> '{}'\n",
            calls.display()
        );
        // `fab init --owner O --trailer T --gates G` writes the settings in
        // fab's form, and exits 2 when they are already there. `mkdir` is not
        // a builtin, so it is named by its path.
        let fab = format!(
            "{record}[ -e {FAB_SETTINGS} ] && exit 2\n\
             [ -d .github ] || /bin/mkdir .github\n\
             printf 'owner = \"%s\"\\ntrailer = \"%s\"\\ngates = \"%s\"\\nversion = \"0.8.0\"\\n' \"$3\" \"$5\" \"$7\" > {FAB_SETTINGS}\n"
        );
        let marker = dir.path().join("skill-failed");
        let fail = match skill {
            Skill::Works => String::new(),
            Skill::Fails => "[ \"$1\" = skill ] && exit 7\n".to_owned(),
            Skill::FailsOnce => format!(
                "if [ \"$1\" = skill ] && ! [ -e '{0}' ]; then : > '{0}'; exit 7; fi\n",
                marker.display()
            ),
        };
        write_script(&bin.join("fab"), &fab);
        write_script(&bin.join("arcl"), &format!("{record}{fail}exit 0\n"));
        Fakes { _dir: dir, bin, calls, root }
    }

    /// Runs `init` with the fakes first on `PATH`.
    fn init(&self, with: &[String], hashes: &dyn HashSource) -> (Result<Pin, SetupError>, String) {
        self.init_with(&settings(), with, hashes)
    }

    /// Runs `init` with `settings` and the fakes first on `PATH`.
    fn init_with(
        &self, settings: &FabSettings, with: &[String], hashes: &dyn HashSource,
    ) -> (Result<Pin, SetupError>, String) {
        self.init_on(&OsString::from(&self.bin), settings, with, hashes)
    }

    /// Runs `init` with `settings` and `path` as `PATH`.
    fn init_on(
        &self, path: &OsStr, settings: &FabSettings, with: &[String], hashes: &dyn HashSource,
    ) -> (Result<Pin, SetupError>, String) {
        let train = Train::embedded().unwrap();
        let mut output = Vec::new();
        let result = init(
            &train,
            "1.2.3",
            Init { root: &self.root, settings, with, hashes, path: Some(path) },
            &mut output,
        );
        (result, String::from_utf8(output).unwrap())
    }

    /// Where `init` records its run.
    fn record(&self) -> PathBuf {
        self.root.join(".git/sltk-init")
    }

    /// Writes fab's settings as fab `version` would for [`settings`].
    fn write_fab_settings(&self, version: &str) {
        std::fs::create_dir_all(self.root.join(".github")).unwrap();
        let text = format!(
            "owner = \"octo\"\ntrailer = \"Ada Lovelace <ada@example.com>\"\ngates = \"just check\"\nversion = \"{version}\"\n"
        );
        std::fs::write(self.root.join(FAB_SETTINGS), text).unwrap();
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
    let fakes = Fakes::new(Skill::Works);
    let (result, output) = fakes.init(&[], &Fixed);
    let pin = result.unwrap();
    assert_eq!(
        fakes.calls(),
        [
            fakes.call(&FAB_INIT),
            fakes.call(&["init", "--workspace"]),
            fakes.call(&["skill", "install"]),
        ]
    );
    assert_eq!(
        output,
        "running fab init --owner octo --trailer 'Ada Lovelace <ada@example.com>' --gates 'just check'\n\
         running arcl init --workspace\n\
         running arcl skill install\n\
         wrote AGENTS.md\n\
         wrote .claude/hooks/session-start.sh\n\
         wrote .claude/settings.json\n\
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
    assert!(!fakes.record().exists());
}

/// The `fab init` call the fakes record for [`settings`].
const FAB_INIT: [&str; 7] = [
    "init",
    "--owner",
    "octo",
    "--trailer",
    "Ada Lovelace <ada@example.com>",
    "--gates",
    "just check",
];

#[test]
fn a_failing_command_names_what_ran_and_stops_before_the_pin() {
    let fakes = Fakes::new(Skill::Fails);
    let (result, _) = fakes.init(&[], &Fixed);
    let err = result.unwrap_err().to_string();
    assert_eq!(
        err,
        "sltk: arcl skill install failed with exit status: 7. \
         Ran: fab init --owner octo --trailer 'Ada Lovelace <ada@example.com>' --gates 'just check'; \
         arcl init --workspace. Not run: none. \
         sltk did not write the pin; rerun sltk init with the same flags to finish"
    );
    assert_eq!(fakes.calls().len(), 3);
    assert!(!fakes.root.join(pin::PATH).exists());
}

#[test]
fn a_command_that_cannot_start_names_the_commands_not_run() {
    let fakes = Fakes::new(Skill::Works);
    std::fs::remove_file(fakes.bin.join("arcl")).unwrap();
    let (result, _) = fakes.init(&[], &Fixed);
    let err = result.unwrap_err().to_string();
    assert!(
        err.starts_with("sltk: could not run arcl init --workspace in "),
        "{err}"
    );
    assert!(
        err.ends_with(
            ". Ran: fab init --owner octo --trailer 'Ada Lovelace <ada@example.com>' --gates 'just check'. \
             Not run: arcl skill install. \
             sltk did not write the pin; rerun sltk init with the same flags to finish"
        ),
        "{err}"
    );
    assert!(!fakes.root.join(pin::PATH).exists());
}

#[test]
fn a_rerun_after_a_failure_skips_fab_init_and_finishes() {
    let fakes = Fakes::new(Skill::FailsOnce);
    let (result, _) = fakes.init(&[], &Fixed);
    assert!(matches!(result, Err(SetupError::Failed { .. })), "{result:?}");
    assert!(!fakes.root.join(pin::PATH).exists());
    let record = std::fs::read_to_string(fakes.record()).unwrap();
    assert!(
        record.ends_with(
            "owner = \"octo\"\ntrailer = \"Ada Lovelace <ada@example.com>\"\ngates = \"just check\"\nfab = \"0.8.0\"\n"
        ),
        "{record}"
    );

    let (result, output) = fakes.init(&[], &Fixed);
    let pin = result.unwrap();
    assert_eq!(
        output,
        "skipping fab init: .github/fabrial.toml already holds these settings\n\
         running arcl init --workspace\n\
         running arcl skill install\n\
         wrote AGENTS.md\n\
         wrote .claude/hooks/session-start.sh\n\
         wrote .claude/settings.json\n\
         wrote .github/sltk.toml\n"
    );
    assert_eq!(
        fakes.calls(),
        [
            fakes.call(&FAB_INIT),
            fakes.call(&["init", "--workspace"]),
            fakes.call(&["skill", "install"]),
            fakes.call(&["init", "--workspace"]),
            fakes.call(&["skill", "install"]),
        ]
    );
    assert_eq!(Pin::read(&fakes.root, &Train::embedded().unwrap()).unwrap(), pin);
    assert!(!fakes.record().exists());
}

#[test]
fn matching_fab_settings_without_a_record_are_refused_before_any_download() {
    let fakes = Fakes::new(Skill::Works);
    fakes.write_fab_settings("0.8.0");
    let (result, output) = fakes.init(&[], &Unread);
    let err = result.unwrap_err().to_string();
    assert!(
        err.ends_with(
            "/.github/fabrial.toml was not written by an earlier sltk init; \
             sltk cannot adopt a repository set up by hand yet \
             (planned in the stack task \"Adopt repositories set up by hand\")"
        ),
        "{err}"
    );
    assert!(output.is_empty());
    assert!(fakes.calls().is_empty());
    assert!(!fakes.record().exists());
}

#[test]
fn fab_settings_from_another_fab_are_refused_even_with_a_record() {
    let fakes = Fakes::new(Skill::FailsOnce);
    let (result, _) = fakes.init(&[], &Fixed);
    assert!(matches!(result, Err(SetupError::Failed { .. })), "{result:?}");
    assert!(fakes.record().exists());
    fakes.write_fab_settings("0.5.0");
    let (result, _) = fakes.init(&[], &Unread);
    assert!(
        matches!(&result, Err(SetupError::FabInstalled { reason, .. })
            if reason == "was written by fab 0.5.0, not by the train's fab 0.8.0"),
        "{result:?}"
    );
    assert_eq!(fakes.calls().len(), 3);
}

#[test]
fn oversize_or_linked_fab_settings_are_refused_unread() {
    let fakes = Fakes::new(Skill::Works);
    std::fs::create_dir_all(fakes.root.join(".github")).unwrap();
    let settings = fakes.root.join(FAB_SETTINGS);
    let mut big = "# padding\n".repeat(64 * 1024 / 10 + 1);
    big.push_str("owner = \"octo\"\n");
    std::fs::write(&settings, big).unwrap();
    let (result, _) = fakes.init(&[], &Unread);
    assert!(
        matches!(&result, Err(SetupError::FabInstalled { reason, .. }) if reason == "is longer than 64 KiB"),
        "{result:?}"
    );
    std::fs::remove_file(&settings).unwrap();
    std::os::unix::fs::symlink("/dev/zero", &settings).unwrap();
    let (result, _) = fakes.init(&[], &Unread);
    assert!(
        matches!(&result, Err(SetupError::FabInstalled { reason, .. })
            if reason == "is a symlink, which sltk does not follow"),
        "{result:?}"
    );
    assert!(fakes.calls().is_empty());
}

#[test]
fn a_program_in_the_repository_is_not_run_for_a_relative_path_entry() {
    for prefix in ["", "."] {
        let fakes = Fakes::new(Skill::Works);
        let planted = fakes.root.join("planted");
        for tool in ["fab", "arcl"] {
            write_script(
                &fakes.root.join(tool),
                &format!("#!/bin/sh\n: > '{}'\n", planted.display()),
            );
        }
        let mut path = OsString::from(prefix);
        path.push(":");
        path.push(&fakes.bin);
        let (result, _) = fakes.init_on(&path, &settings(), &[], &Fixed);
        result.unwrap();
        assert!(!planted.exists(), "PATH={path:?} ran the repository's fab");
        assert_eq!(fakes.calls()[0], fakes.call(&FAB_INIT));
    }
}

#[test]
fn a_program_on_no_absolute_path_entry_is_named() {
    let fakes = Fakes::new(Skill::Works);
    let (result, _) = fakes.init_on(OsStr::new(".:relative"), &settings(), &[], &Fixed);
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains(": no executable file named fab in an absolute PATH directory. Ran: none."),
        "{err}"
    );
    assert!(fakes.calls().is_empty());
}

#[test]
fn fab_settings_with_other_values_are_refused_before_any_download() {
    let fakes = Fakes::new(Skill::Works);
    std::fs::create_dir_all(fakes.root.join(".github")).unwrap();
    let theirs = "owner = \"octo\"\ntrailer = \"Bob <bob@example.com>\"\ngates = \"make\"\nversion = \"0.8.0\"\n";
    std::fs::write(fakes.root.join(FAB_SETTINGS), theirs).unwrap();
    let (result, output) = fakes.init(&[], &Unread);
    let err = result.unwrap_err().to_string();
    assert!(err.starts_with("sltk: "), "{err}");
    assert!(
        err.ends_with(
            "/.github/fabrial.toml holds other values for --trailer, --gates; \
             sltk cannot adopt a repository set up by hand yet \
             (planned in the stack task \"Adopt repositories set up by hand\")"
        ),
        "{err}"
    );
    assert!(output.is_empty());
    assert!(fakes.calls().is_empty());
    assert_eq!(std::fs::read_to_string(fakes.root.join(FAB_SETTINGS)).unwrap(), theirs);
}

#[test]
fn fab_settings_sltk_cannot_read_are_refused_before_any_download() {
    let fakes = Fakes::new(Skill::Works);
    std::fs::create_dir_all(fakes.root.join(".github")).unwrap();
    std::fs::write(fakes.root.join(FAB_SETTINGS), "owner = \"octo\"\n").unwrap();
    let (result, _) = fakes.init(&[], &Unread);
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("/.github/fabrial.toml could not be read as fab's settings: missing field `trailer`;"),
        "{err}"
    );
    assert!(fakes.calls().is_empty());
}

#[test]
fn a_rerun_with_other_flags_is_refused() {
    let fakes = Fakes::new(Skill::FailsOnce);
    let (result, _) = fakes.init(&[], &Fixed);
    assert!(matches!(result, Err(SetupError::Failed { .. })), "{result:?}");
    let other = FabSettings::new(
        "hubot".into(),
        "Ada Lovelace <ada@example.com>".into(),
        "just check".into(),
    )
    .unwrap();
    let (result, _) = fakes.init_with(&other, &[], &Unread);
    assert!(
        matches!(&result, Err(SetupError::FabInstalled { reason, .. }) if reason == "holds other values for --owner"),
        "{result:?}"
    );
    assert_eq!(fakes.calls().len(), 3);
}

#[test]
fn an_existing_pin_is_refused_and_kept() {
    let fakes = Fakes::new(Skill::Works);
    std::fs::create_dir_all(fakes.root.join(".github")).unwrap();
    std::fs::write(fakes.root.join(pin::PATH), "kept\n").unwrap();
    let (result, _) = fakes.init(&[], &Fixed);
    assert!(matches!(result, Err(SetupError::PinExists { .. })));
    assert!(fakes.calls().is_empty());
    assert_eq!(std::fs::read_to_string(fakes.root.join(pin::PATH)).unwrap(), "kept\n");
}

#[test]
fn a_missing_release_hash_stops_init_before_any_command() {
    let fakes = Fakes::new(Skill::Works);
    let (result, _) = fakes.init(&[], &Missing);
    assert_eq!(
        result.unwrap_err().to_string(),
        "sltk: could not read the sha256 of the sltk 1.2.3 archive for aarch64-apple-darwin: 404 Not Found"
    );
    assert!(fakes.calls().is_empty());
}

#[test]
fn a_with_name_the_train_lacks_stops_init_before_any_command() {
    let fakes = Fakes::new(Skill::Works);
    let (result, _) = fakes.init(&["nope".to_owned()], &Fixed);
    assert_eq!(
        result.unwrap_err().to_string(),
        "sltk: .github/sltk.toml: the train has no tool named nope"
    );
    assert!(fakes.calls().is_empty());
}

/// The bytes of each file `init` writes, by path relative to the root.
fn written(root: &Path) -> Vec<(&'static str, Vec<u8>, u32)> {
    [glue::AGENTS, glue::HOOK, glue::SETTINGS, pin::PATH]
        .into_iter()
        .map(|path| {
            let full = root.join(path);
            let mode = std::fs::metadata(&full).unwrap().permissions().mode() & 0o777;
            (path, std::fs::read(&full).unwrap(), mode)
        })
        .collect()
}

#[test]
fn init_writes_the_tasks_section_and_the_hook_and_a_second_run_changes_nothing() {
    let fakes = Fakes::new(Skill::Works);
    let agents = fakes.root.join(glue::AGENTS);
    std::fs::write(&agents, "# AGENTS.md\n\nKeep this.\n").unwrap();
    std::fs::create_dir_all(fakes.root.join(".claude")).unwrap();
    std::fs::write(
        fakes.root.join(glue::SETTINGS),
        "{\n  \"permissions\": {\n    \"allow\": [\"Bash(ls)\"]\n  }\n}\n",
    )
    .unwrap();
    fakes.init(&[], &Fixed).0.unwrap();

    let text = std::fs::read_to_string(&agents).unwrap();
    assert_eq!(
        text,
        format!(
            "# AGENTS.md\n\nKeep this.\n\n{}\n{}{}\n",
            glue::TASKS_BEGIN,
            glue::TASKS,
            glue::TASKS_END
        )
    );
    let hook = fakes.root.join(glue::HOOK);
    assert_eq!(std::fs::read_to_string(&hook).unwrap(), glue::HOOK_SCRIPT);
    assert_eq!(std::fs::metadata(&hook).unwrap().permissions().mode() & 0o777, 0o755);
    let settings = std::fs::read_to_string(fakes.root.join(glue::SETTINGS)).unwrap();
    assert!(settings.starts_with("{\n  \"permissions\": {"), "{settings}");
    assert!(
        settings.contains(&format!("\"command\": \"{}\"", glue::HOOK_COMMAND)),
        "{settings}"
    );

    let before = written(&fakes.root);
    let (result, output) = fakes.init(&[], &Fixed);
    assert!(matches!(result, Err(SetupError::PinExists { .. })));
    assert_eq!(output, "");
    assert_eq!(written(&fakes.root), before);
    assert_eq!(glue::write(&fakes.root).unwrap(), Vec::<&str>::new());
    assert_eq!(written(&fakes.root), before);
}

#[test]
fn the_glue_writers_restore_the_hook_and_keep_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        glue::write(dir.path()).unwrap(),
        [glue::AGENTS, glue::HOOK, glue::SETTINGS]
    );
    let hook = dir.path().join(glue::HOOK);
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(glue::write(dir.path()).unwrap(), [glue::HOOK]);
    assert_eq!(std::fs::metadata(&hook).unwrap().permissions().mode() & 0o777, 0o755);
    assert_eq!(glue::write(dir.path()).unwrap(), Vec::<&str>::new());

    // A hook checked out under umask 002 keeps its group write bit.
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o775)).unwrap();
    assert_eq!(glue::write(dir.path()).unwrap(), Vec::<&str>::new());
    assert_eq!(std::fs::metadata(&hook).unwrap().permissions().mode() & 0o777, 0o775);
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o754)).unwrap();
    assert_eq!(glue::write(dir.path()).unwrap(), [glue::HOOK]);
}

#[test]
fn a_linked_agents_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink("elsewhere.md", dir.path().join(glue::AGENTS)).unwrap();
    let err = glue::write(dir.path()).unwrap_err().to_string();
    assert!(
        err.ends_with("AGENTS.md is a symlink, which sltk does not follow"),
        "{err}"
    );
    assert!(!dir.path().join(".claude").exists());
}
