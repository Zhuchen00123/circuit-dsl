//! Round-4 QA: R4-03 export diagnostics, from a hand-built Dataset.
//!
//! The point of building the dataset by hand is independence: the R4-01 fix
//! may make an illegal expression unreachable, so a test that relies on one
//! would stop covering the export path. Here the non-finite samples come from
//! raw backend data, exactly the documented case.
//!
//! Contract: docs/review-evidence/round4/design-contract.md §1.4 — the
//! documented empty-cell/null rendering is unchanged, and the renderer returns
//! the warnings instead of dropping them.

use circuit_core::Limits;
use circuit_core::units::VOLTAGE;
use circuit_results::dataset::{Axis, BackendInfo, Complex, Dataset, Signal};
use circuit_results::{to_csv_with_diagnostics, to_json_with_diagnostics};

/// A transient result whose values include every non-finite shape.
///
/// axis: t = 0, 1e-3, 2e-3, 3e-3 s (all finite, so no axis warning).
/// v(out): 1, NaN, +inf, -inf.
/// v(mid): (0,1), (inf,0), (NaN,NaN), (1,0).
fn non_finite_dataset() -> Dataset {
    Dataset::new(
        "qa_r4_export",
        "tran1",
        "tran",
        Axis::Time(vec![0.0, 1e-3, 2e-3, 3e-3]),
        vec![
            Signal::real(
                "v(out)",
                VOLTAGE,
                vec![1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY],
            ),
            Signal::complex(
                "v(mid)",
                VOLTAGE,
                vec![
                    Complex::new(0.0, 1.0),
                    Complex::new(f64::INFINITY, 0.0),
                    Complex::new(f64::NAN, f64::NAN),
                    Complex::new(1.0, 0.0),
                ],
            ),
        ],
        BackendInfo::new("qa", "0"),
        &Limits::default(),
    )
    .expect("a hand-built dataset with non-finite samples is legal")
}

/// CSV: non-finite samples stay empty fields, the file remains rectangular,
/// and the renderer returns one warning per affected signal.
#[test]
fn csv_keeps_the_documented_empty_cell_and_returns_warnings() {
    let export = to_csv_with_diagnostics(&non_finite_dataset()).expect("the renderer is total");

    assert!(export.has_warnings(), "non-finite data must warn");
    assert_eq!(export.diagnostics.len(), 2, "{:?}", export.diagnostics);

    let lines: Vec<&str> = export.text.lines().collect();
    assert_eq!(lines[0], "time,v(out),v(mid)_re,v(mid)_im");
    assert_eq!(lines.len(), 5, "header plus four rows:\n{}", export.text);
    // Row 0 is fully finite: 0 s, 1 V, (0 + 1j) V.
    assert_eq!(lines[1], "0,1,0,1");
    // Row 1: v(out) = NaN -> empty; v(mid) = (inf, 0) -> empty real, 0 imag.
    assert_eq!(lines[2], "0.001,,,0");
    // Row 2: v(out) = +inf -> empty; v(mid) = (NaN, NaN) -> two empties.
    assert_eq!(lines[3], "0.002,,,");
    // Row 3: v(out) = -inf -> empty; v(mid) = (1, 0) -> finite again.
    assert_eq!(lines[4], "0.003,,1,0");
    // The file never spells a non-finite value out.
    for forbidden in ["NaN", "nan", "inf", "-inf", "Infinity"] {
        assert!(
            !export.text.contains(forbidden),
            "CSV must not contain {forbidden}:\n{}",
            export.text
        );
    }

    let warns: Vec<String> = export
        .diagnostics
        .iter()
        .map(|d| d.render_plain())
        .collect();
    assert!(
        warns.iter().any(|w| w.contains("v(out)")),
        "one warning must name v(out): {warns:?}"
    );
    assert!(
        warns.iter().any(|w| w.contains("v(mid)")),
        "one warning must name v(mid): {warns:?}"
    );
    for w in &warns {
        assert!(
            w.starts_with("warning["),
            "a missing value is a warning: {w}"
        );
    }
}

/// JSON: non-finite samples become null, complex components are converted
/// independently, and the renderer returns the same set of warnings.
#[test]
fn json_keeps_null_and_returns_warnings() {
    let export = to_json_with_diagnostics(&non_finite_dataset()).expect("the renderer is total");
    assert!(export.has_warnings());
    assert_eq!(export.diagnostics.len(), 2);

    // This crate does not depend on a JSON parser, so the rendering is checked
    // by inspecting the document section by section rather than by parsing it.
    let text = &export.text;
    assert!(
        text.contains("\"schema\": \"circuit-dsl.result/1\""),
        "{text}"
    );
    assert!(text.contains("\"name\": \"v(out)\""), "{text}");

    let vout = signal_block(text, "\"name\": \"v(out)\"");
    assert!(
        vout.contains("1.0"),
        "the finite sample must stay a number: {vout}"
    );
    assert_eq!(
        vout.matches("null").count(),
        3,
        "NaN, +inf and -inf must each be null: {vout}"
    );
    let first_value = vout.find("1.0").expect("a finite sample");
    let first_null = vout.find("null").expect("a null");
    assert!(
        first_value < first_null,
        "the finite sample comes before the missing ones: {vout}"
    );

    let vmid = signal_block(text, "\"name\": \"v(mid)\"");
    assert!(
        vmid.contains("\"im\": 0.0"),
        "a finite component keeps its value: {vmid}"
    );
    assert_eq!(
        vmid.matches("null").count(),
        3,
        "the infinite real part and the NaN pair must be null: {vmid}"
    );

    // The file's own diagnostics array describes the dataset, not the export.
    assert!(
        text.contains("\"diagnostics\": []"),
        "a hand-built dataset carries no diagnostics of its own: {text}"
    );

    for forbidden in ["NaN", "Infinity"] {
        assert!(
            !text.contains(forbidden),
            "JSON must not contain {forbidden}"
        );
    }
}

/// The text of one signal object inside a pretty-printed JSON document: from
/// its name to the next signal's name.
fn signal_block(text: &str, marker: &str) -> String {
    let start = text
        .find(marker)
        .unwrap_or_else(|| panic!("no {marker} in:\n{text}"));
    let rest = &text[start + marker.len()..];
    let end = match rest.find("\"name\":") {
        Some(offset) => start + marker.len() + offset,
        None => text.len(),
    };
    text[start..end].to_string()
}

/// Both formats must raise the *same* warnings for one dataset: otherwise the
/// session writer cannot deduplicate them and a CSV+JSON run would report two
/// different sets.
#[test]
fn both_renderers_report_the_same_warnings_for_one_dataset() {
    let ds = non_finite_dataset();
    let csv = to_csv_with_diagnostics(&ds).expect("csv");
    let json = to_json_with_diagnostics(&ds).expect("json");

    let csv_warnings: Vec<String> = csv.diagnostics.iter().map(|d| d.render_plain()).collect();
    let json_warnings: Vec<String> = json.diagnostics.iter().map(|d| d.render_plain()).collect();
    assert!(!csv_warnings.is_empty(), "the fixture must warn");
    assert_eq!(
        csv_warnings, json_warnings,
        "CSV and JSON must describe the same missing values"
    );
}

/// A fully finite dataset keeps producing no warnings at all: the export path
/// must not become noisy.
#[test]
fn a_finite_dataset_has_no_warnings() {
    let ds = Dataset::new(
        "qa_r4_export",
        "op1",
        "op",
        Axis::None,
        vec![Signal::real("v(out)", VOLTAGE, vec![3.0])],
        BackendInfo::new("qa", "0"),
        &Limits::default(),
    )
    .expect("well formed");

    let csv = to_csv_with_diagnostics(&ds).expect("csv");
    assert!(!csv.has_warnings(), "{:?}", csv.diagnostics);
    assert_eq!(csv.text, "v(out)\n3\n");

    let json = to_json_with_diagnostics(&ds).expect("json");
    assert!(!json.has_warnings(), "{:?}", json.diagnostics);
}

// ---------------------------------------------------------------------------
// The session writer keeps the warnings (contract 1.4)
// ---------------------------------------------------------------------------

use std::path::{Path, PathBuf};

use circuit_core::diagnostic::{Code, Diagnostic, Diagnostics};
use circuit_session::{Format, write_datasets};

/// A per-test scratch directory under target/round4/qa.
fn scratch(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("round4")
        .join("qa")
        .join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the QA scratch directory");
    dir
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// The session writer used to drop the renderer's warnings. Written both
/// formats of one dataset: two files, one warning per affected signal, and the
/// files are exactly what the renderers produce.
#[test]
fn write_datasets_returns_the_files_and_the_warnings_once() {
    let dir = scratch("r4_export_write");
    let ds = non_finite_dataset();

    let written = write_datasets(&dir, Format::Both, std::slice::from_ref(&ds), &mut |_| {
        Ok(())
    })
    .expect("a non-finite sample is a warning, not a failure");

    assert_eq!(written.paths.len(), 2, "{:?}", written.paths);
    assert_eq!(
        written.warnings.len(),
        2,
        "one warning per affected signal, deduplicated across CSV and JSON: {:?}",
        written.warnings
    );

    let csv = to_csv_with_diagnostics(&ds).expect("csv");
    let json = to_json_with_diagnostics(&ds).expect("json");
    let csv_path = written
        .paths
        .iter()
        .find(|p| p.extension().and_then(|e| e.to_str()) == Some("csv"))
        .expect("a csv path");
    let json_path = written
        .paths
        .iter()
        .find(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .expect("a json path");
    assert_eq!(
        std::fs::read_to_string(csv_path).expect("read csv"),
        csv.text,
        "the written CSV must be the renderer's text"
    );
    assert_eq!(
        std::fs::read_to_string(json_path).expect("read json"),
        json.text
    );

    // The documented rendering really is on disk: an empty cell and a null.
    assert!(csv.text.contains("0.001,,,0"), "{}", csv.text);
    assert!(json.text.contains("null"), "{}", json.text);
    assert!(json.text.contains("\"diagnostics\": []"), "{}", json.text);

    // The returned warnings are the renderers' warnings, once each.
    let returned: Vec<String> = written.warnings.iter().map(|w| w.render_plain()).collect();
    let expected: Vec<String> = csv.diagnostics.iter().map(|w| w.render_plain()).collect();
    assert_eq!(returned, expected);

    // The lines a front end prints: indented like the run summary, one per
    // warning, and never a duplicated pair for CSV+JSON.
    let lines = written.warning_lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    for line in &lines {
        assert!(
            line.starts_with("  warning["),
            "user-visible shape: {line:?}"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// A finite dataset stays silent: the new return value must not turn every run
/// into a warning list.
#[test]
fn write_datasets_is_silent_for_a_finite_dataset() {
    let dir = scratch("r4_export_write_finite");
    let ds = Dataset::new(
        "qa_finite",
        "op1",
        "op",
        Axis::None,
        vec![Signal::real("v(out)", VOLTAGE, vec![3.0])],
        BackendInfo::new("qa", "0"),
        &Limits::default(),
    )
    .expect("well formed");

    let written = write_datasets(&dir, Format::Both, &[ds], &mut |_| Ok(()))
        .expect("a finite dataset writes cleanly");
    assert_eq!(written.paths.len(), 2, "{:?}", written.paths);
    assert!(written.warnings.is_empty(), "{:?}", written.warnings);
    assert!(written.warning_lines().is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

/// The approve callback keeps its meaning: refusing a path stops the write.
#[test]
fn write_datasets_writes_nothing_when_the_caller_refuses() {
    let dir = scratch("r4_export_write_refused");
    let ds = non_finite_dataset();
    let mut asked = 0;

    let result = write_datasets(&dir, Format::Both, &[ds], &mut |_| {
        asked += 1;
        Err(Diagnostics::single(Diagnostic::error(
            Code::Io,
            "refused by the test",
        )))
    });

    assert!(result.is_err(), "a refusal must be reported");
    assert_eq!(
        asked, 1,
        "the first path is asked about before anything is written"
    );
    assert!(
        files_in(&dir).is_empty(),
        "a refused write leaves no file: {:?}",
        files_in(&dir)
    );

    let _ = std::fs::remove_dir_all(&dir);
}
