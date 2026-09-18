//! `cdsl` — the command line interface.
//!
//! Contract (brief §11):
//!
//! - `check` runs the whole front end and stops before simulating.
//! - `run` really executes the analyses and writes result files.
//! - `capabilities` reports what the backend can actually do.
//! - Diagnostics go to stderr, data and summaries to stdout.
//! - Exit code 0 on success, 1 for a user error, 2 for an internal failure.
//!
//! Every command that reads a file refuses to write back over it, so a typo in
//! `--out` cannot destroy the source.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod check;
mod repl;
mod run;

use clap::{Parser, Subcommand, ValueEnum};

/// Exit codes, fixed by the CLI contract.
pub const EXIT_OK: u8 = 0;
pub const EXIT_USER_ERROR: u8 = 1;
pub const EXIT_INTERNAL: u8 = 2;

#[derive(Parser, Debug)]
#[command(
    name = "cdsl",
    version,
    about = "Describe, simulate and measure analog circuits",
    long_about = "A circuit description language with a Rust simulation backend.\n\n\
                  Source files use the .cdsl extension; see docs/language.md."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Include backend and solver detail in diagnostics.
    #[arg(long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Parse, check and elaborate a source file without simulating.
    ///
    /// Reports syntax, name, dimension, elaboration and backend-capability
    /// problems. Does not run any analysis.
    Check {
        /// Source file to check.
        file: PathBuf,
        /// Print the elaborated circuit as JSON instead of a summary.
        #[arg(long)]
        json: bool,
    },

    /// Run an experiment and write its results.
    Run {
        /// Source file to run.
        file: PathBuf,
        /// Experiment to run. Required when the file defines more than one.
        #[arg(long)]
        experiment: Option<String>,
        /// Directory to write results into.
        #[arg(long, default_value = "results")]
        out: PathBuf,
        /// Output format.
        #[arg(long, value_enum, default_value_t = Format::Both)]
        format: Format,
    },

    /// List the available backend and what it supports.
    Capabilities,

    /// Start an interactive session.
    ///
    /// Evaluates expressions, defines circuits and experiments, and runs them.
    /// `:help` lists the commands; see docs/repl.md.
    Repl {
        /// Load this file into the session before the first prompt.
        file: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub enum Format {
    Csv,
    Json,
    Both,
}

impl Format {
    pub fn csv(self) -> bool {
        matches!(self, Format::Csv | Format::Both)
    }
    pub fn json(self) -> bool {
        matches!(self, Format::Json | Format::Both)
    }
}

/// Stack for the thread every command runs on.
///
/// The parser and the result-expression evaluator are recursive, and an
/// expression may nest up to `circuit_core::limits::MAX_EXPR_DEPTH` levels
/// before either refuses it. An unoptimised build needs several kilobytes of
/// stack per level, so the default 1 MiB main thread would abort the process —
/// with no diagnostic and no exit code the CLI controls — well before the
/// limit is reached. 64 MiB leaves a wide margin over the deepest accepted
/// expression (round-4 FINDING-1).
const WORK_STACK_BYTES: usize = 64 * 1024 * 1024;

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Run the command on a thread with a stack the accepted depth cannot
    // exhaust. `main` itself stays thin: it only joins the worker and turns
    // its result into an exit code.
    let worker = std::thread::Builder::new()
        .name("cdsl".to_string())
        .stack_size(WORK_STACK_BYTES)
        .spawn(move || dispatch(&cli));

    match worker {
        Ok(handle) => match handle.join() {
            Ok(code) => ExitCode::from(code),
            Err(_) => {
                eprintln!("error[E_INTERNAL]: the processing thread panicked");
                ExitCode::from(EXIT_USER_ERROR)
            }
        },
        Err(e) => {
            eprintln!("error[E_INTERNAL]: cannot start the processing thread: {e}");
            ExitCode::from(EXIT_USER_ERROR)
        }
    }
}

fn dispatch(cli: &Cli) -> u8 {
    let result = match &cli.command {
        Command::Check { file, json } => check::check(file, *json, cli.verbose),
        Command::Run {
            file,
            experiment,
            out,
            format,
        } => run::run(file, experiment.as_deref(), out, *format, cli.verbose),
        Command::Capabilities => {
            check::capabilities(cli.verbose);
            Ok(())
        }
        Command::Repl { file } => repl::repl(file.as_deref(), cli.verbose),
    };

    match result {
        Ok(()) => EXIT_OK,
        Err(code) => code,
    }
}

/// Read a source file, reporting a readable error if it cannot be read.
pub fn read_source(path: &Path) -> Result<String, u8> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) => {
            eprintln!("error[E_IO]: cannot read `{}`: {e}", path.display());
            Err(EXIT_USER_ERROR)
        }
    }
}

/// Refuse to write a result over the input source.
///
/// The brief requires that output paths never overwrite the input; this is the
/// check that makes that true rather than merely intended.
pub fn guard_output(input: &Path, output: &Path) -> Result<(), u8> {
    let same = match (input.canonicalize(), output.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        // The output usually does not exist yet; compare what we can.
        _ => input == output,
    };
    if same {
        eprintln!(
            "error[E_IO]: refusing to write results over the input file `{}`",
            input.display()
        );
        return Err(EXIT_USER_ERROR);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_selects_channels() {
        assert!(Format::Both.csv() && Format::Both.json());
        assert!(Format::Csv.csv() && !Format::Csv.json());
        assert!(!Format::Json.csv() && Format::Json.json());
    }

    #[test]
    fn cli_definition_is_valid() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn guard_rejects_writing_over_the_input() {
        let dir = std::env::temp_dir();
        let f = dir.join("cdsl_guard_test.cdsl");
        std::fs::write(&f, "x").expect("write");
        assert!(guard_output(&f, &f).is_err());
        assert!(guard_output(&f, &dir.join("other.csv")).is_ok());
        let _ = std::fs::remove_file(&f);
    }
}
