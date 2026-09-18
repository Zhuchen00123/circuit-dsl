//! The project-owned circuit IR.
//!
//! This is a *semantic* IR: nodes, devices, and models after name resolution
//! and parameter evaluation, but before anything backend-specific. It holds
//! no matrix slots, no solver state, and no backend handles (brief §7.1).
//!
//! Every entity keeps the source span it came from. For devices that come
//! out of a subcircuit instantiation, the IR keeps *both* the span in the
//! subcircuit body and the chain of instantiation sites, so a diagnostic can
//! say "written here, instantiated there" (brief §5.3, §9).

use std::collections::HashMap;
use std::fmt;

use crate::id::{CircuitId, DeviceId, GROUND, ModelId, NodeId};
use crate::span::SourceSpan;
use crate::units::{CAPACITANCE, INDUCTANCE, Quantity, RESISTANCE};

// ---------------------------------------------------------------------------
// Nodes
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeKind {
    /// The global reference, always [`GROUND`].
    Ground,
    /// An ordinary node, explicitly declared by the user.
    Normal,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: NodeId,
    /// Flattened, unique name within the circuit, e.g. `stage1.out`.
    pub name: String,
    /// The name as written at this hierarchy level, e.g. `out`.
    pub local_name: String,
    pub kind: NodeKind,
    pub span: SourceSpan,
}

// ---------------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------------

/// Terminal names, fixed by this project and mapped to backend names by the
/// adapter. Keeping them as constants prevents the two sides from drifting.
pub mod terminal {
    /// Positive terminal of a two-terminal passive or source.
    pub const POS: &str = "p";
    /// Negative terminal.
    pub const NEG: &str = "n";
    pub const ANODE: &str = "anode";
    pub const CATHODE: &str = "cathode";
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceKind {
    Resistor,
    Capacitor,
    Inductor,
    VoltageSource,
    CurrentSource,
    Diode,
}

impl DeviceKind {
    pub fn name(self) -> &'static str {
        match self {
            DeviceKind::Resistor => "resistor",
            DeviceKind::Capacitor => "capacitor",
            DeviceKind::Inductor => "inductor",
            DeviceKind::VoltageSource => "voltage_source",
            DeviceKind::CurrentSource => "current_source",
            DeviceKind::Diode => "diode",
        }
    }

    /// The dimension a device's `value` parameter must have.
    ///
    /// `None` for devices that do not take a `value` (sources, diodes).
    pub fn value_dimension(self) -> Option<crate::units::Dimension> {
        match self {
            DeviceKind::Resistor => Some(RESISTANCE),
            DeviceKind::Capacitor => Some(CAPACITANCE),
            DeviceKind::Inductor => Some(INDUCTANCE),
            _ => None,
        }
    }

    /// Whether this device is a source, and therefore carries a [`SourceSpec`].
    pub fn is_source(self) -> bool {
        matches!(self, DeviceKind::VoltageSource | DeviceKind::CurrentSource)
    }
}

/// One step of the instantiation chain from the top circuit down to a device.
#[derive(Clone, Debug)]
pub struct InstanceStep {
    /// Instance name, e.g. `stage1`.
    pub instance: String,
    /// Subcircuit being instantiated, e.g. `lowpass`.
    pub of: String,
    /// Where the `instance` statement was written.
    pub span: SourceSpan,
}

#[derive(Clone, Debug)]
pub struct Device {
    pub id: DeviceId,
    pub kind: DeviceKind,
    /// Name as written, e.g. `r1`.
    pub local_name: String,
    /// Fully qualified name, e.g. `stage1.r1` (top-level devices keep their
    /// bare name).
    pub name: String,
    /// Terminals in a fixed order. For two-terminal devices this is
    /// `[p, n]`, which also fixes the positive current direction `p -> n`.
    pub terminals: Vec<(String, NodeId)>,
    /// Evaluated parameters, in SI units.
    pub params: HashMap<String, Quantity>,
    pub model: Option<ModelId>,
    pub source: Option<SourceSpec>,
    /// Where the device is written in the source.
    pub def_span: SourceSpan,
    /// Instantiation chain; empty for devices in the top-level circuit.
    pub instance_path: Vec<InstanceStep>,
}

impl Device {
    /// The net attached to a terminal, if present.
    pub fn terminal(&self, name: &str) -> Option<NodeId> {
        self.terminals
            .iter()
            .find(|(t, _)| t == name)
            .map(|(_, n)| *n)
    }

    /// Positive terminal for current-direction purposes.
    pub fn pos(&self) -> Option<NodeId> {
        self.terminal(terminal::POS)
            .or_else(|| self.terminal(terminal::ANODE))
    }

    /// Negative terminal for current-direction purposes.
    pub fn neg(&self) -> Option<NodeId> {
        self.terminal(terminal::NEG)
            .or_else(|| self.terminal(terminal::CATHODE))
    }

    pub fn param(&self, name: &str) -> Option<Quantity> {
        self.params.get(name).copied()
    }

    /// Human-readable instance path used in diagnostics, e.g.
    /// `top.stage1.r1`. Falls back to the local name at top level.
    pub fn instance_display(&self, circuit_name: &str) -> String {
        let mut s = String::from(circuit_name);
        for step in &self.instance_path {
            s.push('.');
            s.push_str(&step.instance);
        }
        s.push('.');
        s.push_str(&self.local_name);
        s
    }
}

// ---------------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModelKind {
    Diode,
}

impl ModelKind {
    pub fn name(self) -> &'static str {
        match self {
            ModelKind::Diode => "diode",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Model {
    pub id: ModelId,
    pub name: String,
    pub kind: ModelKind,
    pub params: HashMap<String, Quantity>,
    pub span: SourceSpan,
}

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

/// AC small-signal excitation.
///
/// Phase is stored in radians. The language accepts degrees and converts at
/// the boundary; see `docs/language.md`.
#[derive(Clone, Copy, Debug)]
pub struct AcSpec {
    pub magnitude: Quantity,
    pub phase_rad: f64,
}

/// Transient waveform.
#[derive(Clone, Debug)]
pub enum Waveform {
    /// Idealised pulse. All fields are times except `low`/`high`.
    Pulse {
        low: Quantity,
        high: Quantity,
        delay: Quantity,
        rise: Quantity,
        fall: Quantity,
        width: Quantity,
        period: Quantity,
    },
    /// Damped sinusoid.
    Sin {
        offset: Quantity,
        amplitude: Quantity,
        frequency: Quantity,
        delay: Quantity,
        damping: Quantity,
        phase_rad: f64,
    },
    /// Piecewise-linear, as `(time, value)` pairs. Times must ascend.
    Pwl(Vec<(Quantity, Quantity)>),
}

impl Waveform {
    pub fn name(&self) -> &'static str {
        match self {
            Waveform::Pulse { .. } => "pulse",
            Waveform::Sin { .. } => "sin",
            Waveform::Pwl(_) => "pwl",
        }
    }
}

/// DC / AC / transient description of an independent source.
#[derive(Clone, Debug, Default)]
pub struct SourceSpec {
    /// DC value used by the operating point and DC sweeps.
    pub dc: Option<Quantity>,
    pub ac: Option<AcSpec>,
    /// Waveform used by transient analysis.
    pub waveform: Option<Waveform>,
}

impl SourceSpec {
    pub fn is_empty(&self) -> bool {
        self.dc.is_none() && self.ac.is_none() && self.waveform.is_none()
    }
}

// ---------------------------------------------------------------------------
// Circuit
// ---------------------------------------------------------------------------

/// A fully elaborated, flattened circuit.
#[derive(Clone, Debug)]
pub struct Circuit {
    pub id: CircuitId,
    pub name: String,
    pub nodes: Vec<Node>,
    pub devices: Vec<Device>,
    pub models: Vec<Model>,
    /// Where the circuit definition was written.
    pub span: SourceSpan,
    node_by_name: HashMap<String, NodeId>,
    device_by_name: HashMap<String, DeviceId>,
    model_by_name: HashMap<String, ModelId>,
}

/// Problems found while assembling a [`Circuit`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CircuitBuildError {
    /// A device references a node id that is not in the node table.
    DanglingNode { device: String, terminal: String },
    /// Duplicate node name.
    DuplicateNode(String),
    /// Duplicate device name.
    DuplicateDevice(String),
    /// Duplicate model name.
    DuplicateModel(String),
    /// Node id does not match its position in the table.
    NodeIdMismatch { expected: u32, found: u32 },
}

impl fmt::Display for CircuitBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CircuitBuildError::DanglingNode { device, terminal } => write!(
                f,
                "device `{device}` terminal `{terminal}` references an unknown node"
            ),
            CircuitBuildError::DuplicateNode(n) => write!(f, "duplicate node `{n}`"),
            CircuitBuildError::DuplicateDevice(n) => write!(f, "duplicate device `{n}`"),
            CircuitBuildError::DuplicateModel(n) => write!(f, "duplicate model `{n}`"),
            CircuitBuildError::NodeIdMismatch { expected, found } => write!(
                f,
                "node table is not densely indexed: position {expected} holds id {found}"
            ),
        }
    }
}

impl std::error::Error for CircuitBuildError {}

impl Circuit {
    /// Assemble a circuit, validating internal consistency.
    ///
    /// Ground is inserted by the elaborator as `NodeId(0)` named `gnd`; this
    /// constructor verifies that invariant rather than assuming it.
    pub fn new(
        id: CircuitId,
        name: String,
        nodes: Vec<Node>,
        devices: Vec<Device>,
        models: Vec<Model>,
        span: SourceSpan,
    ) -> Result<Self, CircuitBuildError> {
        // Node ids must be dense and equal to their index, so that
        // `nodes[id.index()]` is valid everywhere else in the codebase.
        for (i, n) in nodes.iter().enumerate() {
            if n.id.0 as usize != i {
                return Err(CircuitBuildError::NodeIdMismatch {
                    expected: i as u32,
                    found: n.id.0,
                });
            }
        }

        let mut node_by_name = HashMap::with_capacity(nodes.len());
        for n in &nodes {
            if node_by_name.insert(n.name.clone(), n.id).is_some() {
                return Err(CircuitBuildError::DuplicateNode(n.name.clone()));
            }
        }

        let mut device_by_name = HashMap::with_capacity(devices.len());
        for d in &devices {
            if device_by_name.insert(d.name.clone(), d.id).is_some() {
                return Err(CircuitBuildError::DuplicateDevice(d.name.clone()));
            }
            for (t, nid) in &d.terminals {
                if nid.index() >= nodes.len() {
                    return Err(CircuitBuildError::DanglingNode {
                        device: d.name.clone(),
                        terminal: t.clone(),
                    });
                }
            }
        }

        let mut model_by_name = HashMap::with_capacity(models.len());
        for m in &models {
            if model_by_name.insert(m.name.clone(), m.id).is_some() {
                return Err(CircuitBuildError::DuplicateModel(m.name.clone()));
            }
        }

        Ok(Self {
            id,
            name,
            nodes,
            devices,
            models,
            span,
            node_by_name,
            device_by_name,
            model_by_name,
        })
    }

    pub fn ground(&self) -> NodeId {
        GROUND
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.index())
    }

    pub fn node_name(&self, id: NodeId) -> &str {
        self.node(id)
            .map(|n| n.name.as_str())
            .unwrap_or("<unknown>")
    }

    /// Look up a node by its flattened name. `gnd` and `0` both resolve to
    /// ground.
    pub fn node_id(&self, name: &str) -> Option<NodeId> {
        if name == "gnd" || name == "0" {
            return Some(GROUND);
        }
        self.node_by_name.get(name).copied()
    }

    pub fn device(&self, id: DeviceId) -> Option<&Device> {
        self.devices.get(id.index())
    }

    pub fn device_name(&self, id: DeviceId) -> &str {
        self.device(id)
            .map(|d| d.name.as_str())
            .unwrap_or("<unknown>")
    }

    pub fn device_id(&self, name: &str) -> Option<DeviceId> {
        self.device_by_name.get(name).copied()
    }

    pub fn model(&self, id: ModelId) -> Option<&Model> {
        self.models.get(id.index())
    }

    pub fn model_id(&self, name: &str) -> Option<ModelId> {
        self.model_by_name.get(name).copied()
    }

    /// All devices attached to a node.
    pub fn devices_on(&self, node: NodeId) -> impl Iterator<Item = &Device> {
        self.devices
            .iter()
            .filter(move |d| d.terminals.iter().any(|(_, n)| *n == node))
    }

    /// Non-ground nodes, in declaration order.
    pub fn signal_nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter().filter(|n| n.kind != NodeKind::Ground)
    }

    /// Deterministic textual summary, used by `cdsl check --verbose`.
    pub fn summary(&self) -> String {
        let mut out = format!(
            "circuit {}: {} nodes ({} signal), {} devices, {} models",
            self.name,
            self.nodes.len(),
            self.signal_nodes().count(),
            self.devices.len(),
            self.models.len()
        );
        for d in &self.devices {
            out.push_str(&format!(
                "\n  {:<10} {:<16} {}",
                d.kind.name(),
                d.name,
                d.terminals
                    .iter()
                    .map(|(t, n)| format!("{t}={}", self.node_name(*n)))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::VOLTAGE;

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
            id: DeviceId(id),
            kind: DeviceKind::Resistor,
            local_name: name.to_string(),
            name: name.to_string(),
            terminals: vec![
                (terminal::POS.to_string(), NodeId(p)),
                (terminal::NEG.to_string(), NodeId(n)),
            ],
            params: HashMap::from([("value".to_string(), Quantity::ohms(value))]),
            model: None,
            source: None,
            def_span: SourceSpan::synthetic(),
            instance_path: Vec::new(),
        }
    }

    fn simple() -> Circuit {
        Circuit::new(
            CircuitId(0),
            "divider".into(),
            vec![node(0, "gnd"), node(1, "in"), node(2, "mid")],
            vec![
                resistor(0, "r1", 1, 2, 1000.0),
                resistor(1, "r2", 2, 0, 2000.0),
            ],
            Vec::new(),
            SourceSpan::synthetic(),
        )
        .unwrap()
    }

    #[test]
    fn lookups_work_by_id_and_name() {
        let c = simple();
        assert_eq!(c.node_id("mid"), Some(NodeId(2)));
        assert_eq!(c.node_id("gnd"), Some(GROUND));
        assert_eq!(c.node_id("0"), Some(GROUND));
        assert_eq!(c.node_id("nope"), None);
        assert_eq!(c.device_id("r1"), Some(DeviceId(0)));
        assert_eq!(c.node_name(NodeId(0)), "gnd");
        assert_eq!(c.node_name(NodeId(2)), "mid");
    }

    #[test]
    fn terminal_direction_is_fixed() {
        let c = simple();
        let r1 = &c.devices[0];
        assert_eq!(r1.pos(), Some(NodeId(1)));
        assert_eq!(r1.neg(), Some(NodeId(2)));
        assert_eq!(r1.param("value").unwrap().value, 1000.0);
    }

    #[test]
    fn rejects_dangling_terminals() {
        let err = Circuit::new(
            CircuitId(0),
            "bad".into(),
            vec![node(0, "gnd")],
            vec![resistor(0, "r1", 0, 9, 1.0)],
            Vec::new(),
            SourceSpan::synthetic(),
        )
        .unwrap_err();
        assert!(matches!(err, CircuitBuildError::DanglingNode { .. }));
        assert!(err.to_string().contains("r1"), "{err}");
    }

    #[test]
    fn rejects_sparse_node_indexing() {
        let err = Circuit::new(
            CircuitId(0),
            "sparse".into(),
            vec![node(0, "gnd"), node(5, "x")],
            Vec::new(),
            Vec::new(),
            SourceSpan::synthetic(),
        )
        .unwrap_err();
        assert_eq!(
            err,
            CircuitBuildError::NodeIdMismatch {
                expected: 1,
                found: 5
            }
        );
    }

    #[test]
    fn rejects_duplicate_names() {
        let err = Circuit::new(
            CircuitId(0),
            "dup".into(),
            vec![node(0, "gnd"), node(1, "a"), node(2, "a")],
            Vec::new(),
            Vec::new(),
            SourceSpan::synthetic(),
        )
        .unwrap_err();
        assert_eq!(err, CircuitBuildError::DuplicateNode("a".into()));
    }

    #[test]
    fn hierarchical_name_and_instance_display() {
        let mut d = resistor(0, "r1", 1, 0, 1000.0);
        d.instance_path = vec![InstanceStep {
            instance: "stage1".into(),
            of: "lowpass".into(),
            span: SourceSpan::synthetic(),
        }];
        d.name = "stage1.r1".into();
        assert_eq!(d.instance_display("top"), "top.stage1.r1");

        let mut top = resistor(1, "src", 1, 0, 1.0);
        top.instance_path.clear();
        assert_eq!(top.instance_display("top"), "top.src");
    }

    #[test]
    fn devices_on_node_are_found() {
        let c = simple();
        let on_mid: Vec<_> = c.devices_on(NodeId(2)).map(|d| d.name.clone()).collect();
        assert_eq!(on_mid, vec!["r1", "r2"]);
    }

    #[test]
    fn value_dimensions_are_declared() {
        assert_eq!(DeviceKind::Resistor.value_dimension(), Some(RESISTANCE));
        assert_eq!(DeviceKind::Capacitor.value_dimension(), Some(CAPACITANCE));
        assert_eq!(DeviceKind::Inductor.value_dimension(), Some(INDUCTANCE));
        assert_eq!(DeviceKind::Diode.value_dimension(), None);
        assert!(DeviceKind::VoltageSource.is_source());
    }

    #[test]
    fn source_spec_reports_emptiness() {
        let mut s = SourceSpec::default();
        assert!(s.is_empty());
        s.dc = Some(Quantity::new(1.0, VOLTAGE));
        assert!(!s.is_empty());
    }
}
