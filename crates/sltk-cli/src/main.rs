//! `sltk`, the storm stack's command-line interface.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use sltk_core::train::Train;

/// The commands `sltk` runs.
#[derive(Subcommand)]
enum Command {
    /// Print the version and tier of each tool this release pins.
    Train,
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
    match Cli::parse().command {
        Command::Train => match Train::embedded() {
            Ok(train) => {
                print!("{}", pinned_set(&train));
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("sltk: {err}");
                ExitCode::FAILURE
            }
        },
    }
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
    use sltk_core::train::Train;

    use super::{Cli, pinned_set};

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

    #[test]
    fn train_prints_one_line_per_tool() {
        let train = Train::embedded().unwrap();
        assert_eq!(pinned_set(&train), "fab 0.8.0 essential\narcl 0.3.0 essential\n");
    }
}
