//! Parameter sweep driver.
//!
//! The engine has no way to sweep a *parameter*: its `.dc` analysis varies a
//! source's value. Sweeping a parameter such as `r` changes a device value, so
//! the only correct way to run it is to re-evaluate the design at each point
//! and simulate that point.
//!
//! That is what this module does, and it enforces the rule from brief §5.4
//! while doing it: **a parameter sweep may not change the topology**. After
//! each point is elaborated, the resulting circuit is compared against the
//! first point's; if the node set, device set, device kinds, or terminal
//! connections differ, the sweep is rejected with a diagnostic naming the
//! first point at which they diverged. That turns "topology parameter" from a
//! label the user has to trust into a property that is actually checked.

// See the note in `circuit-dsl`: `Diagnostic` is the shared error type and is
// intentionally not boxed.
#![allow(clippy::result_large_err)]

use circuit_core::diagnostic::{Code, Diagnostic, Diagnostics};
use circuit_core::ir::Circuit;
use circuit_core::plan::{AnalysisPlan, Sweep};
use circuit_core::{DeviceId, NodeId};

use crate::backend::{SimulationBackend, SimulationResults};

/// A device as the topology comparison sees it: name, kind, and its sorted
/// terminal-to-net binding.
type DeviceSignature = (String, String, Vec<(String, u32)>);

/// The topology of a circuit, used to prove a sweep did not change it.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Topology {
    nodes: Vec<(String, bool)>,
    devices: Vec<DeviceSignature>,
}

impl Topology {
    fn of(circuit: &Circuit) -> Self {
        let nodes = circuit
            .nodes
            .iter()
            .map(|n| (n.name.clone(), n.kind == circuit_core::ir::NodeKind::Ground))
            .collect();

        let mut devices: Vec<_> = circuit
            .devices
            .iter()
            .map(|d| {
                // Sorted terminals so declaration order cannot masquerade as a
                // topology change.
                let mut terms: Vec<(String, u32)> =
                    d.terminals.iter().map(|(t, n)| (t.clone(), n.0)).collect();
                terms.sort();
                (d.name.clone(), d.kind.name().to_string(), terms)
            })
            .collect();
        devices.sort();

        Self { nodes, devices }
    }

    /// A human-readable description of the first difference, if any.
    fn first_difference(&self, other: &Self) -> Option<String> {
        if self.nodes != other.nodes {
            let a: Vec<_> = self.nodes.iter().map(|(n, _)| n.as_str()).collect();
            let b: Vec<_> = other.nodes.iter().map(|(n, _)| n.as_str()).collect();
            for name in &a {
                if !b.contains(name) {
                    return Some(format!("node `{name}` is not present at this point"));
                }
            }
            for name in &b {
                if !a.contains(name) {
                    return Some(format!("node `{name}` appears only at this point"));
                }
            }
            return Some("the node set differs".to_string());
        }

        if self.devices != other.devices {
            let a: std::collections::HashSet<_> =
                self.devices.iter().map(|(n, _, _)| n.as_str()).collect();
            let b: std::collections::HashSet<_> =
                other.devices.iter().map(|(n, _, _)| n.as_str()).collect();
            for name in &a {
                if !b.contains(name) {
                    return Some(format!("device `{name}` is not present at this point"));
                }
            }
            for name in &b {
                if !a.contains(name) {
                    return Some(format!("device `{name}` appears only at this point"));
                }
            }
            for (x, y) in self.devices.iter().zip(&other.devices) {
                if x.0 == y.0 && x.1 != y.1 {
                    return Some(format!(
                        "device `{}` changes kind from {} to {}",
                        x.0, x.1, y.1
                    ));
                }
                if x.0 == y.0 && x.2 != y.2 {
                    return Some(format!(
                        "device `{}` is rewired (terminals {x:?} -> {y:?})",
                        x.0
                    ));
                }
            }
            return Some("the device set differs".to_string());
        }

        None
    }
}

/// What a completed parameter sweep produced, in sweep order.
#[derive(Clone, Debug)]
pub struct SweepOutcome {
    /// The swept coordinate for each point, in order.
    pub coordinates: Vec<f64>,
    /// Results for each point; `results[i]` corresponds to `coordinates[i]`.
    pub results: Vec<SimulationResults>,
}

/// Why a sweep could not be completed.
#[derive(Clone, Debug)]
pub enum SweepError {
    /// Diagnostics from elaboration or simulation. Rendered by the CLI.
    Diagnostics(Box<Diagnostics>),
}

impl From<Diagnostics> for SweepError {
    fn from(d: Diagnostics) -> Self {
        SweepError::Diagnostics(Box::new(d))
    }
}

impl std::fmt::Display for SweepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SweepError::Diagnostics(d) => write!(f, "{}", d.render_plain()),
        }
    }
}

impl std::error::Error for SweepError {}

/// Generate the coordinate values a sweep visits.
///
/// Endpoint handling is explicit: `stop` is included only when it lands on the
/// grid, except that a grid which would otherwise exclude it entirely still
/// stops before it. This matches the rule stated in `docs/language.md` §5.1.
pub fn sweep_coordinates(sweep: &Sweep) -> Result<Vec<f64>, Diagnostics> {
    let (start, stop) = (sweep.start, sweep.stop);
    let step = match sweep.step {
        Some(s) => s,
        None => {
            return Err(Diagnostics::single(
                Diagnostic::error(
                    Code::Sweep,
                    "a linear sweep needs a step; use `step:` or sweep a parameter with points",
                )
                .at(sweep.span),
            ));
        }
    };

    if step == 0.0 {
        return Err(Diagnostics::single(
            Diagnostic::error(Code::Sweep, "sweep step must not be zero").at(sweep.span),
        ));
    }
    if (stop - start).signum() != step.signum() {
        return Err(Diagnostics::single(
            Diagnostic::error(
                Code::Sweep,
                format!(
                    "sweep goes from {start} to {stop} but the step has the opposite sign ({step})"
                ),
            )
            .at(sweep.span),
        ));
    }

    let mut out = Vec::new();
    let mut x = start;
    // Guard against a step so small that the loop would not terminate in
    // reasonable time; the caller's limit check reports the real bound.
    while (step > 0.0 && x <= stop + step.abs() * 1e-9)
        || (step < 0.0 && x >= stop - step.abs() * 1e-9)
    {
        out.push(x);
        if out.len() > 10_000_000 {
            return Err(Diagnostics::single(
                Diagnostic::error(Code::Limit, "sweep generated too many points").at(sweep.span),
            ));
        }
        x += step;
    }
    Ok(out)
}

/// Run a parameter sweep by elaborating and simulating each point.
///
/// `build` produces a fresh `(Circuit, AnalysisPlan)` for a given parameter
/// value. It is called once per point, so the caller is responsible for
/// applying the override; this function's job is to enforce that doing so did
/// not change the topology, and to collect the results in order.
///
/// The sweep stops at the first failing point and reports it, rather than
/// carrying on and silently producing a partial dataset (brief §8.2).
pub fn run_parameter_sweep<B, F>(
    backend: &mut B,
    sweep: &Sweep,
    coordinates: &[f64],
    mut build: F,
) -> Result<SweepOutcome, SweepError>
where
    B: SimulationBackend,
    F: FnMut(f64) -> Result<(Circuit, AnalysisPlan), Diagnostics>,
{
    let mut results = Vec::with_capacity(coordinates.len());
    let mut reference: Option<Topology> = None;
    let mut first: Option<(f64, Circuit, AnalysisPlan)> = None;

    for (index, &value) in coordinates.iter().enumerate() {
        let (circuit, plan) = build(value)?;

        if let Some(reference) = &reference {
            let here = Topology::of(&circuit);
            if let Some(detail) = reference.first_difference(&here) {
                let (ref_value, _, _) = first.as_ref().expect("reference implies a first point");
                return Err(Diagnostics::single(
                    Diagnostic::error(
                        Code::TopologyParam,
                        format!("sweeping this parameter changes the circuit topology at {value}"),
                    )
                    .at(sweep.span)
                    .with_context("swept", sweep_target_name(sweep))
                    .with_context("first differing point", format!("{value}"))
                    .with_note(detail)
                    .with_note(format!(
                        "the reference topology came from the point at {ref_value}; \
                         topology-affecting parameters cannot be swept"
                    )),
                )
                .into());
            }
        } else {
            reference = Some(Topology::of(&circuit));
            first = Some((value, circuit.clone(), plan.clone()));
        }

        let outcome = backend.run(&circuit, &plan).map_err(|d| {
            let mut d = d;
            d.push(
                Diagnostic::error(Code::Backend, format!("sweep point {} failed", index + 1))
                    .at(sweep.span)
                    .with_context("swept", sweep_target_name(sweep))
                    .with_context("value", format!("{value}")),
            );
            SweepError::from(d)
        })?;

        results.push(outcome);
    }

    Ok(SweepOutcome {
        coordinates: coordinates.to_vec(),
        results,
    })
}

fn sweep_target_name(sweep: &Sweep) -> String {
    match &sweep.target {
        circuit_core::SweepTarget::SourceValue { name, .. } => format!("source `{name}`"),
        circuit_core::SweepTarget::Parameter { name } => format!("parameter `{name}`"),
    }
}

/// Unused-import guard: these are part of the public vocabulary of a sweep.
#[allow(dead_code)]
fn _type_check(_: DeviceId, _: NodeId) {}

#[cfg(test)]
mod tests {
    use super::*;
    use circuit_core::plan::SweepTarget;
    use circuit_core::span::SourceSpan;

    fn sweep_of(start: f64, stop: f64, step: f64) -> Sweep {
        Sweep {
            target: SweepTarget::Parameter { name: "r".into() },
            start,
            stop,
            dimension: circuit_core::units::RESISTANCE,
            step: Some(step),
            points: None,
            kind: circuit_core::SweepKind::Linear,
            include_endpoint: true,
            span: SourceSpan::synthetic(),
        }
    }

    #[test]
    fn coordinates_include_both_ends_when_aligned() {
        let c = sweep_coordinates(&sweep_of(0.0, 5.0, 1.0)).unwrap();
        assert_eq!(c, vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn coordinates_stop_before_an_unaligned_end() {
        let c = sweep_coordinates(&sweep_of(0.0, 5.0, 2.0)).unwrap();
        assert_eq!(c, vec![0.0, 2.0, 4.0]);
    }

    #[test]
    fn descending_sweeps_work() {
        let c = sweep_coordinates(&sweep_of(5.0, 0.0, -1.0)).unwrap();
        assert_eq!(c, vec![5.0, 4.0, 3.0, 2.0, 1.0, 0.0]);
    }

    #[test]
    fn a_zero_step_is_rejected() {
        let err = sweep_coordinates(&sweep_of(0.0, 1.0, 0.0)).unwrap_err();
        assert_eq!(err.iter().next().unwrap().code, Code::Sweep);
        assert!(err.render_plain().contains("must not be zero"));
    }

    #[test]
    fn a_backwards_step_is_rejected() {
        let err = sweep_coordinates(&sweep_of(0.0, 5.0, -1.0)).unwrap_err();
        assert!(
            err.render_plain().contains("opposite sign"),
            "{}",
            err.render_plain()
        );
    }

    #[test]
    fn a_missing_step_is_rejected_with_advice() {
        let mut s = sweep_of(0.0, 5.0, 1.0);
        s.step = None;
        let err = sweep_coordinates(&s).unwrap_err();
        let text = err.render_plain();
        assert!(text.contains("needs a step"), "{text}");
    }

    // ---- topology comparison -------------------------------------------

    use circuit_core::ir::{Device, DeviceKind, InstanceStep, Node, NodeKind};
    use circuit_core::units::Quantity;
    use circuit_core::{CircuitId, DeviceId as Did};
    use std::collections::HashMap;

    fn node(id: u32, name: &str) -> Node {
        Node {
            id: NodeId(id),
            name: name.to_string(),
            local_name: name.to_string(),
            kind: if id == 0 {
                NodeKind::Ground
            } else {
                NodeKind::Normal
            },
            span: SourceSpan::synthetic(),
        }
    }

    fn resistor(id: u32, name: &str, p: u32, n: u32, value: f64) -> Device {
        Device {
            id: Did(id),
            kind: DeviceKind::Resistor,
            local_name: name.to_string(),
            name: name.to_string(),
            terminals: vec![("p".to_string(), NodeId(p)), ("n".to_string(), NodeId(n))],
            params: HashMap::from([("value".to_string(), Quantity::ohms(value))]),
            model: None,
            source: None,
            def_span: SourceSpan::synthetic(),
            instance_path: Vec::<InstanceStep>::new(),
        }
    }

    fn circuit_with(devices: Vec<Device>, nodes: Vec<Node>) -> Circuit {
        Circuit::new(
            CircuitId(0),
            "t".into(),
            nodes,
            devices,
            Vec::new(),
            SourceSpan::synthetic(),
        )
        .unwrap()
    }

    #[test]
    fn changing_only_a_value_keeps_the_topology() {
        let a = circuit_with(
            vec![resistor(0, "r1", 1, 0, 1000.0)],
            vec![node(0, "gnd"), node(1, "a")],
        );
        let b = circuit_with(
            vec![resistor(0, "r1", 1, 0, 2000.0)],
            vec![node(0, "gnd"), node(1, "a")],
        );
        assert!(
            Topology::of(&a)
                .first_difference(&Topology::of(&b))
                .is_none()
        );
    }

    #[test]
    fn adding_a_device_is_a_topology_change() {
        let a = circuit_with(
            vec![resistor(0, "r1", 1, 0, 1000.0)],
            vec![node(0, "gnd"), node(1, "a")],
        );
        let b = circuit_with(
            vec![
                resistor(0, "r1", 1, 0, 1000.0),
                resistor(1, "r2", 1, 0, 1.0),
            ],
            vec![node(0, "gnd"), node(1, "a")],
        );
        let detail = Topology::of(&a)
            .first_difference(&Topology::of(&b))
            .unwrap();
        assert!(detail.contains("r2"), "{detail}");
    }

    #[test]
    fn rewiring_is_a_topology_change() {
        let a = circuit_with(
            vec![resistor(0, "r1", 1, 0, 1000.0)],
            vec![node(0, "gnd"), node(1, "a")],
        );
        let b = circuit_with(
            // same names and kinds, but r1 now bridges a to a second node
            vec![resistor(0, "r1", 1, 2, 1000.0)],
            vec![node(0, "gnd"), node(1, "a"), node(2, "b")],
        );
        let detail = Topology::of(&a)
            .first_difference(&Topology::of(&b))
            .unwrap();
        // The node set changes first, which is reported.
        assert!(detail.contains("b") || detail.contains("node"), "{detail}");
    }

    #[test]
    fn changing_a_device_kind_is_a_topology_change() {
        let a = circuit_with(
            vec![resistor(0, "x", 1, 0, 1000.0)],
            vec![node(0, "gnd"), node(1, "a")],
        );
        let mut dev = resistor(0, "x", 1, 0, 1000.0);
        dev.kind = DeviceKind::Capacitor;
        let b = circuit_with(vec![dev], vec![node(0, "gnd"), node(1, "a")]);
        let detail = Topology::of(&a)
            .first_difference(&Topology::of(&b))
            .unwrap();
        assert!(detail.contains("kind"), "{detail}");
    }
}
