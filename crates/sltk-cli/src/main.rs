//! `sltk`, the storm stack's command-line interface.

mod prompt;

use std::io::{self, BufRead, IsTerminal, Write};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use sltk_core::install::{self, HttpFetch, InstallError};
use sltk_core::platform::{Host, Plan};
use sltk_core::train::Train;

/// The exit code of a failure that wrote nothing to the bin directory.
const FAILED: u8 = 1;

/// The exit code of `sltk install` when the person answered no: it printed
/// the plan and installed nothing, so a script must not read it as success.
const DECLINED: u8 = 3;

/// The exit code of an install that replaced some tools and then failed, so
/// the bin directory holds a mix of new and old binaries.
const PARTIAL: u8 = 4;

/// The question `sltk install` asks on a terminal without `--yes`.
const QUESTION: &str = "Install these? [Y/n] ";

/// The last line `sltk install` prints when the person answers no.
const DECLINED_LINE: &str = "sltk: nothing was installed\n";

/// The commands `sltk` runs.
#[derive(Subcommand)]
enum Command {
    /// Print the version and tier of each tool this release pins.
    Train,
    /// Install the pinned fab and arcl, and any tool named with --with.
    ///
    /// Downloads each tool's release archive, checks it against the sha256
    /// the train pins, and puts the binary in $CARGO_HOME/bin, or
    /// ~/.cargo/bin when CARGO_HOME is unset or empty. It downloads and checks
    /// every archive before it writes anything. Downloads go through the
    /// proxy in the first of ALL_PROXY, HTTPS_PROXY and HTTP_PROXY that is
    /// set, upper or lower case, except for hosts in NO_PROXY.
    ///
    /// It prints what it will install first. On a terminal without --yes it
    /// then asks before installing, and exits 3 if the answer is no. Off a
    /// terminal, where stdin or stdout is not one, it installs without
    /// asking. Exits 0 once every tool is installed. Exits 1 when it wrote
    /// nothing: a tool has no archive for this machine, a --with name is
    /// wrong, or a download, hash check or write fails. Exits 4 when it
    /// replaced some tools and then failed, leaving the rest as they were;
    /// the error names the tools it replaced.
    Install {
        /// Install without asking. Off a terminal sltk never asks.
        #[arg(long)]
        yes: bool,
        /// Also install this optional tool. Repeat for more than one.
        #[arg(long = "with", value_name = "TOOL")]
        with: Vec<String>,
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

/// Whether to install the plan: yes with `--yes` or off a `terminal`, else the
/// answer to [`QUESTION`], read from `input`.
fn should_install(yes: bool, terminal: bool, input: &mut impl BufRead, output: &mut impl Write) -> io::Result<bool> {
    if yes || !terminal {
        return Ok(true);
    }
    prompt::confirm(input, output, QUESTION)
}

/// Runs `sltk install`: writes the plan to `output`, then installs it when
/// [`should_install`] says to, reading any answer from `input`. Returns the
/// exit code, or the exit code and the error to print.
fn run_install(
    train: &Train, choices: Choices, input: &mut impl BufRead, output: &mut impl Write,
) -> Result<u8, (u8, String)> {
    let io_failed = |err: io::Error| (FAILED, format!("sltk: could not ask before installing: {err}\n"));
    let with: Vec<&str> = choices.with.iter().map(String::as_str).collect();
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
    use sltk_core::train::{Platform, Train};

    use std::path::PathBuf;

    use sltk_core::install::InstallError;

    use super::{
        Choices, Cli, DECLINED, DECLINED_LINE, FAILED, PARTIAL, QUESTION, install_exit_code, install_report,
        pinned_set, run_install, should_install,
    };

    /// Answering no on a terminal prints the plan and the declined line, and
    /// exits 3 before any download.
    #[test]
    fn a_declined_install_exits_3_and_downloads_nothing() {
        if Host::detect().platform().is_none() {
            return;
        }
        let train = Train::embedded().unwrap();
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
}
