//! `sltk`, the storm stack's command-line interface.

mod init;
mod prompt;

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use sltk_core::doctor::{self, Machine, Note, Report};
use sltk_core::host::{self, GitHub};
use sltk_core::install::{self, HttpFetch, InstallError};
use sltk_core::platform::{Host, Plan};
use sltk_core::train::{Tier, Train};

/// The exit code of a failure that wrote nothing to the bin directory.
const FAILED: u8 = 1;

/// The exit code of `sltk install` when the person answered no: it printed
/// the plan and installed nothing, so a script must not read it as success.
const DECLINED: u8 = 3;

/// The exit code of an install that replaced some tools and then failed, so
/// the bin directory holds a mix of new and old binaries.
const PARTIAL: u8 = 4;

/// The exit code of `sltk doctor` when it could not finish its checks, as
/// when the pin cannot be read.
const DOCTOR_ERROR: u8 = 2;

/// The question `sltk install` asks on a terminal without `--yes`.
const QUESTION: &str = "Install these? [Y/n] ";

/// The last line `sltk install` prints when the person answers no.
const DECLINED_LINE: &str = "sltk: nothing was installed\n";

/// The commands `sltk` runs.
#[derive(Subcommand)]
enum Command {
    /// Print the version and tier of each tool this release pins.
    Train,
    /// Install the pinned fab and arcl, and any optional tool chosen.
    ///
    /// Downloads each tool's release archive, checks it against the sha256
    /// the train pins, and puts the binary in $CARGO_HOME/bin, or
    /// ~/.cargo/bin when CARGO_HOME is unset or empty. It downloads and checks
    /// every archive before it writes anything. Downloads go through the
    /// proxy in the first of ALL_PROXY, HTTPS_PROXY and HTTP_PROXY that is
    /// set, upper or lower case, except for hosts in NO_PROXY.
    ///
    /// On a terminal without --yes or --with it lists the optional tools,
    /// unticked, and asks which to add by number or name; Enter adds none.
    /// It skips that question when the train lists no optional tool. It
    /// prints what it will install, and on a terminal without --yes it then
    /// asks before installing, and exits 3 if the answer is no. Off a
    /// terminal, where stdin or stdout is not one, it asks nothing and
    /// installs fab, arcl and the --with tools. Exits 0 once every tool is
    /// installed. Exits 1 when it wrote nothing: a tool has no archive for
    /// this machine, a --with name is wrong, or a download, hash check or
    /// write fails. Exits 4 when it replaced some tools and then failed,
    /// leaving the rest as they were; the error names the tools it replaced.
    Install {
        /// Install without asking. Off a terminal sltk never asks.
        #[arg(long)]
        yes: bool,
        /// Also install this optional tool, and ask for no others. Repeat
        /// for more than one.
        #[arg(long = "with", value_name = "TOOL")]
        with: Vec<String>,
    },
    /// Report what is wrong with the installed tools. Changes nothing.
    ///
    /// Checks fab and arcl, and each optional tool that .github/sltk.toml
    /// lists, or, without a pin, each one installed. It looks for the pin
    /// from the current directory up to the root of its git repository, and
    /// reads none outside a repository. It reports a tool missing from the
    /// bin directory, a version other than the train's, a binary PATH finds
    /// before the bin directory's, and whether that one is Homebrew's, a bin
    /// directory missing from PATH, a pin of another sltk release, and a pin
    /// it will not read: a symlink, not a regular file, or over 64 KiB.
    ///
    /// Inside a git repository whose remote origin is on GitHub, it also
    /// asks GitHub through `gh api` whether stormlight-thunderbot is
    /// installed, THUNDERBOT_APP_ID and THUNDERBOT_PRIVATE_KEY are available
    /// to the repository, and storm/review is required on main. Without gh
    /// or a GitHub origin it skips these and says so. An answer gh could not
    /// give, as when the token lacks a permission, is printed as "could not
    /// check" and is not a problem.
    ///
    /// Exits 0 when nothing is wrong, 1 when something is, and 2 when it
    /// could not check the tools, as when the pin cannot be read or parsed.
    Doctor,
    /// Set up this repository: run fab's and arcl's setup, write the session
    /// hook and the Tasks section, then write the pin.
    ///
    /// Runs `fab init` with --owner, --trailer and --gates, then
    /// `arcl init --workspace` and `arcl skill install`, in the repository
    /// root: --dir, or the top of the git repository holding the current
    /// directory. On a terminal it asks for each of the three settings left
    /// out; off a terminal a missing one is an error. It stops at the first
    /// command that fails and names the commands that ran and the ones that
    /// did not; rerunning with the same flags finishes the setup. When every
    /// command succeeds it writes the Tasks section into AGENTS.md between
    /// the sltk:tasks markers, .claude/hooks/session-start.sh, and the
    /// hook's SessionStart entry in .claude/settings.json, keeping the
    /// file's other keys and hooks. It refuses, before writing any of them,
    /// a hook it did not write, a ## Tasks heading outside the markers, and
    /// a symlink in any of their paths. Then it writes .github/sltk.toml,
    /// pinning this sltk release and the sha256 of its archives, read from
    /// the release's .sha256 files. It reads those first, so it changes
    /// nothing when this sltk has no published release. Before the first
    /// command it records the flags and the train's fab version in
    /// sltk-init in the git directory, and removes that record after
    /// writing the pin. It refuses a repository that already has
    /// .github/sltk.toml, or any .github/fabrial.toml but one an earlier run
    /// left: the record matches the flags, and the file holds the same
    /// settings and the train's fab version. Then it skips `fab init`.
    ///
    /// A repository with .github/fabrial.toml, no pin and no sltk-init
    /// record was set up by hand, and sltk init adopts it. It keeps the
    /// settings in .github/fabrial.toml, so --owner, --trailer and --gates
    /// are optional and must match the file when given. It runs
    /// `arcl skill install --check`, which writes nothing, then prints the
    /// commands it will run and a diff of each file it writes: the session
    /// hook, which replaces any other, the Tasks section, which replaces a
    /// ## Tasks section outside the markers in place, the settings entry and
    /// the pin. The Tasks section goes in whichever of AGENTS.md and
    /// CLAUDE.md holds the sltk:tasks markers, else in AGENTS.md, or in
    /// CLAUDE.md when the repository has that and no AGENTS.md. A ## Tasks
    /// section outside the markers ends at the next # or ## heading or
    /// HTML comment line. Without --yes it asks
    /// before changing anything, and exits 3 without writing when the
    /// answer is no, or off a terminal. With a yes it writes those files,
    /// runs `fab update`, `arcl init --workspace` when there is no .arcl,
    /// and `arcl skill install` when the check exited 4, then writes the
    /// pin. A failure leaves no pin, and a rerun adopts again.
    ///
    /// Exits 0 when set up, 3 when adoption is declined or not confirmed,
    /// and 1 on any other failure.
    Init {
        /// GitHub login of the person who approves pull requests.
        #[arg(long)]
        owner: Option<String>,
        /// The squash commit's Co-authored-by value, as "Name <email>".
        #[arg(long)]
        trailer: Option<String>,
        /// The command agents run before pushing.
        #[arg(long)]
        gates: Option<String>,
        /// Also pin this optional tool. Repeat for more than one.
        #[arg(long = "with", value_name = "TOOL")]
        with: Vec<String>,
        /// The repository root.
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Adopt a repository set up by hand without asking.
        #[arg(long)]
        yes: bool,
    },
}

/// What a person chose for `sltk install`, by flag or by where it runs.
struct Choices<'a> {
    /// `--yes`: install without asking.
    yes: bool,
    /// Whether stdin and stdout are terminals, from [`on_terminal`].
    terminal: bool,
    /// The `--with` names.
    with: &'a [String],
}

/// sltk: installs fabrial, arclightning and, if asked, Mire onto a machine and
/// into a repository.
#[derive(Parser)]
#[command(name = "sltk", version = sltk_core::VERSION, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

fn main() -> ExitCode {
    let command = Cli::parse().command;
    let train = match Train::embedded() {
        Ok(train) => train,
        Err(err) => {
            eprintln!("sltk: {err}");
            return ExitCode::FAILURE;
        }
    };
    let result = match command {
        Command::Train => {
            print!("{}", pinned_set(&train));
            Ok(0)
        }
        Command::Install { yes, with } => {
            let (mut input, mut output) = (io::stdin().lock(), io::stdout().lock());
            run_install(
                &train,
                Choices { yes, terminal: on_terminal(), with: &with },
                &mut input,
                &mut output,
            )
        }
        Command::Doctor => run_doctor(&train),
        Command::Init { owner, trailer, gates, with, dir, yes } => {
            let flags = init::Flags { owner, trailer, gates, with, dir, yes };
            let (mut input, mut output) = (io::stdin().lock(), io::stdout().lock());
            init::run(&train, &flags, on_terminal(), &mut input, &mut output)
        }
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err((code, err)) => {
            eprint!("{err}");
            ExitCode::from(code)
        }
    }
}

/// The exit code for an install that failed with `err`: [`PARTIAL`] when it
/// replaced some tools, else [`FAILED`].
fn install_exit_code(err: &InstallError) -> u8 {
    match err {
        InstallError::Partial { .. } => PARTIAL,
        _ => FAILED,
    }
}

/// Whether a person can answer a question: stdin and stdout are both
/// terminals.
fn on_terminal() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// The optional tools to install: the `--with` names, or, on a terminal
/// without `--yes` or `--with`, the ones the person picks from the train's
/// optional tools. Asks nothing when the train lists no optional tool.
fn optional_tools(
    train: &Train, choices: &Choices, input: &mut impl BufRead, output: &mut impl Write,
) -> io::Result<Vec<String>> {
    if choices.yes || !choices.terminal || !choices.with.is_empty() {
        return Ok(choices.with.to_vec());
    }
    let names = |tier| -> Vec<&str> {
        train
            .tools
            .iter()
            .filter(|tool| tool.tier == tier)
            .map(|tool| tool.name.as_str())
            .collect()
    };
    let optional = names(Tier::Optional);
    if optional.is_empty() {
        return Ok(Vec::new());
    }
    prompt::pick(input, output, &names(Tier::Essential), &optional)
}

/// Whether to install the plan: yes with `--yes` or off a `terminal`, else the
/// answer to [`QUESTION`], read from `input`.
fn should_install(yes: bool, terminal: bool, input: &mut impl BufRead, output: &mut impl Write) -> io::Result<bool> {
    if yes || !terminal {
        return Ok(true);
    }
    prompt::confirm(input, output, QUESTION, true)
}

/// Runs `sltk install`: settles the tools with [`optional_tools`], writes the
/// plan to `output`, then installs it when [`should_install`] says to,
/// reading any answer from `input`. Returns the exit code, or the exit code
/// and the error to print.
fn run_install(
    train: &Train, choices: Choices, input: &mut impl BufRead, output: &mut impl Write,
) -> Result<u8, (u8, String)> {
    let io_failed = |err: io::Error| (FAILED, format!("sltk: could not ask before installing: {err}\n"));
    let with = optional_tools(train, &choices, input, output).map_err(io_failed)?;
    let with: Vec<&str> = with.iter().map(String::as_str).collect();
    let plan = Plan::new(train, Host::detect(), &with).map_err(|err| (FAILED, format!("{err}\n")))?;
    let report = install_report(&plan).map_err(|err| (FAILED, err))?;
    output.write_all(report.as_bytes()).map_err(io_failed)?;
    if !should_install(choices.yes, choices.terminal, input, output).map_err(io_failed)? {
        output.write_all(DECLINED_LINE.as_bytes()).map_err(io_failed)?;
        return Ok(DECLINED);
    }
    let failed = |err: InstallError| (install_exit_code(&err), format!("{err}\n"));
    let bin = install::bin_dir(std::env::var_os("CARGO_HOME"), std::env::var_os("HOME")).map_err(failed)?;
    let installed = install::install(&plan, &HttpFetch::new(), &bin).map_err(failed)?;
    for tool in installed {
        // The tools are installed, so a closed stdout must not turn that into a
        // failure.
        let _ = writeln!(
            output,
            "installed {} {} to {}",
            tool.name,
            tool.version,
            tool.path.display()
        );
    }
    Ok(0)
}

/// Runs `sltk doctor` on this machine, with the pin of the repository the
/// current directory is in, if it has one. Returns the exit code, or the exit
/// code and the error to print.
fn run_doctor(train: &Train) -> Result<u8, (u8, String)> {
    let error = |err: &dyn std::fmt::Display| (DOCTOR_ERROR, format!("sltk: {err}\n"));
    let machine = Machine::from_env().map_err(|err| (DOCTOR_ERROR, format!("{err}\n")))?;
    let cwd = std::env::current_dir().map_err(|err| error(&err))?;
    let pin = match doctor::find_pin_root(&cwd) {
        Some(root) => Some(doctor::read_pin(&root, train, sltk_core::VERSION).map_err(|err| error(&err))?),
        None => None,
    };
    let mut report = doctor::check(train, pin.as_ref(), sltk_core::VERSION, &machine);
    if let Some(root) = doctor::find_repo_root(&cwd) {
        match GitHub::detect(root, machine.path.as_deref(), host::HOST_TIMEOUT, host::HOST_DEADLINE) {
            Ok(github) => doctor::check_repository(&github, &mut report),
            Err(reason) => report.notes.push(Note::Skipped { reason }),
        }
    }
    print!("{}", doctor_report(&report, &machine));
    Ok(if report.findings.is_empty() { 0 } else { FAILED })
}

/// What `sltk doctor` prints: one line per finding, one per repository check
/// it skipped or could not finish, then a line naming what it checked and how
/// many problems it found.
fn doctor_report(report: &Report, machine: &Machine) -> String {
    let mut out: String = report.findings.iter().map(|finding| format!("{finding}\n")).collect();
    out.extend(report.notes.iter().map(|note| format!("{note}\n")));
    let repository = report
        .repository
        .as_ref()
        .map(|repository| format!(" and {repository}"))
        .unwrap_or_default();
    let problems = match report.findings.len() {
        0 => "no problems".to_owned(),
        1 => "1 problem".to_owned(),
        n => format!("{n} problems"),
    };
    out.push_str(&format!(
        "sltk doctor: checked {} in {}{repository}: {problems}\n",
        report.checked.join(", "),
        machine.bin.display()
    ));
    out
}

/// What `sltk install` prints for a plan: the tools and archives it will
/// install, or, as an error, the tools with no archive for the host.
fn install_report(plan: &Plan) -> Result<String, String> {
    if !plan.is_complete() {
        let mut err = format!("sltk: these tools have no archive for {}:\n", plan.host);
        for tool in &plan.unsupported {
            err.push_str(&format!("  {} {}\n", tool.name, tool.version));
        }
        return Err(err);
    }
    let mut out = format!("sltk plan for {}:\n", plan.host);
    for tool in &plan.install {
        out.push_str(&format!(
            "  {} {} from {}\n",
            tool.name, tool.version, tool.archive.name
        ));
    }
    Ok(out)
}

/// One line per tool: its name, version and tier.
fn pinned_set(train: &Train) -> String {
    train
        .tools
        .iter()
        .map(|tool| format!("{} {} {}\n", tool.name, tool.version, tool.tier))
        .collect()
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;
    use clap::error::ErrorKind;
    use sltk_core::platform::{Host, Plan};
    use sltk_core::train::{Platform, Tier, Train};

    use std::path::PathBuf;

    use sltk_core::install::InstallError;

    use sltk_core::doctor::{Finding, Machine, Note, Report};

    use super::{
        Choices, Cli, DECLINED, DECLINED_LINE, FAILED, PARTIAL, QUESTION, doctor_report, install_exit_code,
        install_report, optional_tools, pinned_set, run_install, should_install,
    };

    /// A train like the real one with mire listed: fab and arcl essential,
    /// mire optional, each with an archive for both platforms.
    const STACK: &str = r#"
[[tool]]
name = "fab"
tier = "essential"
binary = "fab"
version = "0.8.0"
url = "https://example.test/v{version}/{archive}"

[tool.platforms.aarch64-apple-darwin]
archive = "fab-aarch64-apple-darwin.tar.xz"
sha256 = "bd8c707af7c6088c1afdf57ff5b9e9f1c4f4830d0dabf1432c4b5ebaced33fe0"

[tool.platforms.x86_64-unknown-linux-gnu]
archive = "fab-x86_64-unknown-linux-gnu.tar.xz"
sha256 = "81cf21ffd2140482a4ea315e60ca34ead891c18e9aa65e7f05daf7fda2404498"

[[tool]]
name = "arcl"
tier = "essential"
binary = "arcl"
version = "0.3.0"
url = "https://example.test/v{version}/{archive}"

[tool.platforms.aarch64-apple-darwin]
archive = "arcl-aarch64-apple-darwin.tar.xz"
sha256 = "7cf3c94e04bac6c9dff4fd357fc8133050fe0dbbca9d4d1dab2e4ecd8c5e13c3"

[tool.platforms.x86_64-unknown-linux-gnu]
archive = "arcl-x86_64-unknown-linux-musl.tar.xz"
sha256 = "80f6bc7e4ce41f7037c91330c814d94e277aed381d192960d00a130a859675e3"

[[tool]]
name = "mire"
tier = "optional"
binary = "mire"
version = "0.1.0"
url = "https://example.test/v{version}/{archive}"

[tool.platforms.aarch64-apple-darwin]
archive = "mire-aarch64-apple-darwin.tar.xz"
sha256 = "bd8c707af7c6088c1afdf57ff5b9e9f1c4f4830d0dabf1432c4b5ebaced33fe0"

[tool.platforms.x86_64-unknown-linux-gnu]
archive = "mire-x86_64-unknown-linux-gnu.tar.xz"
sha256 = "81cf21ffd2140482a4ea315e60ca34ead891c18e9aa65e7f05daf7fda2404498"
"#;

    /// What the picker lists for [`STACK`].
    const STACK_LIST: &str = "Always installed: fab, arcl\nOptional:\n  1. [ ] mire\n";

    /// The optional tools [`optional_tools`] settles on for [`STACK`] when
    /// `typed` is the answer, and what it wrote.
    fn optional(yes: bool, terminal: bool, with: &[String], typed: &str) -> (Vec<String>, String) {
        let train = Train::parse(STACK).unwrap();
        let mut output = Vec::new();
        let picked = optional_tools(
            &train,
            &Choices { yes, terminal, with },
            &mut typed.as_bytes(),
            &mut output,
        );
        (picked.unwrap(), String::from_utf8(output).unwrap())
    }

    #[test]
    fn on_a_terminal_without_flags_the_picker_offers_mire_unticked() {
        let (picked, output) = optional(false, true, &[], "\n");
        assert!(picked.is_empty());
        assert!(output.starts_with(STACK_LIST), "{output}");
        assert_eq!(optional(false, true, &[], "1\n").0, ["mire"]);
        assert_eq!(optional(false, true, &[], "mire\n").0, ["mire"]);
        assert!(optional(false, true, &[], "").0.is_empty());
    }

    #[test]
    fn off_a_terminal_the_picker_stays_closed() {
        assert_eq!(optional(false, false, &[], "1\n"), (Vec::new(), String::new()));
        let with = ["mire".to_owned()];
        assert_eq!(optional(false, false, &with, "\n"), (with.to_vec(), String::new()));
    }

    #[test]
    fn with_or_yes_closes_the_picker() {
        let with = ["mire".to_owned()];
        assert_eq!(optional(false, true, &with, "\n"), (with.to_vec(), String::new()));
        assert_eq!(optional(true, true, &[], "1\n"), (Vec::new(), String::new()));
    }

    #[test]
    fn a_train_with_no_optional_tool_skips_the_picker() {
        let mut train = Train::parse(STACK).unwrap();
        train.tools.retain(|tool| tool.tier == Tier::Essential);
        let mut output = Vec::new();
        let choices = Choices { yes: false, terminal: true, with: &[] };
        let picked = optional_tools(&train, &choices, &mut "1\n".as_bytes(), &mut output).unwrap();
        assert!(picked.is_empty());
        assert!(output.is_empty());
    }

    /// On a terminal the picker comes first, its choice joins the plan, and
    /// the confirmation follows it.
    #[test]
    fn a_picked_tool_joins_the_plan_before_the_confirmation() {
        if Host::detect().platform().is_none() {
            return;
        }
        let train = Train::parse(STACK).unwrap();
        let choices = Choices { yes: false, terminal: true, with: &[] };
        let mut output = Vec::new();
        let code = run_install(&train, choices, &mut "1\nn\n".as_bytes(), &mut output).unwrap();
        assert_eq!(code, DECLINED);
        let output = String::from_utf8(output).unwrap();
        assert!(output.starts_with(STACK_LIST), "{output}");
        assert!(output.contains("\n  mire 0.1.0 from mire-"), "{output}");
        assert!(output.ends_with(&format!("{QUESTION}{DECLINED_LINE}")), "{output}");
    }

    /// End of input at the picker adds nothing, and at the confirmation means
    /// no.
    #[test]
    fn end_of_input_picks_nothing_and_installs_nothing() {
        if Host::detect().platform().is_none() {
            return;
        }
        let train = Train::parse(STACK).unwrap();
        let choices = Choices { yes: false, terminal: true, with: &[] };
        let mut output = Vec::new();
        let code = run_install(&train, choices, &mut "".as_bytes(), &mut output).unwrap();
        assert_eq!(code, DECLINED);
        let output = String::from_utf8(output).unwrap();
        assert!(!output.contains("  mire 0.1.0 from"), "{output}");
        assert!(output.ends_with(&format!("{QUESTION}\n{DECLINED_LINE}")), "{output}");
    }

    /// Answering no on a terminal prints the plan and the declined line, and
    /// exits 3 before any download. The train has no optional tool, so the
    /// confirmation is the only question.
    #[test]
    fn a_declined_install_exits_3_and_downloads_nothing() {
        if Host::detect().platform().is_none() {
            return;
        }
        let mut train = Train::parse(STACK).unwrap();
        train.tools.retain(|tool| tool.tier == Tier::Essential);
        let choices = Choices { yes: false, terminal: true, with: &[] };
        let mut output = Vec::new();
        let code = run_install(&train, choices, &mut "no\n".as_bytes(), &mut output).unwrap();
        assert_eq!((code, DECLINED), (3, 3));
        let output = String::from_utf8(output).unwrap();
        assert!(output.starts_with("sltk plan for "), "{output}");
        assert!(output.ends_with(&format!("{QUESTION}{DECLINED_LINE}")), "{output}");
    }

    /// What [`should_install`] decides when `typed` is the answer, and what it
    /// wrote.
    fn decide(yes: bool, terminal: bool, typed: &str) -> (bool, String) {
        let mut output = Vec::new();
        let install = should_install(yes, terminal, &mut typed.as_bytes(), &mut output).unwrap();
        (install, String::from_utf8(output).unwrap())
    }

    #[test]
    fn off_a_terminal_install_goes_ahead_without_asking() {
        assert_eq!(decide(false, false, "n\n"), (true, String::new()));
    }

    #[test]
    fn yes_goes_ahead_without_asking() {
        assert_eq!(decide(true, true, "n\n"), (true, String::new()));
    }

    #[test]
    fn on_a_terminal_install_asks_first() {
        assert_eq!(decide(false, true, "\n"), (true, QUESTION.to_owned()));
        assert_eq!(decide(false, true, "yes\n"), (true, QUESTION.to_owned()));
        assert_eq!(decide(false, true, "n\n"), (false, QUESTION.to_owned()));
        assert_eq!(decide(false, true, ""), (false, format!("{QUESTION}\n")));
        assert_eq!(
            decide(false, true, "what\ny\n"),
            (true, format!("{QUESTION}answer y or n\n{QUESTION}"))
        );
    }

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn a_partial_install_exits_4_and_a_failure_that_wrote_nothing_exits_1() {
        let denied = || std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let partial = InstallError::Partial {
            path: PathBuf::from("/bin/arcl"),
            source: denied(),
            replaced: vec!["fab".to_owned()],
        };
        let nothing = InstallError::Io { path: PathBuf::from("/bin/fab"), source: denied() };
        assert_eq!((PARTIAL, FAILED), (4, 1));
        assert_eq!(install_exit_code(&partial), 4);
        assert_eq!(install_exit_code(&nothing), 1);
        assert_eq!(install_exit_code(&InstallError::NoBinDir), 1);
    }

    #[test]
    fn version_prints_the_name_and_release() {
        let err = Cli::command().try_get_matches_from(["sltk", "--version"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayVersion);
        assert_eq!(err.to_string(), format!("sltk {}\n", env!("CARGO_PKG_VERSION")));
    }

    /// A two-tool train, so train bumps leave this test alone.
    const FIXTURE: &str = r#"
[[tool]]
name = "alpha"
tier = "essential"
binary = "alpha"
version = "1.2.3"
url = "https://example.test/v{version}/{archive}"

[tool.platforms.x86_64-unknown-linux-gnu]
archive = "alpha-x86_64-unknown-linux-gnu.tar.xz"
sha256 = "bd8c707af7c6088c1afdf57ff5b9e9f1c4f4830d0dabf1432c4b5ebaced33fe0"

[[tool]]
name = "beta"
tier = "optional"
binary = "beta"
version = "0.1.0"
url = "https://example.test/v{version}/{archive}"

[tool.platforms.aarch64-apple-darwin]
archive = "beta-aarch64-apple-darwin.tar.xz"
sha256 = "81cf21ffd2140482a4ea315e60ca34ead891c18e9aa65e7f05daf7fda2404498"
"#;

    #[test]
    fn train_prints_one_line_per_tool() {
        let train = Train::parse(FIXTURE).unwrap();
        assert_eq!(pinned_set(&train), "alpha 1.2.3 essential\nbeta 0.1.0 optional\n");
    }

    #[test]
    fn the_embedded_train_prints() {
        let train = Train::embedded().unwrap();
        assert_eq!(pinned_set(&train).lines().count(), train.tools.len());
    }

    #[test]
    fn install_lists_the_tools_and_archives_it_would_install() {
        let train = Train::parse(FIXTURE).unwrap();
        let plan = Plan::new(&train, Host::Supported(Platform::LinuxX86_64), &[]).unwrap();
        assert_eq!(
            install_report(&plan).unwrap(),
            "sltk plan for x86_64-unknown-linux-gnu:\n  alpha 1.2.3 from alpha-x86_64-unknown-linux-gnu.tar.xz\n"
        );
    }

    #[test]
    fn install_fails_with_the_tools_a_host_cannot_get() {
        let train = Train::embedded().unwrap();
        let plan = Plan::new(&train, Host::from_parts("aarch64", "linux", "gnu", 64), &[]).unwrap();
        let err = install_report(&plan).unwrap_err();
        let mut expected = "sltk: these tools have no archive for aarch64 linux:\n".to_owned();
        for tool in &train.tools {
            expected.push_str(&format!("  {} {}\n", tool.name, tool.version));
        }
        assert_eq!(err, expected);
    }

    #[test]
    fn install_fails_naming_only_the_unsupported_tools() {
        let train = Train::parse(FIXTURE).unwrap();
        let plan = Plan::new(&train, Host::Supported(Platform::LinuxX86_64), &["beta"]).unwrap();
        assert_eq!(
            install_report(&plan).unwrap_err(),
            "sltk: these tools have no archive for x86_64-unknown-linux-gnu:\n  beta 0.1.0\n"
        );
    }

    #[test]
    fn doctor_prints_each_finding_then_a_summary() {
        let machine = Machine {
            path: None,
            bin: PathBuf::from("/h/.cargo/bin"),
            brew_prefixes: Vec::new(),
            timeout: std::time::Duration::from_secs(5),
        };
        let mut report = Report {
            checked: vec!["fab".into(), "arcl".into()],
            findings: Vec::new(),
            repository: None,
            notes: Vec::new(),
        };
        assert_eq!(
            doctor_report(&report, &machine),
            "sltk doctor: checked fab, arcl in /h/.cargo/bin: no problems\n"
        );
        report.findings.push(Finding::BinNotOnPath { bin: machine.bin.clone() });
        report
            .findings
            .push(Finding::Missing { tool: "arcl".into(), path: PathBuf::from("/h/.cargo/bin/arcl") });
        assert_eq!(
            doctor_report(&report, &machine),
            "PATH does not include /h/.cargo/bin\n\
             arcl: /h/.cargo/bin/arcl is not installed\n\
             sltk doctor: checked fab, arcl in /h/.cargo/bin: 2 problems\n"
        );
    }

    #[test]
    fn doctor_prints_repository_findings_and_notes_before_the_summary() {
        let machine = Machine {
            path: None,
            bin: PathBuf::from("/h/.cargo/bin"),
            brew_prefixes: Vec::new(),
            timeout: std::time::Duration::from_secs(5),
        };
        let report = Report {
            checked: vec!["fab".into(), "arcl".into()],
            findings: vec![Finding::SecretMissing { name: "THUNDERBOT_APP_ID".into() }],
            repository: Some("o/r on GitHub".into()),
            notes: vec![Note::CouldNotCheck {
                what: "secret THUNDERBOT_PRIVATE_KEY".into(),
                reason: "HTTP 403".into(),
            }],
        };
        assert_eq!(
            doctor_report(&report, &machine),
            "secret THUNDERBOT_APP_ID is not available to this repository\n\
             could not check secret THUNDERBOT_PRIVATE_KEY: HTTP 403\n\
             sltk doctor: checked fab, arcl in /h/.cargo/bin and o/r on GitHub: 1 problem\n"
        );
        let skipped = Report {
            checked: vec!["fab".into()],
            findings: Vec::new(),
            repository: None,
            notes: vec![Note::Skipped { reason: "gh not found".into() }],
        };
        assert_eq!(
            doctor_report(&skipped, &machine),
            "skipped repository checks: gh not found\n\
             sltk doctor: checked fab in /h/.cargo/bin: no problems\n"
        );
    }
}
