//! Paint commands: adding a stroke to a paint layer or a paint mask effect.
//!
//! The document holds strokes only. The pixels of a stroke, and the tiles saved to undo
//! it, belong to the GPU caches, which recognize the stroke by its pointer.

use std::any::Any;
use std::sync::Arc;

use super::layers::find_mut;
use crate::command::{Command, CommandError, Label};
use crate::ids::{EffectId, LayerId};
use crate::layer::LayerKind;
use crate::mask::MaskEffect;
use crate::project::Project;
use crate::resource::{Resource, ResourceHash};
use crate::stroke::{Stroke, StrokeLog, Tool};

/// What a stroke paints into.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaintTarget {
    /// The content of a paint layer.
    Layer(LayerId),
    /// A paint effect of a layer's mask.
    Mask(LayerId, EffectId),
}

impl PaintTarget {
    /// The layer painted, or whose mask is painted.
    pub fn layer(self) -> LayerId {
        match self {
            Self::Layer(id) | Self::Mask(id, _) => id,
        }
    }
}

/// The stroke log of `target`, which must be a paint layer or a paint mask effect.
fn log_mut(
    project: &mut Project,
    target: PaintTarget,
) -> Result<&mut Arc<StrokeLog>, CommandError> {
    let layer = find_mut(project, target.layer())?;
    match target {
        PaintTarget::Layer(id) => match &mut layer.kind {
            LayerKind::Paint { strokes } => Ok(strokes),
            _ => Err(CommandError::NotAPaintLayer(id)),
        },
        PaintTarget::Mask(id, effect) => {
            let mask = layer.mask.as_mut().ok_or(CommandError::NoMask(id))?;
            let entry = mask
                .effects
                .iter_mut()
                .find(|entry| entry.id == effect)
                .ok_or(CommandError::MissingEffect(effect))?;
            match &mut entry.effect {
                MaskEffect::Paint { strokes } => Ok(strokes),
                _ => Err(CommandError::NotAPaintEffect(effect)),
            }
        }
    }
}

/// Adds a stroke at the end of the log of a paint layer or a paint mask effect, with
/// the image of its brush alpha when the project does not hold it yet.
#[derive(Debug)]
pub struct PaintStroke {
    target: PaintTarget,
    stroke: Arc<Stroke>,
    /// The alpha image the stroke uses, stored in the project if missing.
    alpha: Option<(ResourceHash, Resource)>,
    /// Whether `apply` added the alpha (so `revert` removes it).
    added: bool,
    applied: bool,
    name: String,
}

impl PaintStroke {
    /// Paints `stroke` into `target`. `alpha` is the image of the brush alpha, if the
    /// stroke uses one; `name` (of the layer) is shown in the history.
    pub fn new(
        target: PaintTarget,
        stroke: Arc<Stroke>,
        alpha: Option<(ResourceHash, Resource)>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            target,
            stroke,
            alpha,
            added: false,
            applied: false,
            name: name.into(),
        }
    }

    /// Where the stroke goes.
    pub fn target(&self) -> PaintTarget {
        self.target
    }

    /// The stroke added.
    pub fn stroke(&self) -> &Arc<Stroke> {
        &self.stroke
    }
}

impl Command for PaintStroke {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        if self.applied {
            return Err(CommandError::StateMismatch("already applied"));
        }
        if let Some((hash, resource)) = &self.alpha
            && ResourceHash::of(&resource.bytes) != *hash
        {
            return Err(CommandError::ResourceHash);
        }
        // Found first: an error leaves the document unchanged.
        let log = log_mut(project, self.target)?;
        Arc::make_mut(log).strokes.push(self.stroke.clone());
        if let Some((hash, resource)) = &self.alpha {
            self.added = !project.resources.contains_key(hash);
            if self.added {
                project.resources.insert(*hash, resource.clone());
            }
        }
        self.applied = true;
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError> {
        if !self.applied {
            return Err(CommandError::StateMismatch("not applied"));
        }
        let log = log_mut(project, self.target)?;
        if !log
            .strokes
            .last()
            .is_some_and(|last| Arc::ptr_eq(last, &self.stroke))
        {
            return Err(CommandError::StateMismatch("stroke not last in its log"));
        }
        Arc::make_mut(log).strokes.pop();
        if let Some((hash, _)) = &self.alpha
            && self.added
        {
            project.resources.remove(hash);
        }
        self.applied = false;
        Ok(())
    }

    fn label(&self) -> Label {
        let key = match (self.stroke.tool, self.target) {
            (Tool::Eraser, _) => "command-erase-stroke",
            (_, PaintTarget::Mask(..)) => "command-paint-mask-stroke",
            _ => "command-paint-stroke",
        };
        Label::new(key).arg("name", &self.name)
    }

    fn heap_size(&self) -> usize {
        // The alpha's bytes are shared with the project while applied.
        self.name.len() + self.stroke.heap_size()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
