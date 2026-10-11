//! Stable identifiers of document nodes.
//!
//! Layers, effects and anchors take their identifier from one project-wide counter,
//! [`crate::project::Project::next_id`]. An identifier is never given to two nodes of the
//! same document: undo, instances and anchor references rely on it.

use serde::{Deserialize, Serialize};

macro_rules! node_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub u64);

        impl $name {
            /// Raw value.
            pub fn get(self) -> u64 {
                self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

node_id!(
    /// Identifier of a layer.
    LayerId
);
node_id!(
    /// Identifier of a mask effect or a layer effect.
    EffectId
);
node_id!(
    /// Identifier of an anchor point.
    AnchorId
);

/// Hands out consecutive identifiers from a starting value, to build nodes before the
/// command that inserts them (the command checks that they are fresh).
#[derive(Clone, Debug)]
pub struct IdSequence {
    next: u64,
}

impl IdSequence {
    /// A sequence starting at `first`, usually the project's `next_id`.
    pub fn starting_at(first: u64) -> Self {
        Self { next: first }
    }

    /// The next raw identifier.
    pub fn next_raw(&mut self) -> u64 {
        let id = self.next;
        self.next += 1;
        id
    }

    /// The next layer identifier.
    pub fn layer(&mut self) -> LayerId {
        LayerId(self.next_raw())
    }

    /// The next effect identifier.
    pub fn effect(&mut self) -> EffectId {
        EffectId(self.next_raw())
    }

    /// The next anchor identifier.
    pub fn anchor(&mut self) -> AnchorId {
        AnchorId(self.next_raw())
    }
}
