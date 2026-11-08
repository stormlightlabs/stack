//! `sltk`, the storm stack's command-line interface.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use sltk_core::platform::{Host, Plan};
use sltk_core::train::Train;

/// The commands `sltk` runs.
#[derive(Subcommand)]
enum Command {
    /// Print the version and tier of each tool this release pins.
    Train,
    /// Print the tools and archives an install on this machine would use.
    ///
    /// Downloading comes in a later release: for now this prints the plan and
    /// downloads nothing. Exits non-zero, naming the tools, when a tool has no
    /// archive for this machine.
    Install,
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
        Command::Train => Ok(pinned_set(&train)),
        Command::Install => install_report(&Plan::new(&train, Host::detect(), &[])),
    };
    match result {
        Ok(out) => {
            print!("{out}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprint!("{err}");
            ExitCode::FAILURE
        }
    }
}

/// What `sltk install` prints for a plan: the tools and archives it would
/// install, or, as an error, the tools with no archive for the host.
fn install_report(plan: &Plan) -> Result<String, String> {
    if !plan.is_complete() {
        let mut err = format!("sltk: these tools have no archive for {}:\n", plan.host);
        for tool in &plan.unsupported {
            err.push_str(&format!("  {} {}\n", tool.name, tool.version));
        }
        return Err(err);
    }
    let mut out = format!("sltk would install on {}:\n", plan.host);
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

    use super::{Cli, install_report, pinned_set};

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
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
        let plan = Plan::new(&train, Host::Supported(Platform::LinuxX86_64), &[]);
        assert_eq!(
            install_report(&plan).unwrap(),
            "sltk would install on x86_64-unknown-linux-gnu:\n  alpha 1.2.3 from alpha-x86_64-unknown-linux-gnu.tar.xz\n"
        );
    }

    #[test]
    fn install_fails_with_the_tools_a_host_cannot_get() {
        let train = Train::embedded().unwrap();
        let plan = Plan::new(&train, Host::from_parts("aarch64", "linux"), &[]);
        let err = install_report(&plan).unwrap_err();
        let mut expected = "sltk: these tools have no archive for aarch64 linux:\n".to_owned();
        for tool in &train.tools {
            expected.push_str(&format!("  {} {}\n", tool.name, tool.version));
        }
        assert_eq!(err, expected);
    }

    #[test]
    fn install_fails_when_one_selected_tool_is_unsupported() {
        let train = Train::parse(FIXTURE).unwrap();
        let plan = Plan::new(&train, Host::Supported(Platform::AppleSilicon), &[]);
        assert_eq!(
            install_report(&plan).unwrap_err(),
            "sltk: these tools have no archive for aarch64-apple-darwin:\n  alpha 1.2.3\n"
        );
    }
}
