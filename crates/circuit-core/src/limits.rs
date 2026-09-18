//! Configuration limits that bound elaboration and result size.
//!
//! The language has finite loops and conditional expansion, so a mistake in a
//! loop bound could otherwise allocate unbounded devices. Every limit is
//! explicit and configurable rather than a hidden constant, and exceeding one
//! is a diagnostic (`E_LIMIT`), never a silent truncation (brief §5.4, §9).

/// Limits applied during elaboration and result materialisation.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum devices in the elaborated circuit.
    pub max_devices: usize,
    /// Maximum nodes in the elaborated circuit.
    pub max_nodes: usize,
    /// Maximum number of loop iterations across one `for` statement.
    pub max_loop_iterations: u64,
    /// Maximum total loop iterations executed while elaborating.
    pub max_total_steps: u64,
    /// Maximum subcircuit nesting depth.
    pub max_depth: usize,
    /// Maximum points a sweep may generate.
    pub max_sweep_points: u64,
    /// Maximum scalar values held in one result dataset.
    pub max_result_values: u64,
    /// Maximum length of a `pwl` waveform table.
    pub max_pwl_points: usize,
}

/// Maximum nesting depth of a written expression.
///
/// Both the parser and the result-expression evaluator are recursive, so this
/// is where recursion stops: input deeper than this is refused with
/// `E_LIMIT` naming the observed depth, instead of running the stack out and
/// killing the process with no diagnostic (round-4 FINDING-1).
///
/// It is a `const` and not a `Limits` field on purpose: the depth is enforced
/// in the parser and in `circuit-results`, neither of which carries a
/// `Limits` value, and both sides must agree on one number.
///
/// 256 is far above anything written by hand (real programs nest a handful of
/// levels) and far below what a 64 MiB stack — the stack `cdsl` runs its work
/// on — can hold even in an unoptimised build: measured in a debug build, 300
/// nested calls abort on a 2 MiB thread but are a clean `E_LIMIT` on the
/// work stack, and every depth up to the limit is accepted there.
///
/// The pairing matters: the limit is what makes deep input a *diagnostic*, and
/// the work stack is what makes the accepted depth *processable*. A caller
/// embedding these crates on a small stack should give the work the same kind
/// of stack; `cdsl` does it in `main`.
pub const MAX_EXPR_DEPTH: usize = 256;

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_devices: 100_000,
            max_nodes: 100_000,
            max_loop_iterations: 100_000,
            max_total_steps: 1_000_000,
            max_depth: 16,
            max_sweep_points: 1_000_000,
            max_result_values: 50_000_000,
            max_pwl_points: 100_000,
        }
    }
}

impl Limits {
    /// Tighter limits for tests, so that limit handling is exercised without
    /// building large circuits.
    pub fn for_tests() -> Self {
        Self {
            max_devices: 64,
            max_nodes: 64,
            max_loop_iterations: 32,
            max_total_steps: 128,
            max_depth: 4,
            max_sweep_points: 100,
            max_result_values: 10_000,
            max_pwl_points: 16,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_generous_but_finite() {
        let l = Limits::default();
        assert!(l.max_devices > 0);
        assert!(l.max_depth >= 4);
        assert!(l.max_result_values >= l.max_sweep_points);
    }

    #[test]
    fn test_limits_are_stricter() {
        let t = Limits::for_tests();
        let d = Limits::default();
        assert!(t.max_devices < d.max_devices);
        assert!(t.max_depth < d.max_depth);
    }
}
