//! Session behaviour: the acceptance evidence for the interactive front end's
//! rules.
//!
//! Every test drives the real session — the same object the REPL binary talks
//! to — so a change to input handling, definition replacement or run
//! execution cannot pass by accident here.

use circuit_core::units::Quantity;
use circuit_session::{Reply, Session};

/// Feed a chunk of text, one line at a time, exactly as the REPL does.
fn feed_all(session: &mut Session, text: &str) -> Vec<Result<Reply, String>> {
    let mut out = Vec::new();
    for line in text.lines() {
        let result = session.feed(line).map_err(|d| session.render(&d));
        out.push(result);
    }
    out
}

/// The last reply, which must be a value.
fn value_of(replies: &[Result<Reply, String>]) -> String {
    match replies.last().expect("at least one reply") {
        Ok(Reply::Value { text, .. }) => text.clone(),
        other => panic!("expected a value, got {other:?}"),
    }
}

fn errors_of(replies: &[Result<Reply, String>]) -> Vec<String> {
    replies
        .iter()
        .filter_map(|r| r.as_ref().err().cloned())
        .collect()
}

const DIVIDER: &str = r#"circuit :divider do
  param :r1, default: 1.kohm
  param :r2, default: 1.kohm
  node :in, :out
  voltage_source :v1, p: :in, n: :gnd, dc: 5.V
  resistor :ra, p: :in, n: :out, value: r1
  resistor :rb, p: :out, n: :gnd, value: r2
end
experiment :divider, circuit: :divider do
  op
  save v(:out), i(:ra)
end"#;

fn session_with_divider() -> Session {
    let mut session = Session::default();
    let replies = feed_all(&mut session, DIVIDER);
    assert!(errors_of(&replies).is_empty(), "{replies:?}");
    session
}

/// The value of `v(out)` in the last run's message, in SI units.
fn v_out_of(reply: &Reply) -> f64 {
    let Reply::Message(text) = reply else {
        panic!("expected a run summary, got {reply:?}");
    };
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("v(out) = ") {
            // The display is `2.5 V`, so strip the unit and scale.
            let number: f64 = rest
                .split_whitespace()
                .next()
                .expect("a number")
                .parse()
                .expect("parses");
            let scale = if rest.contains("k") { 1e3 } else { 1.0 };
            return number * scale;
        }
    }
    panic!("no v(out) line in:\n{text}");
}

// ---------------------------------------------------------------------------
// Variables
// ---------------------------------------------------------------------------

#[test]
fn a_variable_can_be_defined_shown_and_reassigned() {
    let mut s = Session::default();
    assert!(matches!(
        feed_all(&mut s, "r = 1.kohm")[0],
        Ok(Reply::Value { .. })
    ));
    let replies = feed_all(&mut s, "r\nr * 2\nr = 2.2.kohm\nr\n");
    assert_eq!(value_of(&replies[0..1]), "1 kohm");
    assert_eq!(value_of(&replies[1..2]), "2 kohm");
    // The reassignment wins, and the session says so.
    match &replies[2] {
        Ok(Reply::Value { name, text, .. }) => {
            assert_eq!(name.as_deref(), Some("r"));
            assert_eq!(text, "2.2 kohm");
        }
        other => panic!("expected an assignment reply, got {other:?}"),
    }
    assert_eq!(value_of(&replies[3..4]), "2.2 kohm");
}

#[test]
fn dimensions_propagate_through_session_arithmetic() {
    let mut s = Session::default();
    let replies = feed_all(&mut s, "r = 1.kohm\nc = 100.nF\ntau = r * c\ntau\n");
    assert!(errors_of(&replies).is_empty(), "{replies:?}");
    assert_eq!(value_of(&replies[3..4]), "100 us");

    // A ratio of like quantities is dimensionless.
    let replies = feed_all(&mut s, "r / 500.ohm\n");
    assert_eq!(value_of(&replies), "2");

    // And a mismatched addition is refused with both dimensions named.
    let replies = feed_all(&mut s, "r + c\n");
    let text = &errors_of(&replies)[0];
    assert!(text.contains("E_DIMENSION"), "{text}");
    assert!(text.contains("ohm") && text.contains("F"), "{text}");
}

#[test]
fn an_undefined_variable_is_reported_not_guessed() {
    let mut s = Session::default();
    let replies = feed_all(&mut s, "nope * 2\n");
    let text = &errors_of(&replies)[0];
    assert!(text.contains("E_NAME"), "{text}");
    assert!(text.contains("`nope` is not declared"), "{text}");
}

#[test]
fn a_variable_must_be_a_number() {
    let mut s = Session::default();
    let replies = feed_all(&mut s, "x = :vin\n");
    let text = &errors_of(&replies)[0];
    assert!(text.contains("must be a number"), "{text}");
    let replies = feed_all(&mut s, "y = [1, 2]\n");
    assert!(errors_of(&replies)[0].contains("must be a number"));
}

#[test]
fn expressions_show_their_type_by_value_not_by_silent_conversion() {
    let mut s = Session::default();
    let replies = feed_all(&mut s, ":vin\n\"text\"\n[1, 2]\n{ a: 1 }\ntrue\n");
    assert_eq!(value_of(&replies[0..1]), ":vin");
    assert_eq!(value_of(&replies[1..2]), "\"text\"");
    assert_eq!(value_of(&replies[2..3]), "[1, 2]");
    assert_eq!(value_of(&replies[3..4]), "{ a: 1 }");
    assert_eq!(value_of(&replies[4..5]), "true");
}

// ---------------------------------------------------------------------------
// Multi-line input versus real errors
// ---------------------------------------------------------------------------

#[test]
fn an_unfinished_input_is_continued_and_then_completed() {
    let mut s = Session::default();
    assert!(matches!(
        s.feed("circuit :d do").unwrap(),
        Reply::Continue { .. }
    ));
    assert!(matches!(
        s.feed("  node :a").unwrap(),
        Reply::Continue { .. }
    ));
    assert!(matches!(
        s.feed("  voltage_source :v1, p: :a, n: :gnd, dc: 1.V")
            .unwrap(),
        Reply::Continue { .. }
    ));
    match s.feed("end").unwrap() {
        Reply::Defined { accepted, .. } => assert_eq!(accepted, ["circuit `d`"]),
        other => panic!("expected a definition, got {other:?}"),
    }
}

#[test]
fn a_real_syntax_error_is_reported_immediately() {
    let mut s = Session::default();
    let err = s.feed("node 5").unwrap_err();
    let text = s.render(&err);
    assert!(text.contains("E_SYNTAX"), "{text}");
    // The input is not left pending, so the next line is a fresh input.
    assert!(!s.is_continuing());
    assert!(matches!(s.feed("1 + 1").unwrap(), Reply::Value { .. }));
}

#[test]
fn a_body_statement_at_the_prompt_says_where_it_belongs() {
    let mut s = Session::default();
    let err = s.feed("for k in 1..3 do").unwrap_err();
    let text = s.render(&err);
    assert!(text.contains("body statement"), "{text}");
    assert!(text.contains("circuit :name do"), "{text}");
    assert!(!s.is_continuing());
}

#[test]
fn cancelling_an_input_keeps_the_session() {
    let mut s = Session::default();
    feed_all(&mut s, "r = 1.kohm");
    assert!(matches!(
        s.feed("circuit :d do").unwrap(),
        Reply::Continue { .. }
    ));
    s.cancel_pending();
    assert!(!s.is_continuing());
    // The variable and the (absent) definitions are untouched, and the next
    // input is read from a clean slate.
    assert_eq!(s.variables().count(), 1);
    assert_eq!(value_of(&feed_all(&mut s, "r")), "1 kohm");
}

#[test]
fn diagnostics_point_into_the_input_that_produced_them() {
    let mut s = Session::default();
    // `1 +` is unfinished: it asks for more rather than reporting anything.
    assert!(matches!(s.feed("1 +").unwrap(), Reply::Continue { .. }));
    s.cancel_pending();

    // The second input is the one that fails, and its source is named after
    // the line it started on, so a location can be found in the scrollback.
    let err = s.feed("x = :a").unwrap_err();
    let text = s.render(&err);
    assert!(text.contains("<repl:2>:1:5"), "{text}");
    assert!(text.contains("^"), "{text}");
}

// ---------------------------------------------------------------------------
// Definitions
// ---------------------------------------------------------------------------

#[test]
fn a_definition_is_replaced_only_when_it_compiles() {
    let mut s = session_with_divider();

    // A redefinition with a different topology replaces the old one.
    let replies = feed_all(
        &mut s,
        r#"circuit :divider do
  param :r1, default: 2.kohm
  node :in, :out
  voltage_source :v1, p: :in, n: :gnd, dc: 6.V
  resistor :ra, p: :in, n: :out, value: r1
  resistor :rb, p: :out, n: :gnd, value: 2.kohm
end"#,
    );
    match &replies.last().unwrap() {
        Ok(Reply::Defined { replaced, .. }) => assert_eq!(replaced, &["circuit `divider`"]),
        other => panic!("expected a replacement, got {other:?}"),
    }
    let run = s.run("divider", &[], None).unwrap();
    assert!(
        (v_out_of(&run) - 3.0).abs() < 1e-9,
        "6 V across two equal halves"
    );

    // A redefinition that does not compile changes nothing.
    let replies = feed_all(
        &mut s,
        r#"circuit :divider do
  node :in
  resistor :rb, p: :in, n: :nope, value: 1.kohm
end"#,
    );
    assert!(!errors_of(&replies).is_empty());
    // The previous definition is still there and still runs.
    let run = s.run("divider", &[], None).unwrap();
    assert!((v_out_of(&run) - 3.0).abs() < 1e-9);
}

#[test]
fn a_failed_definition_leaves_variables_alone() {
    let mut s = session_with_divider();
    feed_all(&mut s, "keep = 42");
    let replies = feed_all(
        &mut s,
        "circuit :bad do\n  node :a\n  resistor :r, p: :a\nend",
    );
    assert!(!errors_of(&replies).is_empty());
    assert_eq!(value_of(&feed_all(&mut s, "keep")), "42");
}

#[test]
fn a_redefinition_that_would_break_an_experiment_is_refused() {
    let mut s = session_with_divider();
    // The experiment probes `v(:out)`; a replacement without that node cannot
    // be stored, because the committed state must always compile.
    let replies = feed_all(
        &mut s,
        "circuit :divider do\n  node :in\n  voltage_source :v1, p: :in, n: :gnd, dc: 5.V\nend",
    );
    let text = errors_of(&replies).join("\n");
    assert!(text.contains("E_NAME"), "{text}");
    // The old definition is intact.
    let run = s.run("divider", &[], None).unwrap();
    assert!((v_out_of(&run) - 2.5).abs() < 1e-9);
}

#[test]
fn definitions_and_variables_are_separate_namespaces() {
    let mut s = Session::default();
    feed_all(&mut s, "divider = 7");
    let replies = feed_all(
        &mut s,
        "circuit :divider do\n  node :a\n  voltage_source :v1, p: :a, n: :gnd, dc: 1.V\nend",
    );
    assert!(errors_of(&replies).is_empty(), "{replies:?}");
    assert_eq!(s.circuit_names(), ["divider"]);
    assert_eq!(value_of(&feed_all(&mut s, "divider")), "7");
}

#[test]
fn an_experiment_defined_before_its_circuit_is_refused_with_a_hint() {
    let mut s = Session::default();
    let replies = feed_all(
        &mut s,
        "experiment :e, circuit: :later do\n  op\n  save v(:a)\nend",
    );
    let text = errors_of(&replies).join("\n");
    assert!(text.contains("unknown circuit `later`"), "{text}");
    assert!(text.contains("define `circuit :later` first"), "{text}");

    // Entering both at once works, because they are checked as one state.
    let replies = feed_all(
        &mut s,
        r#"circuit :later do
  node :a
  voltage_source :v1, p: :a, n: :gnd, dc: 1.V
end
experiment :e, circuit: :later do
  op
  save v(:a)
end"#,
    );
    assert!(errors_of(&replies).is_empty(), "{replies:?}");
}

// ---------------------------------------------------------------------------
// Scope boundary
// ---------------------------------------------------------------------------

#[test]
fn a_session_variable_does_not_leak_into_a_circuit() {
    let mut s = Session::default();
    feed_all(&mut s, "r = 1.kohm");
    // The circuit mentions `r` but never declares it as a parameter.
    let replies = feed_all(
        &mut s,
        "circuit :leak do\n  node :a\n  voltage_source :v1, p: :a, n: :gnd, dc: 1.V\n  resistor :r1, p: :a, n: :gnd, value: r\nend",
    );
    let text = errors_of(&replies).join("\n");
    assert!(text.contains("`r` is not declared"), "{text}");
    assert!(s.circuit_names().is_empty(), "nothing was defined");
}

#[test]
fn a_circuit_parameter_is_not_visible_to_the_session() {
    let mut s = session_with_divider();
    // `r1` is the circuit's parameter, not a session variable.
    let replies = feed_all(&mut s, "r1\n");
    let text = &errors_of(&replies)[0];
    assert!(text.contains("`r1` is not declared"), "{text}");
}

// ---------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------

#[test]
fn an_explicit_override_changes_the_simulation() {
    let mut s = session_with_divider();

    let base = s.run("divider", &[], None).unwrap();
    assert!(
        (v_out_of(&base) - 2.5).abs() < 1e-9,
        "5 V across equal halves"
    );

    let overridden = s
        .run(
            "divider",
            &[("r1".to_string(), Quantity::ohms(3_000.0))],
            None,
        )
        .unwrap();
    // 5 V across 3k and 1k: the tap sits at a quarter.
    assert!(
        (v_out_of(&overridden) - 1.25).abs() < 1e-9,
        "got {}",
        v_out_of(&overridden)
    );

    // The summary states the override, so the run can be read back.
    let Reply::Message(text) = &overridden else {
        panic!("expected a summary");
    };
    assert!(text.contains("override r1 = 3 kohm"), "{text}");

    // And the session is unchanged: the next run without an override is back
    // to the default, because a run never edits the definitions.
    let again = s.run("divider", &[], None).unwrap();
    assert!((v_out_of(&again) - 2.5).abs() < 1e-9);
}

#[test]
fn an_override_for_a_parameter_that_does_not_exist_is_refused() {
    let mut s = session_with_divider();
    let err = s
        .run("divider", &[("r9".to_string(), Quantity::ohms(10.0))], None)
        .unwrap_err();
    let text = s.render(&err);
    assert!(text.contains("has no parameter `r9`"), "{text}");
    assert!(text.contains("declared parameters: r1, r2"), "{text}");
}

#[test]
fn running_an_unknown_experiment_lists_the_defined_ones() {
    let mut s = session_with_divider();
    let err = s.run("nope", &[], None).unwrap_err();
    let text = s.render(&err);
    assert!(text.contains("no experiment named `nope`"), "{text}");
    assert!(text.contains("divider"), "{text}");
}

#[test]
fn a_run_uses_the_current_definition_not_a_cached_one() {
    let mut s = session_with_divider();
    // Change the device value, keep the parameter: the tap must move.
    let replies = feed_all(
        &mut s,
        "circuit :divider do\n  param :r1, default: 1.kohm\n  param :r2, default: 1.kohm\n  node :in, :out\n  voltage_source :v1, p: :in, n: :gnd, dc: 5.V\n  resistor :ra, p: :in, n: :out, value: r1\n  resistor :rb, p: :out, n: :gnd, value: r2 * 4\nend",
    );
    assert!(errors_of(&replies).is_empty(), "{replies:?}");
    let run = s.run("divider", &[], None).unwrap();
    // 5 V across 1k and 4k: the tap sits at four fifths.
    assert!((v_out_of(&run) - 4.0).abs() < 1e-9, "{}", v_out_of(&run));
}

#[test]
fn a_sweep_experiment_runs_through_the_session_too() {
    let mut s = Session::default();
    let replies = feed_all(
        &mut s,
        r#"circuit :swept do
  param :r, default: 1.kohm
  node :in, :out
  voltage_source :src, p: :in, n: :gnd, dc: 3.V
  resistor :r1, p: :in, n: :out, value: r
  resistor :r2, p: :out, n: :gnd, value: 1.5.kohm
end
experiment :sweep, circuit: :swept do
  dc param: :r, from: 0.5.kohm, to: 2.kohm, step: 0.5.kohm
  save v(:out)
end"#,
    );
    assert!(errors_of(&replies).is_empty(), "{replies:?}");

    let reply = s.run("sweep", &[], None).unwrap();
    let Reply::Message(text) = &reply else {
        panic!("expected a summary");
    };
    assert!(text.contains("4 sweep points"), "{text}");
    assert!(text.contains("dc_param_r"), "{text}");
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[test]
fn commands_are_not_part_of_the_language() {
    let mut s = Session::default();
    // A bare symbol with a colon is a value; only the fixed command words are
    // commands, so a symbol expression is never swallowed by the dispatcher.
    assert_eq!(value_of(&feed_all(&mut s, ":vin")), ":vin");
    assert_eq!(value_of(&feed_all(&mut s, ":nope")), ":nope");
    // A command word is a command, so the symbol needs the parenthesised form.
    assert!(matches!(s.feed(":help").unwrap(), Reply::Message(_)));
    assert_eq!(value_of(&feed_all(&mut s, "(:help)")), ":help");
    // An argument makes no difference: still a command.
    assert!(matches!(s.feed(":list").unwrap(), Reply::Message(_)));
}

#[test]
fn help_list_and_reset_report_what_they_did() {
    let mut s = session_with_divider();
    feed_all(&mut s, "r = 1.kohm");
    match s.feed(":list").unwrap() {
        Reply::Message(text) => {
            assert!(text.contains("circuit :divider"), "{text}");
            assert!(text.contains("experiment :divider"), "{text}");
            assert!(text.contains("r = 1 kohm"), "{text}");
        }
        other => panic!("expected a listing, got {other:?}"),
    }
    match s.feed(":reset").unwrap() {
        Reply::Message(text) => {
            assert!(text.contains("2 definition(s)"), "{text}");
            assert!(text.contains("1 variable(s)"), "{text}");
        }
        other => panic!("expected a report, got {other:?}"),
    }
    assert!(s.circuit_names().is_empty());
    assert_eq!(s.variables().count(), 0);
    assert!(matches!(s.feed(":quit").unwrap(), Reply::Quit));
}

#[test]
fn an_empty_line_produces_nothing() {
    let mut s = Session::default();
    assert!(matches!(s.feed("").unwrap(), Reply::Nothing));
    assert!(matches!(s.feed("   ").unwrap(), Reply::Nothing));
    assert!(matches!(s.feed("# a comment").unwrap(), Reply::Nothing));
}
