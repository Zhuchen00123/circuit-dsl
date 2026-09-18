//! Exporters: CSV and JSON.
//!
//! Both exporters are total over well-formed datasets and never emit invalid
//! output:
//!
//! - **CSV** writes one column per axis value followed by one column per
//!   signal; a complex signal is split into `<name>_re` and `<name>_im`
//!   (spec §6). A non-finite sample becomes an **empty field**, never the text
//!   `NaN` or `inf`.
//! - **JSON** keeps the experiment, analysis, kind, axis type and unit, every
//!   signal's name and unit, and the backend metadata. A non-finite sample
//!   becomes `null`; `serde_json` cannot represent `NaN` or `inf` and would
//!   otherwise produce output no parser accepts.
//!
//! Both produce a [`Diagnostic`] warning per affected signal so that "the
//! number is missing" is visible rather than silent (spec §6).

use std::fmt;

use circuit_core::{Code, Diagnostic};
use serde_json::{Map, Value as Json};

use crate::dataset::{Axis, BackendInfo, Data, Dataset, Signal};
use crate::{Complex, format_number};

/// The rendered text plus the warnings produced while rendering.
///
/// The warnings are *not* errors: the file is complete and well formed, but it
/// is missing values the caller should be told about.
#[derive(Clone, Debug)]
pub struct Export {
    pub text: String,
    pub diagnostics: Vec<Diagnostic>,
}

impl Export {
    pub fn new(text: impl Into<String>, diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            text: text.into(),
            diagnostics,
        }
    }

    /// Whether anything was rendered as empty/null.
    pub fn has_warnings(&self) -> bool {
        !self.diagnostics.is_empty()
    }

    pub fn into_text(self) -> String {
        self.text
    }
}

// ---------------------------------------------------------------------------
// CSV
// ---------------------------------------------------------------------------

/// Render the dataset as CSV.
pub fn to_csv(dataset: &Dataset) -> Result<String, Diagnostic> {
    Ok(to_csv_with_diagnostics(dataset)?.text)
}

/// Render the dataset as CSV, keeping the non-finite warnings.
pub fn to_csv_with_diagnostics(dataset: &Dataset) -> Result<Export, Diagnostic> {
    let (axis_name, axis_values) = axis_column(dataset);
    let rows = match dataset.axis {
        Axis::None => 1,
        _ => axis_values.len(),
    };

    let mut header: Vec<String> = Vec::new();
    if let Some(name) = axis_name {
        header.push(quote_field(name));
    }
    for signal in &dataset.signals {
        header.extend(signal_columns(signal));
    }

    let mut text = String::new();
    if !header.is_empty() {
        text.push_str(&header.join(","));
        text.push('\n');
        for row in 0..rows {
            let mut fields: Vec<String> = Vec::new();
            if axis_name.is_some() {
                fields.push(cell(axis_values.get(row).copied()));
            }
            for signal in &dataset.signals {
                match &signal.data {
                    Data::Real(values) => fields.push(cell(values.get(row).copied())),
                    Data::Complex(values) => match values.get(row) {
                        Some(z) => {
                            fields.push(cell(Some(z.re)));
                            fields.push(cell(Some(z.im)));
                        }
                        None => {
                            fields.push(String::new());
                            fields.push(String::new());
                        }
                    },
                }
            }
            text.push_str(&fields.join(","));
            text.push('\n');
        }
    }

    Ok(Export::new(text, non_finite_diagnostics(dataset)))
}

/// The axis column name and values; `None` for an operating point.
fn axis_column(dataset: &Dataset) -> (Option<&'static str>, &[f64]) {
    match &dataset.axis {
        Axis::None => (None, &[]),
        other => (Some(other.kind_name()), other.samples()),
    }
}

/// Column headers contributed by one signal.
fn signal_columns(signal: &Signal) -> Vec<String> {
    match signal.data {
        Data::Real(_) => vec![quote_field(&signal.name)],
        Data::Complex(_) => vec![
            quote_field(&format!("{}_re", signal.name)),
            quote_field(&format!("{}_im", signal.name)),
        ],
    }
}

/// Quote a header when RFC 4180 requires it.
///
/// This is not hypothetical: a differential probe is named `v(a,b)` and its
/// name contains a comma, so an unquoted header would silently shift every
/// column to its right.
fn quote_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

/// One CSV field. Non-finite values are empty, never `NaN` or `inf` text.
fn cell(value: Option<f64>) -> String {
    match value {
        Some(x) if x.is_finite() => format_number(x),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

/// Render the dataset as pretty-printed JSON.
pub fn to_json(dataset: &Dataset) -> Result<String, Diagnostic> {
    Ok(to_json_with_diagnostics(dataset)?.text)
}

/// Render the dataset as pretty-printed JSON, keeping the warnings.
pub fn to_json_with_diagnostics(dataset: &Dataset) -> Result<Export, Diagnostic> {
    let value = to_json_value(dataset)?;
    let text = serde_json::to_string_pretty(&value).map_err(|e| {
        Diagnostic::error(
            Code::Io,
            format!("could not serialise the result as JSON: {e}"),
        )
    })?;
    Ok(Export::new(text, non_finite_diagnostics(dataset)))
}

/// Build the JSON structure without rendering it.
///
/// Exposed separately so a caller can embed a result in a larger document.
pub fn to_json_value(dataset: &Dataset) -> Result<Json, Diagnostic> {
    let mut root = Map::new();
    root.insert("schema".to_string(), Json::from(SCHEMA));
    root.insert(
        "experiment".to_string(),
        Json::from(dataset.experiment.as_str()),
    );
    root.insert(
        "analysis".to_string(),
        Json::from(dataset.analysis.as_str()),
    );
    root.insert("kind".to_string(), Json::from(dataset.kind.as_str()));
    root.insert("axis".to_string(), axis_json(&dataset.axis));
    root.insert(
        "signals".to_string(),
        Json::Array(dataset.signals.iter().map(signal_json).collect()),
    );
    root.insert("backend".to_string(), backend_json(&dataset.backend));
    root.insert(
        "diagnostics".to_string(),
        Json::Array(dataset.diagnostics.iter().map(diagnostic_json).collect()),
    );
    Ok(Json::Object(root))
}

/// The schema identifier written into every JSON result.
pub const SCHEMA: &str = "circuit-dsl.result/1";

fn axis_json(axis: &Axis) -> Json {
    match axis {
        Axis::None => Json::Object(Map::from_iter([("type".to_string(), Json::from("none"))])),
        other => {
            let mut object = Map::new();
            object.insert("type".to_string(), Json::from(other.kind_name()));
            object.insert(
                "unit".to_string(),
                match other.unit() {
                    Some(unit) => Json::from(unit.to_string()),
                    // A swept parameter can be ohms, volts, anything: the
                    // dataset does not carry which, so it is not claimed.
                    None => Json::Null,
                },
            );
            object.insert(
                "values".to_string(),
                Json::Array(other.samples().iter().map(|x| number(*x)).collect()),
            );
            Json::Object(object)
        }
    }
}

fn signal_json(signal: &Signal) -> Json {
    let mut object = Map::new();
    object.insert("name".to_string(), Json::from(signal.name.as_str()));
    object.insert("unit".to_string(), Json::from(signal.unit.to_string()));
    match &signal.data {
        Data::Real(values) => {
            object.insert("type".to_string(), Json::from("real"));
            object.insert(
                "values".to_string(),
                Json::Array(values.iter().map(|x| number(*x)).collect()),
            );
        }
        Data::Complex(values) => {
            object.insert("type".to_string(), Json::from("complex"));
            object.insert(
                "values".to_string(),
                Json::Array(values.iter().map(complex_json).collect()),
            );
        }
    }
    Json::Object(object)
}

/// A complex sample as `{"re": .., "im": ..}`.
///
/// Each component is converted independently, so a sample whose imaginary
/// part is `inf` keeps its real part rather than losing the whole sample.
fn complex_json(z: &Complex) -> Json {
    let mut object = Map::new();
    object.insert("re".to_string(), number(z.re));
    object.insert("im".to_string(), number(z.im));
    Json::Object(object)
}

fn backend_json(backend: &BackendInfo) -> Json {
    let mut object = Map::new();
    object.insert("name".to_string(), Json::from(backend.name.as_str()));
    object.insert("version".to_string(), Json::from(backend.version.as_str()));
    object.insert(
        "settings".to_string(),
        Json::Array(
            backend
                .settings
                .iter()
                .map(|(key, value)| {
                    let mut pair = Map::new();
                    pair.insert("key".to_string(), Json::from(key.as_str()));
                    pair.insert("value".to_string(), Json::from(value.as_str()));
                    Json::Object(pair)
                })
                .collect(),
        ),
    );
    Json::Object(object)
}

fn diagnostic_json(diagnostic: &Diagnostic) -> Json {
    let mut object = Map::new();
    object.insert(
        "severity".to_string(),
        Json::from(diagnostic.severity.as_str()),
    );
    object.insert("code".to_string(), Json::from(diagnostic.code.as_str()));
    object.insert(
        "message".to_string(),
        Json::from(diagnostic.message.as_str()),
    );
    object.insert(
        "notes".to_string(),
        Json::Array(
            diagnostic
                .notes
                .iter()
                .map(|n| Json::from(n.as_str()))
                .collect(),
        ),
    );
    object.insert(
        "context".to_string(),
        Json::Array(
            diagnostic
                .context
                .iter()
                .map(|(key, value)| {
                    let mut pair = Map::new();
                    pair.insert("key".to_string(), Json::from(key.as_str()));
                    pair.insert("value".to_string(), Json::from(value.as_str()));
                    Json::Object(pair)
                })
                .collect(),
        ),
    );
    Json::Object(object)
}

/// `null` for `NaN` and the infinities: they are not JSON numbers.
fn number(x: f64) -> Json {
    serde_json::Number::from_f64(x)
        .map(Json::Number)
        .unwrap_or(Json::Null)
}

// ---------------------------------------------------------------------------
// Non-finite warnings
// ---------------------------------------------------------------------------

/// One warning per signal (and one for the axis) that contains non-finite
/// samples.
///
/// Both exporters render these values as empty/null; this is what tells the
/// user which numbers are missing and where.
pub fn non_finite_diagnostics(dataset: &Dataset) -> Vec<Diagnostic> {
    let mut out = Vec::new();

    if !dataset.axis.is_none() {
        let bad: Vec<usize> = dataset
            .axis
            .samples()
            .iter()
            .enumerate()
            .filter(|(_, x)| !x.is_finite())
            .map(|(i, _)| i)
            .collect();
        if !bad.is_empty() {
            out.push(
                Diagnostic::warning(
                    Code::Value,
                    format!(
                        "the {} axis has {} non-finite sample(s)",
                        dataset.axis.kind_name(),
                        bad.len()
                    ),
                )
                .with_context("analysis", dataset.analysis.clone())
                .with_note(format!("sample indices: {}", index_preview(&bad)))
                .with_note("exported as an empty CSV field and as JSON null"),
            );
        }
    }

    for signal in &dataset.signals {
        let bad = signal.data.non_finite_indices();
        if bad.is_empty() {
            continue;
        }
        out.push(
            Diagnostic::warning(
                Code::Value,
                format!(
                    "signal `{}` has {} non-finite sample(s)",
                    signal.name,
                    bad.len()
                ),
            )
            .with_context("analysis", dataset.analysis.clone())
            .with_note(format!("sample indices: {}", index_preview(&bad)))
            .with_note("exported as an empty CSV field and as JSON null"),
        );
    }

    out
}

/// A short list of indices; long runs are abbreviated.
fn index_preview(indices: &[usize]) -> String {
    const MAX: usize = 8;
    let mut text = indices
        .iter()
        .take(MAX)
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    if indices.len() > MAX {
        text.push_str(", ...");
    }
    text
}

impl fmt::Display for Export {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::Signal;
    use circuit_core::Limits;
    use circuit_core::units::{CURRENT, VOLTAGE};

    fn limits() -> Limits {
        Limits::default()
    }

    fn backend() -> BackendInfo {
        BackendInfo::new("thevenin", "0.5.0").with_setting("max_step", "50ns")
    }

    /// A transient result with one real and one complex signal.
    fn tran_dataset() -> Dataset {
        Dataset::new(
            "response",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1e-3, 2e-3]),
            vec![
                Signal::real("v(out)", VOLTAGE, vec![0.0, 0.5, 1.0]),
                Signal::complex(
                    "i(r1)",
                    CURRENT,
                    vec![
                        Complex::new(1.0, -1.0),
                        Complex::new(2.0, -2.0),
                        Complex::new(3.0, -3.0),
                    ],
                ),
            ],
            backend(),
            &limits(),
        )
        .expect("well formed")
    }

    fn op_dataset() -> Dataset {
        Dataset::new(
            "bias",
            "op1",
            "op",
            Axis::None,
            vec![Signal::real("v(out)", VOLTAGE, vec![1.25])],
            backend(),
            &limits(),
        )
        .expect("well formed")
    }

    // -- CSV ---------------------------------------------------------------

    #[test]
    fn csv_has_one_column_per_axis_and_signal() {
        let csv = to_csv(&tran_dataset()).expect("csv");
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(
            lines[0], "time,v(out),i(r1)_re,i(r1)_im",
            "axis first, complex signals split into _re/_im"
        );
        assert_eq!(lines.len(), 4, "header plus one row per sample");
        assert_eq!(lines[1], "0,0,1,-1");
        assert_eq!(lines[2], "0.001,0.5,2,-2");
        assert_eq!(lines[3], "0.002,1,3,-3");
    }

    #[test]
    fn csv_of_an_operating_point_has_no_axis_column() {
        let csv = to_csv(&op_dataset()).expect("csv");
        assert_eq!(csv, "v(out)\n1.25\n");
    }

    #[test]
    fn csv_writes_non_finite_values_as_empty_fields() {
        let ds = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1.0, 2.0, 3.0]),
            vec![Signal::real(
                "v(out)",
                VOLTAGE,
                vec![1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY],
            )],
            backend(),
            &limits(),
        )
        .expect("well formed");

        let export = to_csv_with_diagnostics(&ds).expect("csv");
        assert_eq!(export.text, "time,v(out)\n0,1\n1,\n2,\n3,\n");
        for forbidden in ["NaN", "nan", "inf", "Inf", "infinity"] {
            assert!(
                !export.text.contains(forbidden),
                "CSV must not contain `{forbidden}`:\n{}",
                export.text
            );
        }
        assert!(export.has_warnings());
        let warning = export.diagnostics.first().expect("one warning");
        assert_eq!(warning.severity, circuit_core::Severity::Warning);
        assert_eq!(warning.code, Code::Value);
        assert!(warning.message.contains("v(out)"), "{}", warning.message);
        assert!(warning.message.contains('3'), "{}", warning.message);
        assert!(
            warning.notes.iter().any(|n| n.contains("1, 2, 3")),
            "{:?}",
            warning.notes
        );
    }

    #[test]
    fn csv_of_an_empty_sweep_is_just_the_header() {
        let ds = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(Vec::new()),
            Vec::new(),
            backend(),
            &limits(),
        )
        .expect("well formed");
        assert_eq!(to_csv(&ds).expect("csv"), "time\n");

        // With no axis and no signals there is nothing to write at all.
        let ds = Dataset::new(
            "exp",
            "op1",
            "op",
            Axis::None,
            Vec::new(),
            backend(),
            &limits(),
        )
        .expect("well formed");
        assert_eq!(to_csv(&ds).expect("csv"), "");
    }

    #[test]
    fn csv_quotes_probe_names_that_contain_commas() {
        // A differential probe is named `v(a,b)`, so this is a real case and
        // not a hypothetical one.
        let ds = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1.0]),
            vec![
                Signal::real("v(a,b)", VOLTAGE, vec![0.5, 1.5]),
                Signal::real("v(out)", VOLTAGE, vec![0.0, 1.0]),
            ],
            backend(),
            &limits(),
        )
        .expect("well formed");
        let csv = to_csv(&ds).expect("csv");
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "time,\"v(a,b)\",v(out)");
        assert_eq!(lines[1], "0,0.5,0");
        assert_eq!(lines[2], "1,1.5,1");
    }

    #[test]
    fn csv_keeps_extreme_magnitudes_parseable() {
        let ds = Dataset::new(
            "exp",
            "op1",
            "op",
            Axis::None,
            vec![Signal::real("v(out)", VOLTAGE, vec![1e-300])],
            backend(),
            &limits(),
        )
        .expect("well formed");
        let csv = to_csv(&ds).expect("csv");
        let field = csv.lines().nth(1).expect("one data row");
        let parsed: f64 = field.parse().expect("the field must parse back");
        assert!(parsed > 0.0 && parsed < 1e-290, "{field}");
    }

    // -- JSON --------------------------------------------------------------

    #[test]
    fn json_preserves_metadata_units_and_axis() {
        let text = to_json(&tran_dataset()).expect("json");
        let parsed: Json = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(parsed["schema"], Json::from(SCHEMA));
        assert_eq!(parsed["experiment"], Json::from("response"));
        assert_eq!(parsed["analysis"], Json::from("tran1"));
        assert_eq!(parsed["kind"], Json::from("tran"));
        assert_eq!(parsed["axis"]["type"], Json::from("time"));
        assert_eq!(parsed["axis"]["unit"], Json::from("s"));
        assert_eq!(parsed["axis"]["values"][2], Json::from(0.002));

        let signals = parsed["signals"].as_array().expect("signals array");
        assert_eq!(signals.len(), 2);
        assert_eq!(signals[0]["name"], Json::from("v(out)"));
        assert_eq!(signals[0]["unit"], Json::from("V"));
        assert_eq!(signals[0]["type"], Json::from("real"));
        assert_eq!(signals[0]["values"][1], Json::from(0.5));
        assert_eq!(signals[1]["name"], Json::from("i(r1)"));
        assert_eq!(signals[1]["unit"], Json::from("A"));
        assert_eq!(signals[1]["type"], Json::from("complex"));
        assert_eq!(signals[1]["values"][0]["re"], Json::from(1.0));
        assert_eq!(signals[1]["values"][0]["im"], Json::from(-1.0));

        assert_eq!(parsed["backend"]["name"], Json::from("thevenin"));
        assert_eq!(parsed["backend"]["version"], Json::from("0.5.0"));
        assert_eq!(
            parsed["backend"]["settings"][0]["key"],
            Json::from("max_step")
        );
        assert_eq!(
            parsed["backend"]["settings"][0]["value"],
            Json::from("50ns")
        );
    }

    #[test]
    fn json_serialises_nan_and_infinity_as_null_and_round_trips() {
        let ds = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1.0, 2.0, 3.0]),
            vec![
                Signal::real(
                    "v(out)",
                    VOLTAGE,
                    vec![f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1.0],
                ),
                Signal::complex(
                    "v(in)",
                    VOLTAGE,
                    vec![
                        Complex::new(f64::INFINITY, 1.0),
                        Complex::new(1.0, f64::NAN),
                        Complex::new(2.0, 3.0),
                        Complex::new(-1.0, 0.0),
                    ],
                ),
            ],
            backend(),
            &limits(),
        )
        .expect("well formed");

        let export = to_json_with_diagnostics(&ds).expect("json");
        // The text must be parseable by a strict JSON parser.
        let parsed: Json = serde_json::from_str(&export.text).expect("valid JSON");
        let values = parsed["signals"][0]["values"].as_array().expect("array");
        assert_eq!(values.len(), 4);
        assert_eq!(values[0], Json::Null, "NaN becomes null");
        assert_eq!(values[1], Json::Null, "+inf becomes null");
        assert_eq!(values[2], Json::Null, "-inf becomes null");
        assert_eq!(values[3], Json::from(1.0));

        let complex = parsed["signals"][1]["values"].as_array().expect("array");
        assert_eq!(complex[0]["re"], Json::Null, "non-finite real part");
        assert_eq!(complex[0]["im"], Json::from(1.0), "finite part is kept");
        assert_eq!(complex[1]["re"], Json::from(1.0));
        assert_eq!(complex[1]["im"], Json::Null, "non-finite imaginary part");
        assert_eq!(complex[2]["re"], Json::from(2.0));

        for forbidden in ["NaN", "Infinity", "-Infinity"] {
            assert!(
                !export.text.contains(forbidden),
                "JSON must not contain `{forbidden}`:\n{}",
                export.text
            );
        }
    }

    #[test]
    fn json_is_valid_even_when_every_sample_is_non_finite() {
        let ds = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, 1.0]),
            vec![Signal::real(
                "v(out)",
                VOLTAGE,
                vec![f64::NAN, f64::INFINITY],
            )],
            backend(),
            &limits(),
        )
        .expect("well formed");
        let export = to_json_with_diagnostics(&ds).expect("json");
        let parsed: Json = serde_json::from_str(&export.text).expect("still valid JSON");

        // The rest of the document survives: name, unit and backend metadata.
        assert_eq!(parsed["signals"][0]["name"], Json::from("v(out)"));
        assert_eq!(parsed["signals"][0]["unit"], Json::from("V"));
        assert_eq!(parsed["backend"]["name"], Json::from("thevenin"));
        assert_eq!(
            parsed["signals"][0]["values"],
            Json::Array(vec![Json::Null, Json::Null])
        );

        // A non-finite axis value is null too.
        let ds = Dataset::new(
            "exp",
            "tran1",
            "tran",
            Axis::Time(vec![0.0, f64::NAN]),
            vec![Signal::real("v(out)", VOLTAGE, vec![0.0, 1.0])],
            backend(),
            &limits(),
        )
        .expect("well formed");
        let parsed: Json = serde_json::from_str(&to_json(&ds).expect("json")).expect("valid");
        assert_eq!(parsed["axis"]["values"][1], Json::Null);
    }

    #[test]
    fn json_of_an_operating_point_has_no_axis_values() {
        let parsed: Json =
            serde_json::from_str(&to_json(&op_dataset()).expect("json")).expect("valid JSON");
        assert_eq!(parsed["axis"]["type"], Json::from("none"));
        assert!(parsed["axis"].get("values").is_none());
        assert_eq!(parsed["signals"][0]["values"][0], Json::from(1.25));
        assert_eq!(parsed["kind"], Json::from("op"));
    }

    #[test]
    fn json_reports_result_diagnostics() {
        let mut ds = tran_dataset();
        ds.push_diagnostic(
            Diagnostic::warning(Code::Backend, "solver reduced the step").with_note("at 1 ms"),
        );
        let parsed: Json = serde_json::from_str(&to_json(&ds).expect("json")).expect("valid JSON");
        assert_eq!(parsed["diagnostics"][0]["severity"], Json::from("warning"));
        assert_eq!(parsed["diagnostics"][0]["code"], Json::from("E_BACKEND"));
        assert_eq!(
            parsed["diagnostics"][0]["message"],
            Json::from("solver reduced the step")
        );
        assert_eq!(parsed["diagnostics"][0]["notes"][0], Json::from("at 1 ms"));
    }

    #[test]
    fn parameter_axis_has_no_unit_but_keeps_its_values() {
        let ds = Dataset::new(
            "exp",
            "dc1",
            "dc",
            Axis::Parameter(vec![0.0, 1e3, 2e3]),
            vec![Signal::real("v(out)", VOLTAGE, vec![0.0, 1.0, 2.0])],
            backend(),
            &limits(),
        )
        .expect("well formed");
        let parsed: Json = serde_json::from_str(&to_json(&ds).expect("json")).expect("valid JSON");
        assert_eq!(parsed["axis"]["type"], Json::from("parameter"));
        assert_eq!(parsed["axis"]["unit"], Json::Null);
        assert_eq!(parsed["axis"]["values"][1], Json::from(1000.0));

        let csv = to_csv(&ds).expect("csv");
        assert_eq!(csv.lines().next().expect("header"), "parameter,v(out)");
    }

    #[test]
    fn clean_datasets_produce_no_warnings() {
        let export = to_json_with_diagnostics(&tran_dataset()).expect("json");
        assert!(!export.has_warnings());
        assert!(non_finite_diagnostics(&tran_dataset()).is_empty());
        assert_eq!(export.clone().into_text(), export.text);
    }

    #[test]
    fn complex_division_by_zero_is_not_hidden() {
        // Guards the interaction with the exporter: a divide-by-zero in an
        // expression produces inf, which becomes null in JSON.
        let z = Complex::new(1.0, 0.0) / Complex::ZERO;
        assert!(!z.is_finite());
        assert_eq!(number(z.re), Json::Null);
    }
}
