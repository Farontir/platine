//! The document's commands.
//!
//! Most commands change one value with the *swap* pattern: the command holds the new
//! value, `apply` swaps it with the document's (the command then holds the old one) and
//! `revert` swaps them back. Merging two such commands on the same target only keeps the
//! first one, which already holds the value from before the gesture.

mod environment;
mod layers;
mod masks;
mod paint;
mod texture_set;

pub use environment::{EnvironmentField, SetEnvironment};
pub use layers::{
    InsertLayer, MoveLayer, RemoveLayer, RenameLayer, SetChannelBlend, SetFillSource, SetFillValue,
    SetLayerVisible, SetProjection,
};
pub use masks::{InsertMaskEffect, MoveMaskEffect, RemoveMaskEffect, SetMask, SetMaskEffect};
pub use paint::{PaintStroke, PaintTarget};
pub use texture_set::{RenameTextureSet, SetChannelDef, SetHeightForce, SetTextureSetResolution};

use crate::command::CommandError;

/// Tracks whether a command is applied, to reject `apply` twice or `revert` first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Applied(bool);

impl Applied {
    fn set(&mut self, applied: bool) -> Result<(), CommandError> {
        if self.0 == applied {
            Err(CommandError::StateMismatch(if applied {
                "already applied"
            } else {
                "not applied"
            }))
        } else {
            self.0 = applied;
            Ok(())
        }
    }
}
