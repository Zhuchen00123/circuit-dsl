//! Typed identifiers for IR entities.
//!
//! Each kind of entity gets its own `u32`-backed newtype so that a
//! `NodeId` cannot be passed where a `DeviceId` is expected. Indices are
//! stable within one elaborated circuit and are assigned in a deterministic
//! order, so repeated runs over the same source produce the same mapping.

/// Declare a `u32`-backed identifier newtype.
macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u32);

        impl $name {
            pub const fn index(self) -> usize {
                self.0 as usize
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}{}", $prefix, self.0)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}{}", $prefix, self.0)
            }
        }
    };
}

define_id!(
    /// A node (net) in the elaborated circuit. `NodeId(0)` is always ground.
    NodeId,
    "n"
);
define_id!(
    /// A device instance in the elaborated circuit.
    DeviceId,
    "d"
);
define_id!(
    /// A model card.
    ModelId,
    "m"
);
define_id!(
    /// An analysis task inside an experiment.
    AnalysisId,
    "a"
);
define_id!(
    /// A circuit definition.
    CircuitId,
    "c"
);

/// Ground is always node 0, in every circuit, at every hierarchy level.
pub const GROUND: NodeId = NodeId(0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_type_distinct_and_ordered() {
        assert_eq!(NodeId(3).index(), 3);
        assert_eq!(format!("{}", DeviceId(7)), "d7");
        assert_eq!(format!("{:?}", ModelId(1)), "m1");
        assert!(NodeId(1) < NodeId(2));
        assert_eq!(GROUND, NodeId(0));
    }
}
