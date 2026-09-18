//! DSL-layer regression for `tran` transient-option validation.
//!
//! Why this file exists: `crates/circuit-dsl/src/elaborate.rs::tran_spec` now
//! requires an explicit `output_interval:` — and an explicit `max_step:` — to be
//! a **finite number greater than zero**. An offender is reported as
//! `Code::Value` with the primary span on that very argument, and `tran_spec`
//! returns `None`: no `TranSpec` is built, and nothing falls back to a default.
//!
//! What the un-fixed front end did — measured, not inferred; see
//! `docs/review-evidence/round2/repro-baseline.md` §2.3, §5, §6 and §7:
//!
//! * `tran stop: 2.us, max_step: 1.ns, output_interval: -1.ns`: `cdsl check`
//!   exited **0** with **zero diagnostics**; `cdsl run` exited 0 and returned
//!   **2015** time points.
//! * `output_interval: 0.s` on the same circuit: check exit 0 (stderr 0 bytes),
//!   run 2015 points, CSV SHA-256
//!   `348973A51D487072DFDCD93460796B3ACC44A17039959919C65309F7FDEFCA96` — the
//!   same hash as the `1.ns` run and the `-1.ns` run, i.e. **byte-for-byte
//!   identical output**. On that circuit the silent fallback value
//!   `stop/1000 = 2 ns` was hidden underneath the declared `rise: 10.ns`.
//! * The discriminating probe in that evidence (`stop: 20.us`,
//!   `max_step: 100.ns`, `output_interval: -1.ns`) showed the fallback really
//!   did reach the engine: `stop/1000 = 20 ns` widened the declared
//!   `rise: 10.ns` to 20 ns (225 points, first `v(vin) >= 0.999999` at exactly
//!   `0.000000019999999999999997` s), while the `output_interval: 1.ns` control
//!   kept the declared 10 ns edge (228 points, first `v(vin) = 1` at
//!   `0.00000001` s). No warning told the user the declaration had been
//!   dropped, so the only visible difference was a silently widened edge.
//!
//! These tests drive the real front end (`lex`, `parse`, `compile` and
//! `elaborate_experiment`) over the same source shape as that evidence. They
//! assert on the `Diagnostics` themselves — code, exact message, and the raw
//! `SourceSpan` read back with `SourceMap::snippet` — not only on rendered
//! prose, so a silent fallback cannot pass by accident. The "no fallback" test
//! is structural on purpose: after a rejection `compile` returns `Err`, so the
//! test never receives a plan that could hold a `TranSpec` at all, and the
//! `E_ARGUMENT`/`declares no analysis` diagnostic proves `tran_spec` pushed no
//! `AnalysisTask` (a defaulted interval would have built one, and that
//! diagnostic would disappear).

use circuit_core::{AnalysisKind, Code, Diagnostic, Diagnostics, Limits, SourceMap, TranSpec};
use circuit_dsl::ast::{AnalysisCall, ExpStmt, Expr};
use circuit_dsl::eval::{NoVariables, eval as eval_expr};
use circuit_dsl::token::{QuantityLiteral, TokenKind};
use circuit_dsl::{compile, elaborate_experiment, lex, parse};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// The real front end applied to one source file, keeping everything a test
/// needs to check a diagnostic *and* to evaluate an argument on its own.
struct Frontend {
    /// The source text, so a span can be turned back into the code it covers.
    source: String,
    /// The source map, so diagnostics can be rendered the way `cdsl check`
    /// renders them.
    sources: SourceMap,
    /// The parsed program, for the tests that also go through
    /// `elaborate_experiment`.
    program: circuit_dsl::Program,
    result: Result<circuit_dsl::Compiled, Diagnostics>,
}

fn front_end(src: &str) -> Frontend {
    let mut sources = SourceMap::new();
    let id = sources.add("tran_option_validation.cdsl", src);

    let tokens = match lex(id, src) {
        Ok(t) => t,
        Err(d) => panic!("this file's sources must lex:\n{}", d.render(&sources)),
    };
    let program = match parse(&tokens) {
        Ok(p) => p,
        Err(d) => panic!("this file's sources must parse:\n{}", d.render(&sources)),
    };
    let result = compile(&program, &Limits::default());

    Frontend {
        source: src.to_string(),
        sources,
        program,
        result,
    }
}

impl Frontend {
    /// The diagnostics of a run that was expected to fail.
    fn diagnostics(&self) -> &Diagnostics {
        match &self.result {
            Ok(_) => panic!("expected diagnostics, but compilation succeeded"),
            Err(d) => d,
        }
    }

    /// Every error, in the order the elaborator produced them.
    fn errors(&self) -> Vec<&Diagnostic> {
        self.diagnostics().errors().collect()
    }

    /// Render exactly as `cdsl check` does.
    fn rendered(&self) -> String {
        self.diagnostics().render(&self.sources)
    }

    /// The one error whose code is `code` and whose message contains `needle`.
    fn error(&self, code: Code, needle: &str) -> &Diagnostic {
        let found: Vec<&Diagnostic> = self
            .diagnostics()
            .errors()
            .filter(|d| d.code == code && d.message.contains(needle))
            .collect();
        assert_eq!(
            found.len(),
            1,
            "expected exactly one {code} error whose message contains {needle:?} in:\n{}",
            self.rendered()
        );
        found[0]
    }

    fn has_error(&self, code: Code, needle: &str) -> bool {
        self.diagnostics()
            .errors()
            .any(|d| d.code == code && d.message.contains(needle))
    }

    /// The verbatim source text a diagnostic's primary span covers.
    fn primary_text(&self, d: &Diagnostic) -> &str {
        let label = d
            .primary
            .as_ref()
            .unwrap_or_else(|| panic!("`{}` has no primary span", d.message));
        self.sources.snippet(label.span)
    }

    /// The source text that precedes the primary span, on the same line.
    fn primary_prefix(&self, d: &Diagnostic) -> &str {
        let label = d
            .primary
            .as_ref()
            .unwrap_or_else(|| panic!("`{}` has no primary span", d.message));
        &self.source[..label.span.start as usize]
    }

    fn error_count(&self) -> usize {
        self.errors().len()
    }

    /// Every error code of the run, for readable failure messages.
    fn codes(&self) -> Vec<&'static str> {
        self.errors().iter().map(|d| d.code.as_str()).collect()
    }
}

/// Parse, elaborate and expect a rejection.
fn rejected(src: &str) -> Frontend {
    let run = front_end(src);
    assert!(
        run.result.is_err(),
        "expected the front end to reject this source, but it compiled:\n{src}"
    );
    run
}

/// Parse, elaborate and expect success.
fn compiled(src: &str) -> circuit_dsl::Compiled {
    let run = front_end(src);
    match &run.result {
        Ok(c) => c.clone(),
        Err(d) => panic!(
            "expected success, got diagnostics:\n{}",
            d.render(&run.sources)
        ),
    }
}

/// The `TranSpec` of the single `tran` task of experiment `probe`. Panics if
/// the source was rejected or produced a different task, so a test that expects
/// a usable spec cannot silently get `None`.
fn tran_spec_of(src: &str) -> TranSpec {
    let run = front_end(src);
    let compiled = match &run.result {
        Ok(c) => c,
        Err(d) => panic!(
            "expected success, got diagnostics:\n{}",
            d.render(&run.sources)
        ),
    };
    only_tran_spec(compiled, "probe")
}

fn only_tran_spec(compiled: &circuit_dsl::Compiled, experiment_name: &str) -> TranSpec {
    let experiment = compiled
        .experiment(experiment_name)
        .unwrap_or_else(|| panic!("experiment `{experiment_name}` is missing"));
    assert_eq!(
        experiment.plan.tasks.len(),
        1,
        "one `tran` statement must produce exactly one task, found {}",
        experiment.plan.tasks.len()
    );
    match &experiment.plan.tasks[0].kind {
        AnalysisKind::Tran(spec) => spec.clone(),
        other => panic!("expected a `tran` task, found `{}`", other.name()),
    }
}

/// The single `tran (...)` call of experiment `probe`.
fn tran_call(program: &circuit_dsl::Program) -> &AnalysisCall {
    let experiment = program
        .experiment("probe")
        .unwrap_or_else(|| panic!("experiment `probe` is missing"));
    experiment
        .body
        .iter()
        .find_map(|stmt| match stmt {
            ExpStmt::Tran(call) => Some(call),
            _ => None,
        })
        .unwrap_or_else(|| panic!("experiment `probe` has no `tran` statement"))
}

/// The `f64` the evaluator returns for the `output_interval:` argument of a
/// source; the arguments used here are constant expressions, so no variables
/// are in scope.
fn eval_output_interval(src: &str) -> f64 {
    let run = front_end(src);
    let expr: Expr = tran_call(&run.program)
        .arg("output_interval")
        .expect("the source must carry an `output_interval:` argument")
        .value
        .clone();
    match eval_expr(&expr, &NoVariables) {
        Ok(circuit_dsl::Value::Num(q)) => q.value,
        Ok(other) => panic!("`output_interval:` evaluated to {}", other.type_name()),
        Err(d) => panic!(
            "`output_interval:` failed to evaluate:\n{}",
            d.render(&run.sources)
        ),
    }
}

fn lex_quantity(text: &str) -> QuantityLiteral {
    let mut sources = SourceMap::new();
    let id = sources.add("literal.cdsl", text);
    let tokens =
        lex(id, text).unwrap_or_else(|d| panic!("`{text}` must lex:\n{}", d.render(&sources)));
    tokens
        .iter()
        .find_map(|t| match &t.kind {
            TokenKind::Quantity(q) => Some(q.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("`{text}` must lex to a quantity literal"))
}

/// An RC transient experiment with the `tran` arguments under test. The circuit
/// is the one `docs/review-evidence/round2/repro-baseline.md` used to reproduce
/// the silent fallback, so the pre-fix behaviour quoted above is the behaviour
/// of *these* sources.
fn rc_tran(tran_args: &str) -> String {
    format!(
        r#"
circuit :rc do
  node :vin, :out
  voltage_source :v1, p: :vin, n: :gnd, dc: 0.V, waveform: pulse(low: 0.V, high: 1.V, delay: 0.s, rise: 10.ns, fall: 10.ns, width: 10.us, period: 20.us)
  resistor :r1, p: :vin, n: :out, value: 1.kohm
  capacitor :c1, p: :out, n: :gnd, value: 100.nF
end

experiment :probe, circuit: :rc do
  tran {tran_args}
  save v(:vin), v(:out)
end
"#
    )
}

// ---------------------------------------------------------------------------
// Shared assertions
// ---------------------------------------------------------------------------

const FINITE_POSITIVE_RULE: &str = "must be a finite number greater than zero";

/// The new `tran_spec` rule fired for `arg` with its primary span on exactly the
/// argument expression `arg_text`.
///
/// Both halves matter: the message names the argument, and the span covers the
/// offending expression — a diagnostic about the whole `tran` line would not
/// tell the user which of four arguments was wrong.
fn assert_rejected_argument<'a>(run: &'a Frontend, arg: &str, arg_text: &str) -> &'a Diagnostic {
    let message = format!("`{arg}:` {FINITE_POSITIVE_RULE}");
    let d = run.error(Code::Value, &message);
    assert_eq!(
        d.message, message,
        "the message is part of the user-facing contract"
    );
    assert_eq!(
        run.primary_text(d),
        arg_text,
        "the primary span must cover the `{arg}:` expression `{arg_text}`, not the whole call:\n{}",
        run.rendered()
    );

    // …and the span must belong to *that* argument, not to another one that
    // happens to hold the same text.
    let prefix = run.primary_prefix(d);
    assert!(
        prefix.trim_end().ends_with(&format!("{arg}:")),
        "the span must start immediately after `{arg}:`; the text before it is {prefix:?}"
    );

    // The rendered form is what `cdsl check` prints, and it is what a user
    // greps for when they were told `output_interval` is wrong.
    assert!(
        run.rendered().contains(arg),
        "the rendered diagnostics must name `{arg}`:\n{}",
        run.rendered()
    );
    d
}

fn assert_rejected_output_interval(run: &Frontend, arg_text: &str) {
    let d = assert_rejected_argument(run, "output_interval", arg_text);
    assert_eq!(
        d.notes.len(),
        1,
        "the `output_interval:` rule explains how to ask for the default:\n{}",
        run.rendered()
    );
    assert!(
        d.notes[0].contains("omit it to keep the solver's own time points"),
        "unexpected note: {:?}",
        d.notes[0]
    );
}

fn assert_rejected_max_step(run: &Frontend, arg_text: &str) {
    assert_rejected_argument(run, "max_step", arg_text);
}

/// No `TranSpec` was built and no default was substituted.
///
/// `tran_spec` returning `None` means the elaborator pushed no `AnalysisTask`,
/// so the experiment has nothing to run and reports `E_ARGUMENT`/“declares no
/// analysis”. Had the front end defaulted the interval instead, a task would
/// exist and this diagnostic would be absent — which is exactly the failure
/// mode the fix exists to remove. The text checks below cover the weaker
/// question of what the user is told: nothing may describe a default or the old
/// `span/1000` fallback.
fn assert_no_plan_was_built(run: &Frontend) {
    let d = run.error(Code::Argument, "declares no analysis");
    assert!(
        run.primary_text(d).contains("probe"),
        "the missing-analysis diagnostic must point at the experiment:\n{}",
        run.rendered()
    );

    let text = run.rendered();
    for forbidden in ["span/1000", "span / 1000", "default", "fallback"] {
        assert!(
            !text.contains(forbidden),
            "a rejected `tran` must not be described as defaulted or fallen back on; \
             found {forbidden:?} in:\n{text}"
        );
    }
}

/// The exact set of errors, so an extra or a missing diagnostic is visible. The
/// counts are deterministic: one `E_VALUE` per rule that fires, plus the
/// `E_ARGUMENT` for the experiment left without an analysis.
fn assert_error_count(run: &Frontend, expected: usize) {
    assert_eq!(
        run.error_count(),
        expected,
        "expected {expected} errors, found {:?}:\n{}",
        run.codes(),
        run.rendered()
    );
}

// ---------------------------------------------------------------------------
// 1. `output_interval:` must be finite and greater than zero
// ---------------------------------------------------------------------------

#[test]
fn output_interval_zero_is_rejected() {
    // Was: `cdsl check` exit 0 with zero diagnostics, `cdsl run` 2015 points and
    // a CSV byte-identical to the `1.ns` run (repro-baseline.md §6).
    let run = rejected(&rc_tran("stop: 2.us, max_step: 1.ns, output_interval: 0.s"));

    assert_rejected_output_interval(&run, "0.s");
    assert_error_count(&run, 2); // E_VALUE + E_ARGUMENT(no analysis)
    assert_no_plan_was_built(&run);
}

#[test]
fn output_interval_negative_is_rejected() {
    // Was: `cdsl check` exit 0, zero diagnostics, exit-0 `cdsl run` whose CSV
    // matched the `1.ns` run bit for bit (repro-baseline.md §5).
    let run = rejected(&rc_tran(
        "stop: 2.us, max_step: 1.ns, output_interval: -1.ns",
    ));

    assert_rejected_output_interval(&run, "-1.ns");
    assert_error_count(&run, 2);
    assert_no_plan_was_built(&run);
}

#[test]
fn output_interval_infinity_is_rejected() {
    // `1e400` overflows the `f64` literal parse to `+inf`; see
    // `non_finite_values_are_reachable_through_ordinary_spellings` below for the
    // lexer's actual return value.
    let run = rejected(&rc_tran(
        "stop: 2.us, max_step: 1.ns, output_interval: 1e400.s",
    ));

    assert_rejected_output_interval(&run, "1e400.s");
    // The pre-existing `check_finite` guard (`elaborate.rs::num_arg`) fires as
    // well, so the value is refused twice over rather than once.
    assert!(
        run.has_error(Code::Value, "`output_interval` is not a finite number"),
        "the pre-existing non-finite guard must still fire:\n{}",
        run.rendered()
    );
    assert_error_count(&run, 3); // check_finite + the rule + E_ARGUMENT
    assert_no_plan_was_built(&run);
}

#[test]
fn output_interval_nan_is_rejected() {
    // `inf - inf` is NaN. The span is the whole expression, which is the
    // argument the user wrote.
    let run = rejected(&rc_tran(
        "stop: 2.us, max_step: 1.ns, output_interval: 1e400.s - 1e400.s",
    ));
    assert_rejected_output_interval(&run, "1e400.s - 1e400.s");
    assert!(
        run.has_error(Code::Value, "`output_interval` is not a finite number"),
        "the pre-existing non-finite guard must still fire:\n{}",
        run.rendered()
    );
    assert_error_count(&run, 3);
    assert_no_plan_was_built(&run);

    // `0 * inf` is NaN as well: a second, independent spelling that reaches the
    // same branch, so the guard is not accidentally keyed to one token sequence.
    let run = rejected(&rc_tran(
        "stop: 2.us, max_step: 1.ns, output_interval: 0.s * 1e400",
    ));
    assert_rejected_output_interval(&run, "0.s * 1e400");
    assert_error_count(&run, 3);
    assert_no_plan_was_built(&run);
}

/// `inf` and NaN are not spellable as literals or built-ins in this language —
/// there is no `inf`/`nan` keyword (`token.rs::KEYWORDS`), the lexer only
/// consumes decimal literals with an optional exponent, and the evaluator's
/// built-ins are `abs, sqrt, min, max, str, pulse, sin, pwl, v, i`
/// (`eval.rs::eval_call`). They are nevertheless **reachable**, and this test
/// records the lexer's and the evaluator's actual answers instead of asserting
/// that the cases above are hypothetical:
///
/// * `1e400` overflows to `+inf` when the numeric text is parsed
///   (`units::build_quantity` uses `f64::from_str`; `lexer.rs::emit_plain_number`
///   documents the same overflow behaviour for plain numbers), and
/// * `1e400.s - 1e400.s` and `0.s * 1e400` evaluate to NaN under the ordinary
///   arithmetic in `eval.rs`.
#[test]
fn non_finite_values_are_reachable_through_ordinary_spellings() {
    let literal = lex_quantity("1e400.s");
    assert_eq!(
        literal.value.to_bits(),
        f64::INFINITY.to_bits(),
        "`1e400.s` must lex to +inf, found {}",
        literal.value
    );
    assert_eq!(literal.text, "1e400.s");

    let inf = eval_output_interval(&rc_tran("stop: 2.us, output_interval: 1e400.s"));
    assert_eq!(
        inf.to_bits(),
        f64::INFINITY.to_bits(),
        "the argument `1e400.s` must evaluate to +inf, found {inf}"
    );

    let nan = eval_output_interval(&rc_tran("stop: 2.us, output_interval: 1e400.s - 1e400.s"));
    assert!(
        nan.is_nan(),
        "`1e400.s - 1e400.s` must evaluate to NaN, found {nan}"
    );

    let nan = eval_output_interval(&rc_tran("stop: 2.us, output_interval: 0.s * 1e400"));
    assert!(
        nan.is_nan(),
        "`0.s * 1e400` must evaluate to NaN, found {nan}"
    );
}

// ---------------------------------------------------------------------------
// 2. Omitting the option, and keeping a legal value exactly as written
// ---------------------------------------------------------------------------

#[test]
fn omitted_output_interval_stays_none() {
    // No default is invented for the omission either: the solver's own time
    // axis is requested by leaving the field `None`.
    let spec = tran_spec_of(&rc_tran("stop: 2.us, max_step: 1.ns"));
    assert_eq!(spec.output_interval, None);
    assert_eq!(spec.max_step, Some(1e-9));
    assert_eq!(spec.start_s, 0.0);
    assert_eq!(spec.stop_s, 2e-6);
}

#[test]
fn legal_output_interval_is_kept_bit_for_bit() {
    // `2.ns` is 2 × the second's nanosecond scale, unchanged: not rescaled,
    // not clamped, not replaced by anything.
    let spec = tran_spec_of(&rc_tran(
        "stop: 2.us, max_step: 1.ns, output_interval: 2.ns",
    ));
    let value = spec
        .output_interval
        .expect("an explicit `output_interval: 2.ns` must be kept");
    assert_eq!(
        value.to_bits(),
        (2.0f64 * 1e-9).to_bits(),
        "expected exactly 2 × 1 ns, found {value:e}"
    );
    assert!(value.is_finite() && value > 0.0);

    // The old fallback was `stop/1000`. On the 2 us circuit that happens to be
    // 2 ns as well, so the discriminating case uses the 20 us probe circuit from
    // repro-baseline.md §5, where the fallback would have been 20 ns.
    let spec = tran_spec_of(&rc_tran(
        "stop: 20.us, max_step: 100.ns, output_interval: 2.ns",
    ));
    // `20 × 1 µs` is not the same double as the literal `2e-5`, so compare the
    // value the elaborator actually builds: the unit scale, applied once.
    assert_eq!(spec.stop_s.to_bits(), (20.0f64 * 1e-6).to_bits());
    assert_eq!(spec.output_interval, Some(2e-9));
    assert_ne!(
        spec.output_interval,
        Some(spec.stop_s / 1000.0),
        "`2.ns` must not be replaced by the old `stop/1000` fallback"
    );
}

// ---------------------------------------------------------------------------
// 3. `max_step:` follows the same rule
// ---------------------------------------------------------------------------

#[test]
fn max_step_must_be_finite_and_greater_than_zero() {
    for (args, arg_text) in [
        ("stop: 2.us, max_step: 0.s", "0.s"),
        ("stop: 2.us, max_step: -1.ns", "-1.ns"),
        ("stop: 2.us, max_step: 1e400.s", "1e400.s"),
    ] {
        let run = rejected(&rc_tran(args));
        assert_rejected_max_step(&run, arg_text);
        assert_no_plan_was_built(&run);
        // Zero and a negative step are refused by the rule alone; an infinite
        // one trips the pre-existing non-finite guard as well.
        let expected = if arg_text.starts_with("1e400") { 3 } else { 2 };
        assert_error_count(&run, expected);
    }
}

#[test]
fn legal_max_step_is_kept_bit_for_bit() {
    let spec = tran_spec_of(&rc_tran("stop: 2.us, max_step: 1.ns"));
    let value = spec
        .max_step
        .expect("an explicit `max_step: 1.ns` must be kept");
    assert_eq!(value.to_bits(), (1.0f64 * 1e-9).to_bits());

    // A non-power-of-ten numeric part, so the assertion covers the unit scale
    // rather than a value the compiler could reproduce from a rounded literal.
    let spec = tran_spec_of(&rc_tran("stop: 2.us, max_step: 250.ps"));
    let value = spec
        .max_step
        .expect("an explicit `max_step: 250.ps` must be kept");
    assert_eq!(value.to_bits(), (250.0f64 * 1e-12).to_bits());
    assert_eq!(spec.output_interval, None);
}

// ---------------------------------------------------------------------------
// 4. The pre-existing `tran` diagnostics still fire
// ---------------------------------------------------------------------------

/// `tran stop:` must exceed `start:` (E_SWEEP) — untouched by this change.
#[test]
fn stop_not_after_start_still_reports_sweep() {
    for args in ["start: 1.us, stop: 1.us", "start: 2.us, stop: 1.us"] {
        let run = rejected(&rc_tran(args));
        let d = run.error(Code::Sweep, "must be greater than");
        assert!(
            run.primary_text(d).starts_with("tran "),
            "the sweep diagnostic points at the call, found {:?}",
            run.primary_text(d)
        );
        // The new rule is not involved: neither option was written.
        assert!(!run.has_error(Code::Value, FINITE_POSITIVE_RULE));
        assert_no_plan_was_built(&run);
        assert_error_count(&run, 2);
    }
}

/// A negative `start:` is still `E_VALUE` with its own message and its own span.
#[test]
fn negative_start_still_reports_its_own_value_error() {
    let run = rejected(&rc_tran("start: -1.ns, stop: 2.us"));

    let d = run.error(Code::Value, "`tran start:` must not be negative");
    assert_eq!(run.primary_text(d), "-1.ns");
    assert!(
        !run.has_error(Code::Value, FINITE_POSITIVE_RULE),
        "the `start:` message must not have been replaced by the interval rule:\n{}",
        run.rendered()
    );
    assert_no_plan_was_built(&run);
    assert_error_count(&run, 2);
}

/// A wrong dimension is still `E_DIMENSION`, reported before any value check.
#[test]
fn wrong_dimension_is_still_a_dimension_error() {
    let run = rejected(&rc_tran("stop: 2.us, output_interval: 1.V"));
    let d = run.error(Code::Dimension, "output_interval");
    assert!(
        d.message.contains("needs s"),
        "unexpected message: {}",
        d.message
    );
    assert_eq!(run.primary_text(d), "1.V");
    assert!(
        !run.has_error(Code::Value, FINITE_POSITIVE_RULE),
        "a dimension error must not be doubled by the value rule:\n{}",
        run.rendered()
    );
    assert_no_plan_was_built(&run);
    assert_error_count(&run, 2);

    let run = rejected(&rc_tran("stop: 2.us, max_step: 1.V"));
    let d = run.error(Code::Dimension, "max_step");
    assert!(
        d.message.contains("needs s"),
        "unexpected message: {}",
        d.message
    );
    assert_eq!(run.primary_text(d), "1.V");
    assert_error_count(&run, 2);
}

/// A missing required `stop:` is still `E_ARGUMENT`, and a non-finite `stop:`
/// is still caught by the pre-existing finite check.
#[test]
fn missing_stop_and_non_finite_stop_keep_their_diagnostics() {
    let run = rejected(&rc_tran("max_step: 1.ns"));
    run.error(Code::Argument, "requires `stop:`");
    assert_no_plan_was_built(&run);
    assert_error_count(&run, 2);

    let run = rejected(&rc_tran("stop: 1e400.s"));
    let d = run.error(Code::Value, "`tran stop:` and `start:` must be finite");
    assert_eq!(run.primary_text(d), "1e400.s");
    assert!(run.has_error(Code::Value, "`stop` is not a finite number"));
    assert_no_plan_was_built(&run);
    assert_error_count(&run, 3);
}

// ---------------------------------------------------------------------------
// 5. The other public entry point agrees
// ---------------------------------------------------------------------------

/// `elaborate_experiment` — the hook the parameter-sweep driver uses — must
/// reject the same source and keep the same `None` for the omission.
#[test]
fn elaborate_experiment_agrees_with_compile() {
    // Rejected: no `Elaborated` at all, so no `plan.tasks` that could carry a
    // defaulted `TranSpec`.
    let src = rc_tran("stop: 2.us, output_interval: 0.s");
    let run = front_end(&src);
    match elaborate_experiment(&run.program, "probe", &[], &Limits::default()) {
        Ok(el) => panic!(
            "`elaborate_experiment` returned a plan with {} task(s) for `output_interval: 0.s`",
            el.plan.tasks.len()
        ),
        Err(d) => {
            let rendered = d.render(&run.sources);
            assert!(rendered.contains("output_interval"), "{rendered}");
            assert!(
                d.errors()
                    .any(|e| e.code == Code::Value && e.message.contains(FINITE_POSITIVE_RULE)),
                "{rendered}"
            );
        }
    }

    // Accepted with the option omitted: `None`, and the same `None` the
    // `compile` path reports.
    let src = rc_tran("stop: 2.us, max_step: 1.ns");
    let run = front_end(&src);
    let el = elaborate_experiment(&run.program, "probe", &[], &Limits::default())
        .unwrap_or_else(|d| panic!("expected success:\n{}", d.render(&run.sources)));
    let spec = match &el.plan.tasks[0].kind {
        AnalysisKind::Tran(spec) => spec.clone(),
        other => panic!("expected a `tran` task, found `{}`", other.name()),
    };
    assert_eq!(spec.output_interval, None);
    assert_eq!(
        spec.output_interval,
        only_tran_spec(&compiled(&src), "probe").output_interval
    );

    // An accepted explicit value: the same bits through both entry points.
    let src = rc_tran("stop: 2.us, max_step: 1.ns, output_interval: 2.ns");
    let run = front_end(&src);
    let el = elaborate_experiment(&run.program, "probe", &[], &Limits::default())
        .unwrap_or_else(|d| panic!("expected success:\n{}", d.render(&run.sources)));
    let spec = match &el.plan.tasks[0].kind {
        AnalysisKind::Tran(spec) => spec.clone(),
        other => panic!("expected a `tran` task, found `{}`", other.name()),
    };
    assert_eq!(
        spec.output_interval.map(f64::to_bits),
        tran_spec_of(&src).output_interval.map(f64::to_bits)
    );
}

// ---------------------------------------------------------------------------
// 6. A legal `tran` is still a legal `tran` (the control for the tests above)
// ---------------------------------------------------------------------------

/// The control that makes `assert_no_plan_was_built` discriminating: when the
/// options are legal, the experiment *does* get its task and the
/// `declares no analysis` diagnostic does not appear.
#[test]
fn legal_tran_builds_exactly_one_task() {
    let src = rc_tran("stop: 2.us, max_step: 1.ns, output_interval: 2.ns");
    let run = front_end(&src);
    assert!(
        run.result.is_ok(),
        "expected success, got diagnostics:\n{}",
        match &run.result {
            Ok(_) => String::new(),
            Err(d) => d.render(&run.sources),
        }
    );
    let spec = tran_spec_of(&src);
    assert_eq!(spec.output_interval, Some(2e-9));
    assert_eq!(spec.max_step, Some(1e-9));
    assert_eq!(spec.stop_s, 2e-6);
    assert!(!spec.uic, "the language does not expose `uic` yet");
}
