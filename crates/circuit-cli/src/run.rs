//! `cdsl run`: execute an experiment and write its results.
//!
//! The run itself — including the parameter-sweep path, which re-elaborates
//! the design once per point — lives in `circuit_session::execute`, so the
//! REPL and this command cannot disagree about what an experiment produces.
//! What is left here is file handling and the printed summary.

use std::path::Path;

use circuit_backend::thevenin::TheveninBackend;
use circuit_core::diagnostic::{Code, Diagnostic, Diagnostics};
use circuit_results::dataset::Dataset;
use circuit_session::execute::Written;
use circuit_session::{Format, RunRequest, write_datasets};

use crate::{EXIT_USER_ERROR, Format as CliFormat, check, guard_output};

pub fn run(
    file: &Path,
    experiment: Option<&str>,
    out: &Path,
    format: CliFormat,
    verbose: bool,
) -> Result<(), u8> {
    let fe = check::front_end(file, verbose)?;
    let sources = &fe.sources;
    let compiled = &fe.compiled;

    // ---- choose the experiment ------------------------------------------
    let chosen = match experiment {
        Some(name) => match compiled.experiment(name) {
            Some(e) => e,
            None => {
                eprintln!(
                    "error[E_NAME]: no experiment named `{name}` in `{}`",
                    file.display()
                );
                let names: Vec<&str> = compiled
                    .experiments
                    .iter()
                    .map(|e| e.plan.name.as_str())
                    .collect();
                if names.is_empty() {
                    eprintln!("   = the file defines no experiments");
                } else {
                    eprintln!("   = experiments: {}", names.join(", "));
                }
                return Err(EXIT_USER_ERROR);
            }
        },
        None => match compiled.experiments.as_slice() {
            [only] => only,
            [] => {
                eprintln!(
                    "error[E_NAME]: `{}` defines no experiment to run",
                    file.display()
                );
                return Err(EXIT_USER_ERROR);
            }
            many => {
                eprintln!(
                    "error[E_ARGUMENT]: `{}` defines {} experiments; choose one with --experiment",
                    file.display(),
                    many.len()
                );
                let names: Vec<&str> = many.iter().map(|e| e.plan.name.as_str()).collect();
                eprintln!("   = experiments: {}", names.join(", "));
                return Err(EXIT_USER_ERROR);
            }
        },
    };

    // ---- execute ---------------------------------------------------------
    // The front end hands back the parsed program: a parameter sweep
    // re-elaborates once per point, which needs the definitions rather than
    // one already elaborated circuit.
    let outcome = {
        let mut backend = TheveninBackend::new();
        let request = RunRequest {
            program: &fe.program,
            experiment: &chosen.plan.name,
            // A file has no session variables, so there is nothing extra to
            // override: the experiment's own `param` statements are the whole
            // override chain.
            overrides: &[],
            limits: &circuit_core::Limits::default(),
        };
        match circuit_session::execute(&request, &mut backend, sources) {
            Ok(o) => o,
            Err(d) => {
                eprintln!("{}", d.render(sources));
                return Err(EXIT_USER_ERROR);
            }
        }
    };

    // ---- write -----------------------------------------------------------
    let format = match format {
        CliFormat::Csv => Format::Csv,
        CliFormat::Json => Format::Json,
        CliFormat::Both => Format::Both,
    };
    let mut refusal: Option<u8> = None;
    // The output view, not the raw solver grid: when the experiment asked for
    // an `output_interval:` these are the resampled traces (same file names).
    let written =
        write_datasets(
            out,
            format,
            &outcome.output_datasets,
            &mut |path| match guard_output(file, path) {
                Ok(()) => Ok(()),
                Err(code) => {
                    refusal = Some(code);
                    Err(Diagnostics::single(Diagnostic::error(
                        Code::Io,
                        format!("refusing to write `{}`", path.display()),
                    )))
                }
            },
        );
    if let Some(code) = refusal {
        return Err(code);
    }
    let written = match written {
        Ok(w) => w,
        Err(d) => {
            eprintln!("{}", d.render(sources));
            return Err(EXIT_USER_ERROR);
        }
    };

    // ---- summary ---------------------------------------------------------
    let overrides: Vec<String> = chosen
        .plan
        .param_overrides
        .iter()
        .map(|(name, value, _)| format!("{name}={}", value.value))
        .collect();
    print_summary(&outcome, &chosen.plan, &overrides, &written);
    Ok(())
}

fn print_summary(
    outcome: &circuit_session::RunOutcome,
    plan: &circuit_core::plan::AnalysisPlan,
    overrides: &[String],
    written: &Written,
) {
    let first: &Dataset = &outcome.datasets[0];
    println!(
        "experiment `{}` on circuit `{}` (backend {} {})",
        plan.name, plan.circuit_name, first.backend.name, first.backend.version
    );
    if !overrides.is_empty() {
        println!("  overrides: {}", overrides.join(", "));
    }
    for s in outcome.summaries() {
        println!("  {s}");
    }
    for w in &outcome.warnings {
        eprintln!("  warning: {w}");
    }
    // The warnings the renderer raised for the files just written (a non-finite
    // sample is an empty cell, not a number). They are printed in the same
    // shape as the run's own warnings, and `warning_lines` is shared with the
    // REPL so both front ends say the same thing about the same dataset.
    for line in written.warning_lines() {
        eprintln!("{line}");
    }
    for m in &outcome.measures {
        // The analysis identity is part of the value: the `max` of a transient
        // and the `max` of an AC sweep are different numbers even under one
        // name, so the name alone would not say what was measured.
        println!("  measure {}", m.render_with_analysis());
    }
    for path in &written.paths {
        println!("  wrote {}", path.display());
    }
}
