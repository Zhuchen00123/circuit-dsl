//! Showing a value in a session.
//!
//! One rule, inherited from `docs/language.md`: what is shown must be
//! something the language can be told. Numbers carry their unit
//! (`100 us`, `1.5 kohm`), symbols keep their colon, strings keep their
//! quotes, and arrays and dictionaries show their structure.

use circuit_core::format_quantity;
use circuit_dsl::Value;

/// Render a value the way a session shows it.
pub fn value(v: &Value) -> String {
    match v {
        Value::Num(q) => format_quantity(*q),
        Value::Bool(b) => b.to_string(),
        Value::Sym(s) => format!(":{s}"),
        Value::Str(s) => format!("\"{s}\""),
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(value).collect();
            format!("[{}]", inner.join(", "))
        }
        Value::Dict(entries) => {
            let inner: Vec<String> = entries
                .iter()
                .map(|(k, v)| format!("{k}: {}", value(v)))
                .collect();
            format!("{{ {} }}", inner.join(", "))
        }
    }
}

/// The type name to show when a value alone would be ambiguous.
pub fn type_name(v: &Value) -> &'static str {
    v.type_name()
}

#[cfg(test)]
mod tests {
    use super::*;
    use circuit_core::units::Quantity;

    #[test]
    fn numbers_carry_their_unit() {
        assert_eq!(value(&Value::Num(Quantity::ohms(1500.0))), "1.5 kohm");
        assert_eq!(value(&Value::Num(Quantity::seconds(1e-4))), "100 us");
        assert_eq!(value(&Value::Num(Quantity::scalar(2.0))), "2");
    }

    #[test]
    fn other_values_keep_their_syntax() {
        assert_eq!(value(&Value::Sym("vin".into())), ":vin");
        assert_eq!(value(&Value::Str("r1".into())), "\"r1\"");
        assert_eq!(value(&Value::Bool(true)), "true");
        assert_eq!(
            value(&Value::Array(vec![
                Value::Num(Quantity::scalar(1.0)),
                Value::Sym("a".into())
            ])),
            "[1, :a]"
        );
        assert_eq!(
            value(&Value::Dict(vec![(
                "input".into(),
                Value::Sym("vin".into())
            )])),
            "{ input: :vin }"
        );
    }

    #[test]
    fn nested_values_render() {
        let nested = Value::Array(vec![Value::Array(vec![Value::Num(Quantity::volts(1.0))])]);
        assert_eq!(value(&nested), "[[1 V]]");
    }
}
