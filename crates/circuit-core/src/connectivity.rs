//! DC reference-path analysis.
//!
//! The brief (§9) warns that "floating node" cannot be decided by asking
//! whether *any* connection exists — a capacitor path is not a DC reference
//! path. Two different things happen in the engine, and neither is a usable
//! diagnostic (verified against the vendored source in this round):
//!
//! * A **linear** unreferenced network is solved directly and fails with an
//!   unlocated `matrix is singular, cannot solve` error that names no node and
//!   cannot distinguish a legal open load from a real floating network
//!   (`thevenin-0.5.0/src/simulate.rs:77-83`).
//! * Once a **non-linear** device is present the Newton path runs, and its
//!   diagonal-gmin stepping (`thevenin-0.5.0/src/newton.rs:361-364`, with
//!   `diag_gmin` forced to 0 for the operating point at
//!   `thevenin-0.5.0/src/simulate.rs:99-102`) can return `Ok` with a finite,
//!   gmin-dependent node voltage.
//!
//! Both measured in `docs/review-evidence/floating-audit.md` and
//! `docs/review-evidence/backend-contract.md`. The earlier blanket claim in
//! `docs/backend-evaluation.md` §4.6 ("the engine returns `Ok` and gmin keeps
//! the node finite") describes only the second case and was recorded without
//! that distinction. So the located, node-naming check lives here.
//!
//! # The rule
//!
//! Every non-ground node must be reachable from ground through devices that
//! conduct at DC:
//!
//! | device | conducts at DC? | why |
//! |---|---|---|
//! | resistor | yes | `v = iR`, including `i = 0` |
//! | inductor | yes | a short at DC |
//! | voltage source | yes | it fixes a potential difference |
//! | diode | yes | non-linear, but it carries a DC current |
//! | capacitor | **no** | an open circuit at DC |
//! | current source | **no** | it fixes a current, not a potential |
//!
//! A node that fails this is not "unconnected" — it may have several
//! capacitor terminals — but its operating point is undetermined.

use std::collections::VecDeque;

use crate::id::{DeviceId, GROUND, NodeId};
use crate::ir::{Circuit, DeviceKind, NodeKind};

/// Whether a device provides a DC conduction path between its terminals.
pub fn conducts_dc(kind: DeviceKind) -> bool {
    match kind {
        DeviceKind::Resistor
        | DeviceKind::Inductor
        | DeviceKind::VoltageSource
        | DeviceKind::Diode => true,
        DeviceKind::Capacitor | DeviceKind::CurrentSource => false,
    }
}

/// Why a node's operating point is not determined.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FloatingKind {
    /// No device terminal touches the node at all.
    Unused,
    /// The node has connections, but every path to ground is blocked by a
    /// device that does not conduct at DC.
    AcCoupledOnly,
}

/// A node whose DC operating point is undetermined, with the reason.
#[derive(Clone, Debug)]
pub struct FloatingNode {
    pub node: NodeId,
    pub kind: FloatingKind,
    /// The non-conducting devices attached to it, for the diagnostic.
    pub blocking: Vec<DeviceId>,
}

/// Find every node with no DC path to ground.
///
/// Nodes are visited in declaration order so the report is deterministic.
pub fn floating_nodes(circuit: &Circuit) -> Vec<FloatingNode> {
    let n = circuit.nodes.len();
    let mut reachable = vec![false; n];

    // Flood fill from ground along DC-conducting devices.
    let mut queue = VecDeque::new();
    let ground = GROUND.index();
    if ground < n {
        reachable[ground] = true;
        queue.push_back(ground);
    }
    while let Some(node) = queue.pop_front() {
        for device in circuit.devices_on(NodeId(node as u32)) {
            if !conducts_dc(device.kind) {
                continue;
            }
            for (_, other) in &device.terminals {
                let o = other.index();
                if o < n && !reachable[o] {
                    reachable[o] = true;
                    queue.push_back(o);
                }
            }
        }
    }

    let mut out = Vec::new();
    for node in &circuit.nodes {
        if node.kind == NodeKind::Ground || reachable[node.id.index()] {
            continue;
        }
        let attached: Vec<&crate::ir::Device> = circuit.devices_on(node.id).collect();
        if attached.is_empty() {
            out.push(FloatingNode {
                node: node.id,
                kind: FloatingKind::Unused,
                blocking: Vec::new(),
            });
            continue;
        }
        let blocking: Vec<DeviceId> = attached
            .iter()
            .filter(|d| !conducts_dc(d.kind))
            .map(|d| d.id)
            .collect();
        out.push(FloatingNode {
            node: node.id,
            kind: FloatingKind::AcCoupledOnly,
            blocking,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Device, Node, SourceSpec, terminal};
    use crate::span::SourceSpan;
    use crate::{CircuitId, ModelId};
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

    fn dev(id: u32, name: &str, kind: DeviceKind, p: u32, n: u32) -> Device {
        Device {
            id: DeviceId(id),
            kind,
            local_name: name.to_string(),
            name: name.to_string(),
            terminals: vec![
                (terminal::POS.to_string(), NodeId(p)),
                (terminal::NEG.to_string(), NodeId(n)),
            ],
            params: HashMap::new(),
            model: None,
            source: if kind.is_source() {
                Some(SourceSpec::default())
            } else {
                None
            },
            def_span: SourceSpan::synthetic(),
            instance_path: Vec::new(),
        }
    }

    fn circuit(nodes: Vec<Node>, devices: Vec<Device>) -> Circuit {
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
    fn conductive_devices_are_classified() {
        for k in [
            DeviceKind::Resistor,
            DeviceKind::Inductor,
            DeviceKind::VoltageSource,
            DeviceKind::Diode,
        ] {
            assert!(conducts_dc(k), "{k:?} should conduct at DC");
        }
        for k in [DeviceKind::Capacitor, DeviceKind::CurrentSource] {
            assert!(!conducts_dc(k), "{k:?} should not conduct at DC");
        }
    }

    #[test]
    fn a_resistive_divider_has_no_floating_nodes() {
        let c = circuit(
            vec![node(0, "gnd"), node(1, "in"), node(2, "mid")],
            vec![
                dev(0, "v1", DeviceKind::VoltageSource, 1, 0),
                dev(1, "r1", DeviceKind::Resistor, 1, 2),
                dev(2, "r2", DeviceKind::Resistor, 2, 0),
            ],
        );
        assert!(floating_nodes(&c).is_empty());
    }

    #[test]
    fn an_inductor_provides_a_dc_path() {
        let c = circuit(
            vec![node(0, "gnd"), node(1, "in"), node(2, "mid")],
            vec![
                dev(0, "v1", DeviceKind::VoltageSource, 1, 0),
                dev(1, "l1", DeviceKind::Inductor, 1, 2),
                dev(2, "r1", DeviceKind::Resistor, 2, 0),
            ],
        );
        assert!(floating_nodes(&c).is_empty());
    }

    /// The case the brief calls out: a capacitor path is not a DC path.
    #[test]
    fn a_capacitor_does_not_provide_a_dc_path() {
        let c = circuit(
            vec![node(0, "gnd"), node(1, "in"), node(2, "mid")],
            vec![
                dev(0, "v1", DeviceKind::VoltageSource, 1, 0),
                dev(1, "c1", DeviceKind::Capacitor, 1, 2),
            ],
        );
        let f = floating_nodes(&c);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].node, NodeId(2));
        assert_eq!(f[0].kind, FloatingKind::AcCoupledOnly);
        // The capacitor is named as the blocking device.
        assert_eq!(f[0].blocking, vec![DeviceId(1)]);
    }

    /// An AC-coupled stage with a bias resistor is fine: the resistor is the
    /// DC path even though the signal arrives through a capacitor.
    #[test]
    fn a_coupled_node_with_a_bias_resistor_is_fine() {
        let c = circuit(
            vec![node(0, "gnd"), node(1, "in"), node(2, "base")],
            vec![
                dev(0, "v1", DeviceKind::VoltageSource, 1, 0),
                dev(1, "c1", DeviceKind::Capacitor, 1, 2),
                dev(2, "rb", DeviceKind::Resistor, 2, 0),
            ],
        );
        assert!(floating_nodes(&c).is_empty());
    }

    #[test]
    fn a_current_source_does_not_anchor_a_node() {
        // A current source from ground into an isolated node leaves it
        // undefined: it sets a current, not a potential.
        let c = circuit(
            vec![node(0, "gnd"), node(1, "x")],
            vec![dev(0, "i1", DeviceKind::CurrentSource, 0, 1)],
        );
        let f = floating_nodes(&c);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, FloatingKind::AcCoupledOnly);
    }

    #[test]
    fn a_declared_but_unused_node_is_reported_separately() {
        let c = circuit(
            vec![node(0, "gnd"), node(1, "in"), node(2, "spare")],
            vec![
                dev(0, "v1", DeviceKind::VoltageSource, 1, 0),
                dev(1, "r1", DeviceKind::Resistor, 1, 0),
            ],
        );
        let f = floating_nodes(&c);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, FloatingKind::Unused);
        assert!(f[0].blocking.is_empty());
    }

    #[test]
    fn a_diode_anchors_a_node() {
        // A node held only by a diode is non-linear but still determined.
        let c = circuit(
            vec![node(0, "gnd"), node(1, "in"), node(2, "out")],
            vec![
                dev(0, "v1", DeviceKind::VoltageSource, 1, 0),
                dev(1, "r1", DeviceKind::Resistor, 1, 2),
                dev(2, "d1", DeviceKind::Diode, 2, 0),
            ],
        );
        let mut c = c;
        c.devices[2].model = Some(ModelId(0));
        assert!(floating_nodes(&c).is_empty());
    }

    #[test]
    fn ground_itself_is_never_reported() {
        let c = circuit(vec![node(0, "gnd")], Vec::new());
        assert!(floating_nodes(&c).is_empty());
    }
}
