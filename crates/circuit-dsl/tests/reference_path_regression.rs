//! Product-path regression tests for the DC reference-path rule.
//!
//! Everything here is driven through the real front end exactly as `cdsl check`
//! drives it: source text -> `lex` -> `parse` -> `compile`. No AST is built by
//! hand and no internal helper is called to decide the outcome, so a change to
//! the grammar, to elaboration, or to the connectivity rule has to show up as a
//! different accept/reject verdict right here.
//!
//! # The two circuits that must not be confused
//!
//! * A **legal open output** -- a node attached to a device that conducts at DC
//!   (r1, l1, v1, d1) -- is an ordinary, well-posed operating point (its branch
//!   current is simply zero). The front end must accept it. Re-measured for this
//!   round with an out-of-repo harness modelled on _probe/src/bin/robustness.rs
//!   (F:/tmp/probe-check): the backend solves this exact topology to
//!   v(a) = v(b) = 1.0 V exactly with v1#branch = 0.0, and the answer is
//!   bit-identical for .options GMIN = 1e-12, 1e-3 and 1.0 and with GMIN stepping
//!   disabled. It is not a node dragged to zero by a floor conductance; it is a
//!   genuine open circuit.
//! * A **network with no DC reference** -- a connected component that never
//!   reaches ground, or a node whose every path to ground is blocked by a
//!   capacitor or a current source -- has an *undefined* operating point and
//!   must be rejected by the front end with a diagnostic that names the node.
//!
//! The backend cannot draw that line for us. A truly unreferenced linear network
//! (r1 between a and b, no terminal on ground) still answers with exactly
//! "simulation failed: failed to solve MNA system: matrix is singular, cannot
//! solve" -- no node name, no location, and no way to tell it apart from a legal
//! open output, and no GMIN setting rescues it. The old "gmin keeps a floating
//! node finite" explanation of that behaviour does not hold on the linear
//! operating-point path measured above. So circuit_core::connectivity (a
//! reachability flood fill from ground along DC-conducting devices) is the only
//! thing between a user and an unactionable solver error, and these tests pin its
//! observable behaviour.
//!
//! # Keeping this file honest
//!
//! Every test names, in a comment, the wrong implementation it blocks. The
//! assertions are two-sided: the reject cases are paired with accept cases built
//! from the *same* device set, so a rule that simply refuses more (or simply
//! accepts more) cannot pass all of them. The failure direction was checked by
//! running an inverted copy of this file outside the repository, where
//! `cargo test` exits non-zero; see the round report.

use circuit_core::diagnostic::Code;
use circuit_core::ir::{DeviceKind, NodeKind};
use circuit_core::{Limits, SourceMap};
use circuit_dsl::{Compiled, compile, lex, parse};

// ---------------------------------------------------------------------------
// Harness: the real front end, and assertions with readable failures
// ---------------------------------------------------------------------------

/// What the front end did with one source text.
enum Outcome {
    /// `compile` returned `Ok`.
    Accepted(Compiled),
    /// Lexing, parsing or elaboration failed.
    Rejected(Rejected),
}

/// A failure, with the rendered diagnostics and the codes they carry.
struct Rejected {
    text: String,
    codes: Vec<Code>,
}

/// Run source text through the real pipeline.
fn run(src: &str) -> Outcome {
    let mut sm = SourceMap::new();
    let id = sm.add("reference_path_regression.cdsl", src);

    let tokens = match lex(id, src) {
        Ok(t) => t,
        Err(d) => {
            return Outcome::Rejected(Rejected {
                text: d.render(&sm),
                codes: d.errors().map(|e| e.code).collect(),
            });
        }
    };
    let program = match parse(&tokens) {
        Ok(p) => p,
        Err(d) => {
            return Outcome::Rejected(Rejected {
                text: d.render(&sm),
                codes: d.errors().map(|e| e.code).collect(),
            });
        }
    };
    match compile(&program, &Limits::default()) {
        Ok(c) => Outcome::Accepted(c),
        Err(d) => Outcome::Rejected(Rejected {
            text: d.render(&sm),
            codes: d.errors().map(|e| e.code).collect(),
        }),
    }
}

/// Compile and require the front end to accept the circuit.
fn accepted(src: &str) -> Compiled {
    match run(src) {
        Outcome::Accepted(c) => c,
        Outcome::Rejected(r) => panic!(
            "expected the front end to accept this circuit, but it was rejected:\n{}",
            r.text
        ),
    }
}

/// Compile and require the front end to reject the circuit.
fn rejected(src: &str) -> Rejected {
    match run(src) {
        Outcome::Rejected(r) => r,
        Outcome::Accepted(_) => {
            panic!("expected the front end to reject this circuit, but compilation succeeded")
        }
    }
}

/// Assert the rendered diagnostics mention a phrase; print the whole text on
/// failure so a regression is diagnosable without re-running under a debugger.
fn assert_mentions(r: &Rejected, needle: &str) {
    assert!(
        r.text.contains(needle),
        "expected the diagnostic to contain {needle:?}, but it was:\n{}",
        r.text
    );
}

/// Assert the rendered diagnostics do **not** mention a phrase.
fn assert_does_not_mention(r: &Rejected, needle: &str) {
    assert!(
        !r.text.contains(needle),
        "the diagnostic must not contain {needle:?}, but it was:\n{}",
        r.text
    );
}

/// Assert how many errors were reported and that every one of them is a
/// reference-path error.
///
/// The front end has no dedicated connectivity code: a node with no DC path to
/// ground is reported as E_NAME, the code for "this name does not denote a
/// usable thing", followed by the message that says what is actually wrong.
/// Pinning the count and the code stops a future change from reporting one node
/// twice, or from reporting the problem under a code that callers filter out.
fn assert_name_errors(r: &Rejected, expected: usize) {
    assert_eq!(
        r.codes.len(),
        expected,
        "expected {expected} error(s), got {:?}:\n{}",
        r.codes,
        r.text
    );
    for code in &r.codes {
        assert_eq!(
            *code,
            Code::Name,
            "expected only E_NAME diagnostics, found {code}:\n{}",
            r.text
        );
    }
    assert!(r.text.contains("error[E_NAME]"), "{}", r.text);
}

// ---------------------------------------------------------------------------
// 1. A legal open output is accepted
// ---------------------------------------------------------------------------

/// Node b is held up only by r1, which carries no current. That is an open
/// output, not a floating node: b is still DC-referenced to ground through
/// r1 -> a -> v1 -> gnd.
///
/// Blocks: any implementation that treats "no device carrying current" or
/// "terminal count is one" as floating, and any implementation that prunes the
/// dangling node out of the IR instead of keeping it.
#[test]
fn a_legal_open_output_is_accepted() {
    let c = accepted(
        r#"
circuit :open do
  node :a, :b
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  resistor :r1, p: :a, n: :b, value: 1.kohm
end
"#,
    );

    assert_eq!(c.circuits.len(), 1);
    let circuit = c.circuit("open").expect("circuit :open must be elaborated");
    // gnd + a + b, and nothing was dropped on the way in.
    assert_eq!(circuit.nodes.len(), 3, "{}", circuit.summary());
    assert_eq!(circuit.devices.len(), 2, "{}", circuit.summary());

    let a = circuit.node_id("a").expect("node a must exist in the IR");
    let b = circuit.node_id("b").expect("node b must exist in the IR");
    assert_ne!(a, circuit.ground());
    assert_ne!(b, circuit.ground());
    // b is a declared signal node, not silently folded into ground.
    assert_eq!(circuit.node(b).unwrap().kind, NodeKind::Normal);

    // r1 really joins a and b, so b's only attachment is a DC-conducting one.
    let r1 = circuit
        .device(circuit.device_id("r1").unwrap())
        .expect("device r1");
    assert_eq!(r1.kind, DeviceKind::Resistor);
    assert_eq!(r1.pos(), Some(a));
    assert_eq!(r1.neg(), Some(b));
    assert_eq!(r1.param("value").unwrap().value, 1000.0);
    let on_b: Vec<&str> = circuit.devices_on(b).map(|d| d.name.as_str()).collect();
    assert_eq!(on_b, vec!["r1"]);

    // ... and the far end of that path is the source, whose return terminal is
    // ground. That is precisely why b is referenced.
    let v1 = circuit
        .device(circuit.device_id("v1").unwrap())
        .expect("device v1");
    assert_eq!(v1.pos(), Some(a));
    assert_eq!(v1.neg(), Some(circuit.ground()));
    assert_eq!(v1.source.as_ref().unwrap().dc.unwrap().value, 1.0);
}

// ---------------------------------------------------------------------------
// 2. An isolated resistor network is rejected
// ---------------------------------------------------------------------------

/// Two nodes joined to each other and to nothing else. Every terminal is
/// connected, so a "does anything touch this node?" test would pass it -- but
/// the whole component floats at an arbitrary potential, and the backend answers
/// with a singular matrix that names no node.
///
/// Blocks: the seductive bug of deciding "floating" by counting terminals
/// instead of walking a DC path to ground, and the regression of reporting only
/// the first offending node.
#[test]
fn an_isolated_resistor_network_is_rejected() {
    let r = rejected(
        r#"
circuit :island do
  node :a, :b
  resistor :r1, p: :a, n: :b, value: 1.kohm
end
"#,
    );

    // Both ends of the island are named: the user must not have to guess which
    // node the solver disliked.
    assert_mentions(&r, "node `a` has no DC path to ground");
    assert_mentions(&r, "node `b` has no DC path to ground");
    // The reason is the missing reference, not a missing connection: r1 does
    // connect to both nodes. This negative assertion is what separates the two
    // messages the elaborator can emit.
    assert_does_not_mention(&r, "nothing connects to it");
    assert_name_errors(&r, 2);
}

// ---------------------------------------------------------------------------
// 3. A node coupled only through a capacitor is rejected
// ---------------------------------------------------------------------------

/// Node out reaches in through c1, and in is properly referenced. A capacitor is
/// an open circuit at DC, so out has no operating point: two different solvers
/// would give two different numbers, both consistent with the network.
///
/// Blocks: the classic "a capacitor path is a connection" bug, and any rule that
/// stops at terminal counting ("out has one terminal, so it is used").
#[test]
fn a_node_coupled_only_through_a_capacitor_is_rejected() {
    let r = rejected(
        r#"
circuit :ac_coupled do
  node :in, :out
  voltage_source :v1, p: :in, n: :gnd, dc: 1.V
  resistor :r1, p: :in, n: :gnd, value: 1.kohm
  capacitor :c1, p: :in, n: :out, value: 1.uF
end
"#,
    );

    assert_mentions(&r, "node `out` has no DC path to ground");
    // The blocking device is named, so the fix ("add a resistor to ground") is
    // obvious from the message alone.
    assert_mentions(&r, "attached but not conducting at DC: c1");
    // Node in is referenced twice over (v1 and r1); an implementation that
    // rejects a whole circuit as soon as one node fails would also pass the two
    // assertions above, and this one stops it.
    assert_does_not_mention(&r, "node `in` has no DC path to ground");
    assert_name_errors(&r, 1);
}

// ---------------------------------------------------------------------------
// 4. Counter-example guard: the circuit from the brief is legal
// ---------------------------------------------------------------------------

/// This is the circuit the task brief described as "coupled only through a
/// capacitor": v1 in->gnd, r1 in->out, c1 out->gnd.
///
/// It is **not** AC-coupled-only, and it must not be "fixed" by anyone reading
/// test 3 as a template: out is reached from ground by v1 -> in -> r1 -> out, an
/// unbroken DC path made of devices that conduct at DC. c1 merely shunts the
/// output at AC. The two circuits differ only in where r1 and c1 are attached:
///
///   test 3 (rejected)                test 4 (accepted)
///     v1: in -> gnd                    v1: in -> gnd
///     r1: in -> gnd                    r1: in -> out    <- the DC path to out
///     c1: in -> out   <- out hangs     c1: out -> gnd   <- the AC load
///
/// Blocks: over-correction of test 3 -- a rule that rejects any node with a
/// capacitor attached, and a rule that only accepts the exact topologies already
/// in the test suite.
#[test]
fn the_same_devices_with_the_capacitor_to_ground_are_accepted() {
    let c = accepted(
        r#"
circuit :rc_lowpass do
  node :in, :out
  voltage_source :v1, p: :in, n: :gnd, dc: 1.V
  resistor :r1, p: :in, n: :out, value: 1.kohm
  capacitor :c1, p: :out, n: :gnd, value: 1.uF
end
"#,
    );

    let circuit = c.circuit("rc_lowpass").expect("circuit :rc_lowpass");
    assert_eq!(circuit.nodes.len(), 3, "{}", circuit.summary());
    assert_eq!(circuit.devices.len(), 3, "{}", circuit.summary());

    let gnd = circuit.ground();
    let input = circuit.node_id("in").expect("node in");
    let out = circuit.node_id("out").expect("node out");

    let r1 = circuit.device(circuit.device_id("r1").unwrap()).unwrap();
    assert_eq!(r1.kind, DeviceKind::Resistor);
    assert_eq!(r1.pos(), Some(input));
    assert_eq!(r1.neg(), Some(out));

    let c1 = circuit.device(circuit.device_id("c1").unwrap()).unwrap();
    assert_eq!(c1.kind, DeviceKind::Capacitor);
    assert_eq!(c1.pos(), Some(out));
    assert_eq!(c1.neg(), Some(gnd));

    let v1 = circuit.device(circuit.device_id("v1").unwrap()).unwrap();
    assert_eq!(v1.pos(), Some(input));
    assert_eq!(v1.neg(), Some(gnd));

    // out carries the resistor and the capacitor, and the resistor side is the
    // one that conducts at DC -- the fact the rule under test has to get right.
    let on_out: Vec<&str> = circuit.devices_on(out).map(|d| d.name.as_str()).collect();
    assert_eq!(on_out, vec!["r1", "c1"]);
}

// ---------------------------------------------------------------------------
// 5. A declared but unconnected node is reported separately
// ---------------------------------------------------------------------------

/// Node spare has no terminal on it at all. That is a different mistake from a
/// node that is wired up but unreferenced, and the user gets a different,
/// actionable message ("remove the declaration").
///
/// Blocks: collapsing the two reasons into one message -- which would tell a
/// user to "add a resistor to ground" for a node nothing can attach to.
#[test]
fn a_declared_but_unconnected_node_is_reported_separately() {
    let r = rejected(
        r#"
circuit :spare_node do
  node :a, :spare
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  resistor :r1, p: :a, n: :gnd, value: 1.kohm
end
"#,
    );

    assert_mentions(&r, "node `spare` is declared but nothing connects to it");
    // The rest of the circuit is fine, so there is exactly one error, and it is
    // not the "no DC path" message: spare never had a path to lose.
    assert_does_not_mention(&r, "no DC path to ground");
    assert_name_errors(&r, 1);
}

// ---------------------------------------------------------------------------
// 6. The "what conducts at DC" table, observed from the product path
// ---------------------------------------------------------------------------

/// A current source fixes a current, not a potential: into a node that is
/// otherwise alone it leaves the voltage completely undetermined, and it never
/// counts as a reference path even when one of its terminals is ground.
///
/// Blocks: a rule that treats "there is a device between this node and ground"
/// as sufficient, which would accept i1 and hand the solver a singular matrix.
#[test]
fn a_current_source_alone_does_not_reference_a_node() {
    let r = rejected(
        r#"
circuit :current_referenced do
  node :x
  current_source :i1, p: :x, n: :gnd, dc: 1.mA
end
"#,
    );

    assert_mentions(&r, "node `x` has no DC path to ground");
    assert_mentions(&r, "attached but not conducting at DC: i1");
    assert_name_errors(&r, 1);
}

/// The positive control for the same rule: an inductor is a short at DC, so a
/// node behind one is fully referenced and must be accepted.
///
/// Blocks: a rule that only trusts resistors and voltage sources, which would
/// reject every LC network and every wire-like inductor connection.
#[test]
fn an_inductor_is_a_dc_reference_path() {
    let c = accepted(
        r#"
circuit :inductive do
  node :a, :b
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  inductor :l1, p: :a, n: :b, value: 1.mH
end
"#,
    );

    let circuit = c.circuit("inductive").expect("circuit :inductive");
    let a = circuit.node_id("a").unwrap();
    let b = circuit.node_id("b").unwrap();
    let l1 = circuit.device(circuit.device_id("l1").unwrap()).unwrap();
    assert_eq!(l1.kind, DeviceKind::Inductor);
    assert_eq!(l1.pos(), Some(a));
    assert_eq!(l1.neg(), Some(b));
    let v1 = circuit.device(circuit.device_id("v1").unwrap()).unwrap();
    assert_eq!(v1.neg(), Some(circuit.ground()));
}

/// A diode is non-linear but it carries DC current, so it is a reference path
/// too: with d1 from a to b and a driven from ground, b is determined (the diode
/// current must be zero, which fixes v(b) = v(a)).
///
/// Blocks: the tempting simplification "only linear devices anchor a node",
/// which would reject every rectifier whose output has no load resistor.
#[test]
fn a_diode_is_a_dc_reference_path() {
    let c = accepted(
        r#"
circuit :diode_biased do
  model :dmod, type: :diode, is: 1e-14.A, n: 1
  node :a, :b
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  diode :d1, p: :a, n: :b, model: :dmod
end
"#,
    );

    let circuit = c.circuit("diode_biased").expect("circuit :diode_biased");
    let d1 = circuit.device(circuit.device_id("d1").unwrap()).unwrap();
    assert_eq!(d1.kind, DeviceKind::Diode);
    assert_eq!(d1.pos(), Some(circuit.node_id("a").unwrap()));
    assert_eq!(d1.neg(), Some(circuit.node_id("b").unwrap()));
}
