//! `cdsl check` and `cdsl capabilities`.

use std::path::Path;

use circuit_backend::SimulationBackend;
use circuit_backend::thevenin::TheveninBackend;
use circuit_core::{Limits, SourceMap};

use crate::{EXIT_USER_ERROR, read_source};

/// Load, parse, elaborate and backend-validate a file.
///
/// Returns the elaborated program and the source map that produced it, so the
/// caller can render diagnostics against real source text.
pub struct FrontEnd {
    pub sources: SourceMap,
    /// The parsed program, kept so a caller that needs the definitions (the
    /// parameter-sweep path re-elaborates per point) does not read and parse
    /// the file a second time.
    pub program: circuit_dsl::Program,
    pub compiled: circuit_dsl::Compiled,
}

/// Run the front end, printing diagnostics on failure.
pub fn front_end(file: &Path, verbose: bool) -> Result<FrontEnd, u8> {
    let text = read_source(file)?;

    let mut sources = SourceMap::new();
    let source_id = sources.add(file.display().to_string(), text.clone());

    let tokens = match circuit_dsl::lex(source_id, &text) {
        Ok(t) => t,
        Err(d) => {
            eprintln!("{}", d.render(&sources));
            return Err(EXIT_USER_ERROR);
        }
    };
    let program = match circuit_dsl::parse(&tokens) {
        Ok(p) => p,
        Err(d) => {
            eprintln!("{}", d.render(&sources));
            return Err(EXIT_USER_ERROR);
        }
    };
    let compiled = match circuit_dsl::compile(&program, &Limits::default()) {
        Ok(c) => c,
        Err(d) => {
            eprintln!("{}", d.render(&sources));
            return Err(EXIT_USER_ERROR);
        }
    };

    // Backend capability checking is part of `check`: the brief asks the
    // command to state whether it includes it, and this one does.
    let backend = TheveninBackend::new();
    let mut failed = false;
    for e in &compiled.experiments {
        if let Err(d) = backend.validate(&e.circuit, &e.plan) {
            eprintln!("{}", d.render(&sources));
            failed = true;
        }
    }
    if failed {
        return Err(EXIT_USER_ERROR);
    }

    if verbose {
        eprintln!("checked `{}` successfully", file.display());
    }

    Ok(FrontEnd {
        sources,
        program,
        compiled,
    })
}

pub fn check(file: &Path, json: bool, verbose: bool) -> Result<(), u8> {
    let fe = front_end(file, verbose)?;

    if json {
        println!("{}", circuit_json(&fe));
        return Ok(());
    }

    println!(
        "{}: {} circuit(s), {} experiment(s)",
        file.display(),
        fe.compiled.circuits.len(),
        fe.compiled.experiments.len()
    );
    for c in &fe.compiled.circuits {
        println!("  {}", c.summary());
    }
    for e in &fe.compiled.experiments {
        println!(
            "  experiment `{}` on `{}`: {} analysis task(s), {} measure(s)",
            e.plan.name,
            e.plan.circuit_name,
            e.plan.tasks.len(),
            e.plan.measures.len()
        );
        for t in &e.plan.tasks {
            let probes: Vec<&str> = t.probes.iter().map(|p| p.name.as_str()).collect();
            println!(
                "    - {:<5}{}",
                t.kind.name(),
                if probes.is_empty() {
                    String::new()
                } else {
                    format!(" save {}", probes.join(", "))
                }
            );
        }
    }
    Ok(())
}

/// A machine-readable dump of the elaborated design.
///
/// Hand-rolled rather than derived, because the IR deliberately does not
/// depend on `serde`; this keeping `circuit-core` free of a serialisation
/// choice is worth the few extra lines.
fn circuit_json(fe: &FrontEnd) -> String {
    use serde_json::{Value, json};

    let circuits: Vec<Value> = fe
        .compiled
        .circuits
        .iter()
        .map(|c| {
            json!({
                "name": c.name,
                "nodes": c.nodes.iter().map(|n| json!({
                    "id": n.id.0,
                    "name": n.name,
                    "kind": if n.kind == circuit_core::NodeKind::Ground { "gnd" } else { "signal" },
                })).collect::<Vec<_>>(),
                "devices": c.devices.iter().map(|d| json!({
                    "id": d.id.0,
                    "name": d.name,
                    "kind": d.kind.name(),
                    "terminals": d.terminals.iter()
                        .map(|(t, n)| json!([t, c.node_name(*n)]))
                        .collect::<Vec<_>>(),
                    "params": d.params.iter()
                        .map(|(k, v)| json!([k, v.value]))
                        .collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();

    let experiments: Vec<Value> = fe
        .compiled
        .experiments
        .iter()
        .map(|e| {
            json!({
                "name": e.plan.name,
                "circuit": e.plan.circuit_name,
                "analyses": e.plan.tasks.iter().map(|t| json!({
                    "kind": t.kind.name(),
                    "probes": t.probes.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "measures": e.plan.measures.iter()
                    .map(|m| json!([m.name, m.kind.name(), m.target_name]))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();

    let doc = json!({ "circuits": circuits, "experiments": experiments });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

pub fn capabilities(verbose: bool) {
    let backend = TheveninBackend::new();
    let caps = backend.capabilities();

    println!("backend: {} {}", caps.name, caps.version);
    println!("analyses: {}", caps.analyses.join(", "));
    println!("devices: {}", caps.devices.join(", "));
    println!(
        "source-value DC sweep: {}",
        if caps.source_sweep { "yes" } else { "no" }
    );
    println!(
        "parameter DC sweep: {}",
        if caps.parameter_sweep {
            "yes".to_string()
        } else {
            "yes (one elaboration per point, via the CLI)".to_string()
        }
    );
    for note in &caps.notes {
        println!("note: {note}");
    }

    if verbose {
        println!();
        println!("limits:");
        let l = backend.limits();
        println!("  max devices: {}", l.max_devices);
        println!("  max nodes: {}", l.max_nodes);
        println!("  max loop iterations: {}", l.max_loop_iterations);
        println!("  max subcircuit depth: {}", l.max_depth);
        println!("  max sweep points: {}", l.max_sweep_points);
        println!("  max result values: {}", l.max_result_values);
    }
}
