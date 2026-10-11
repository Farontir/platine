//! Commands: the only way to change a document.
//!
//! The interface builds a command from what it reads in the document, then hands it to
//! [`crate::history::History::push`], which applies it and keeps it for undo.

use std::any::Any;
use std::fmt::Debug;

use thiserror::Error;

use crate::ids::{EffectId, LayerId};
use crate::project::Project;

/// Text describing a command in the history: a translation key and its arguments.
/// `patine_core` does not depend on the translation system; the application translates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    /// Translation key, e.g. `command-rename-layer`.
    pub key: &'static str,
    /// Named arguments of the translation.
    pub args: Vec<(&'static str, String)>,
}

impl Label {
    /// A label without arguments.
    pub fn new(key: &'static str) -> Self {
        Self {
            key,
            args: Vec::new(),
        }
    }

    /// Adds an argument.
    pub fn arg(mut self, name: &'static str, value: impl ToString) -> Self {
        self.args.push((name, value.to_string()));
        self
    }
}

/// Why a command cannot be applied or reverted. The document is left unchanged.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CommandError {
    /// The texture set does not exist.
    #[error("texture set {0} does not exist")]
    MissingTextureSet(u32),
    /// The layer does not exist.
    #[error("layer {0} does not exist")]
    MissingLayer(LayerId),
    /// The target parent is not a folder of the texture set.
    #[error("layer {0} is not a folder of this texture set")]
    NotAFolder(LayerId),
    /// The index is past the end of the target list.
    #[error("position {index} is past the {len} layers of the target")]
    InvalidIndex {
        /// Requested index.
        index: usize,
        /// Number of layers there.
        len: usize,
    },
    /// A new node reuses an identifier.
    #[error("identifier {0} is already allocated")]
    IdNotFresh(u64),
    /// A folder cannot move into itself or one of its descendants.
    #[error("layer {0} cannot move into itself")]
    MoveIntoItself(LayerId),
    /// The layer is not a uniform fill.
    #[error("layer {0} is not a uniform fill")]
    NotAUniformFill(LayerId),
    /// The layer has no mask.
    #[error("layer {0} has no mask")]
    NoMask(LayerId),
    /// The mask effect does not exist in that mask.
    #[error("mask effect {0} does not exist")]
    MissingEffect(EffectId),
    /// Another layer depends on this one (instance or anchor reference).
    #[error("layer {0} is used by another layer")]
    LayerInUse(LayerId),
    /// A resource added by the command has a hash that does not match its bytes.
    #[error("resource bytes do not match their hash")]
    ResourceHash,
    /// `revert` was called on a command that was not applied, or the document no
    /// longer matches what the command changed.
    #[error("the document does not match the command ({0})")]
    StateMismatch(&'static str),
}

/// A reversible change of the document.
///
/// Contract:
/// - `apply` and `revert` leave the document unchanged when they return an error;
/// - `revert` after a successful `apply` restores the document exactly, and `apply`
///   after `revert` produces the same document again (redo);
/// - the command records what it needs for `revert` during `apply`.
pub trait Command: Debug + Send + 'static {
    /// Applies the change.
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError>;

    /// Undoes the change made by the last `apply`.
    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError>;

    /// Text shown in the history.
    fn label(&self) -> Label;

    /// Absorbs `next`, already applied right after `self`, so that one continuous
    /// gesture (a slider drag) makes one history entry. Returns `false` when the commands
    /// do not change the same thing; `self` is then left as is.
    fn merge(&mut self, next: &dyn Command) -> bool {
        let _ = next;
        false
    }

    /// Approximate heap memory held by the command, for the history budget.
    fn heap_size(&self) -> usize {
        0
    }

    /// The command as `Any`, so that `merge` can downcast `next`.
    fn as_any(&self) -> &dyn Any;
}

/// Several commands applied as one history entry, in order, and reverted in reverse.
/// If one fails to apply, those already applied are reverted and the error returned.
#[derive(Debug)]
pub struct CommandGroup {
    label: Label,
    commands: Vec<Box<dyn Command>>,
}

impl CommandGroup {
    /// A group shown in the history as `label`.
    pub fn new(label: Label, commands: Vec<Box<dyn Command>>) -> Self {
        Self { label, commands }
    }
}

impl Command for CommandGroup {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        for index in 0..self.commands.len() {
            if let Err(error) = self.commands[index].apply(project) {
                for applied in self.commands[..index].iter_mut().rev() {
                    // Reverting what was just applied cannot meet a changed document.
                    let _ = applied.revert(project);
                }
                return Err(error);
            }
        }
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError> {
        for command in self.commands.iter_mut().rev() {
            command.revert(project)?;
        }
        Ok(())
    }

    fn label(&self) -> Label {
        self.label.clone()
    }

    fn heap_size(&self) -> usize {
        self.commands
            .iter()
            .map(|command| command.heap_size())
            .sum()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
