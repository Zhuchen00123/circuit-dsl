//! Round-4 phase-B QA: the parameter graph through the session/REPL path.
//!
//! Contract: docs/review-evidence/round4/design-contract.md §4.7. The session
//! is what cdsl repl drives, so these tests exercise the same run path the
//! transcript does, plus the transactional promise: a failed override leaves
//! the next successful run exactly where it was.
//!
//! Independent reference for the fixture below: with r = 1 kohm the chain is
//! r_doubled = 2 r = 2 kohm and r_eff = 2 * r_doubled = 4 kohm, so
//!   v(out) = 6 V * r_eff / (1 kohm + r_eff) = 6 * 4/5 = 4.8 V.
//! With r = 2 kohm: r_eff = 8 kohm and v(out) = 6 * 8/9 = 5.333333... V.
//! With r = 3 kohm: r_eff = 12 kohm and v(out) = 6 * 12/13 = 5.5384615... V.

use std::path::{Path, PathBuf};

use circuit_session::Reply;
use circuit_session::session::{Options, Session};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .to_path_buf()
}

fn scratch(tag: &str) -> PathBuf {
    let dir = workspace_root()
        .join("target")
        .join("round4")
        .join("qa-b")
        .join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the QA scratch directory");
    dir
}

/// r -> r_doubled -> r_eff, both edges forward-referenced on purpose: the
/// declared chain is r_eff first, r_doubled second, r last.
const DAG: &str = "circuit :div do
  param :r, default: 1.kohm
  param :r_eff, default: 2 * r_doubled
  param :r_doubled, default: r + r
  node :in, :out
  voltage_source :src, p: :in, n: :gnd, dc: 6.V
  resistor :r1, p: :in, n: :out, value: 1.kohm
  resistor :r2, p: :out, n: :gnd, value: r_eff
end
experiment :e, circuit: :div do
  op
  save v(:out)
end
";

/// n -> width -> the loop range: a topology-affecting sweep, refused before a
/// single point is solved.
const TOPO_SWEEP: &str = "circuit :var_indirect do
  param :n, default: 1
  param :width, default: n + 1
  node :a, :b
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
  for k in 1..width do
    resistor (\"r\" + k), p: :a, n: :b, value: 1.kohm
  end
  resistor :rload, p: :b, n: :gnd, value: 1.kohm
end
experiment :sweep, circuit: :var_indirect do
  dc param: :n, from: 0, to: 2, step: 1
  save v(:b)
end
";

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let file = dir.join(name);
    std::fs::write(&file, text).expect("write the fixture");
    file
}

fn message(reply: Reply) -> String {
    match reply {
        Reply::Message(text) => text,
        other => panic!("expected a run summary, got {other:?}"),
    }
}

fn loaded(path: &Path) -> Session {
    let mut session = Session::new(Options::default());
    session.load(path, false).expect("the fixture loads");
    session
}

// ---------------------------------------------------------------------------
// Overrides recompute the dependents
// ---------------------------------------------------------------------------

#[test]
fn an_override_recomputes_every_dependent_parameter() {
    let dir = scratch("r4b_session_recompute");
    let file = write(&dir, "dag.cdsl", DAG);
    let mut session = loaded(&file);

    let first = message(session.feed(":run e").expect("the default run succeeds"));
    assert!(
        first.contains("v(out) = 4.8 V"),
        "r_eff = 2 * (2 * 1 kohm) = 4 kohm -> 4.8 V:\n{first}"
    );

    let second = message(
        session
            .feed(":run e r=2.kohm")
            .expect("the override run succeeds"),
    );
    assert!(
        second.contains("override r = 2 kohm"),
        "the summary must show the override:\n{second}"
    );
    assert!(
        second.contains("v(out) = 5.33333 V"),
        "r_eff must follow the override: 2 * (2 * 2 kohm) = 8 kohm -> 6*8/9 = 5.33333 V:\n{second}"
    );

    let third = message(
        session
            .feed(":run e r=3.kohm")
            .expect("the second override run succeeds"),
    );
    assert!(
        third.contains("v(out) = 5.53846 V"),
        "r_eff = 12 kohm -> 6*12/13 = 5.5384615 V:\n{third}"
    );

    // Going back to the default must recompute back to 4.8 V, not stay at the
    // last override.
    let again = message(session.feed(":run e").expect("the default run still works"));
    assert!(
        again.contains("v(out) = 4.8 V"),
        "the default must be recomputed from the base parameter again:\n{again}"
    );
}

// ---------------------------------------------------------------------------
// A failed update does not pollute the next one
// ---------------------------------------------------------------------------

#[test]
fn a_failed_override_does_not_pollute_the_next_run() {
    let dir = scratch("r4b_session_failure");
    let file = write(&dir, "dag.cdsl", DAG);
    let mut session = loaded(&file);

    let before = message(session.feed(":run e").expect("the default run succeeds"));
    assert!(before.contains("v(out) = 4.8 V"), "{before}");

    // r = 0 makes every resistor in the chain zero, which the elaboration
    // refuses; the session must not commit a half-applied override.
    let failure = session
        .feed(":run e r=0.ohm")
        .expect_err("a zero-valued parameter must fail the run");
    let rendered = session.render(&failure);
    assert!(
        rendered.contains("[E_"),
        "a structured diagnostic is expected:\n{rendered}"
    );

    // The failure changed nothing: the same run gives the same text.
    let after = message(
        session
            .feed(":run e")
            .expect("the session still runs after a failure"),
    );
    assert_eq!(
        before, after,
        "a failed override must not change the next result"
    );

    // And a successful override afterwards still works.
    let fixed = message(
        session
            .feed(":run e r=2.kohm")
            .expect("the next override succeeds"),
    );
    assert!(fixed.contains("v(out) = 5.33333 V"), "{fixed}");

    // A failed override after a successful one is equally harmless.
    let _ = session
        .feed(":run e r=0.ohm")
        .expect_err("zero fails again");
    let repeated = message(session.feed(":run e").expect("still runs"));
    assert_eq!(repeated, before, "{repeated}");
}

// ---------------------------------------------------------------------------
// The session refuses a topology sweep at definition time
// ---------------------------------------------------------------------------

/// A value-sweep program through the session: the swept parameter only ever
/// lands in a numeric position, so it must stay legal.
const VALUE_SWEEP: &str = "circuit :swept_session do
  param :rf, default: 1.kohm
  param :r1, default: 1.kohm
  node :in, :out
  voltage_source :src, p: :in, n: :gnd, dc: 3.V
  resistor :ra, p: :in, n: :out, value: r1
  resistor :rb, p: :out, n: :gnd, value: rf
end
experiment :sweep, circuit: :swept_session do
  dc param: :rf, from: 1.kohm, to: 3.kohm, step: 1.kohm
  save v(:out)
end
";

/// Refusal happens where the contract puts it: compile() builds the experiment
/// plan, and :load/:define use the same compile as cdsl check, so a program
/// that would sweep a topology-affecting parameter is rejected before any
/// solve - it never becomes runnable at all.
#[test]
fn the_session_refuses_a_topology_sweep_before_it_can_run() {
    let dir = scratch("r4b_session_topo");
    let file = write(&dir, "topo.cdsl", TOPO_SWEEP);
    let mut session = Session::new(Options::default());

    let failure = session
        .load(&file, false)
        .expect_err("a topology sweep must be refused at definition time");
    let rendered = session.render(&failure);
    assert!(
        rendered.contains("[E_TOPO_PARAM]"),
        "the sweep must be refused at check time:\n{rendered}"
    );
    assert!(
        rendered.contains("width"),
        "the explanation must show the intermediate parameter:\n{rendered}"
    );
    assert!(
        rendered.contains("n -> width"),
        "the explanation path must be printed:\n{rendered}"
    );
    assert!(
        rendered.contains("sweep"),
        "the refusal must point at the sweep statement:\n{rendered}"
    );

    // Nothing was committed, and no point was ever solved: the rejected
    // program left no experiment behind.
    assert!(
        session.experiment_names().is_empty(),
        "a refused definition must not be stored: {:?}",
        session.experiment_names()
    );
    let unknown = session
        .run("sweep", &[], None)
        .expect_err("the refused experiment does not exist");
    assert!(
        session.render(&unknown).contains("[E_NAME]"),
        "the experiment was never defined:\n{}",
        session.render(&unknown)
    );

    // The rejection is not a blanket ban: a plain value sweep still defines and
    // still runs, delivering its three points.
    let ok_file = write(&dir, "value.cdsl", VALUE_SWEEP);
    let mut ok = Session::new(Options::default());
    ok.load(&ok_file, false)
        .expect("a value sweep must still define");
    let reply = ok
        .run("sweep", &[], None)
        .expect("a value sweep must still run");
    let text = message(reply);
    assert!(
        text.contains("3 sweep points"),
        "the sweep must deliver its three points:\n{text}"
    );
}
