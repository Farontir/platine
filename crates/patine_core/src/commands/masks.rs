//! Mask commands: adding or removing a layer's mask, and editing its stack of effects.
//! Effects are designated by their identifier, unique in the project.

use std::any::Any;

use super::Applied;
use super::layers::find_mut;
use crate::command::{Command, CommandError, Label};
use crate::ids::{EffectId, LayerId};
use crate::mask::{MaskEffectEntry, MaskInitial, MaskStack};
use crate::project::Project;

/// The mask of the layer `id`, which must have one.
fn mask_mut(project: &mut Project, id: LayerId) -> Result<&mut MaskStack, CommandError> {
    find_mut(project, id)?
        .mask
        .as_mut()
        .ok_or(CommandError::NoMask(id))
}

/// Index of the effect `effect` in the mask of the layer `id`.
fn effect_index(mask: &MaskStack, effect: EffectId) -> Result<usize, CommandError> {
    mask.index_of(effect)
        .ok_or(CommandError::MissingEffect(effect))
}

/// Gives a layer a mask, replaces it, or removes it (`None`).
///
/// A new mask may only keep effects of the current one (to change the initial value or
/// drop effects); new effects go through [`InsertMaskEffect`], which allocates their
/// identifiers.
#[derive(Debug)]
pub struct SetMask {
    id: LayerId,
    /// New mask before `apply`, old one after.
    mask: Option<MaskStack>,
    label: &'static str,
    name: String,
    applied: Applied,
}

impl SetMask {
    /// Sets the mask of the layer `id`. `name` is shown in the history.
    pub fn new(id: LayerId, mask: Option<MaskStack>, name: impl Into<String>) -> Self {
        let label = match &mask {
            None => "command-remove-mask",
            Some(mask) if mask.initial == MaskInitial::Black => "command-black-mask",
            Some(_) => "command-white-mask",
        };
        Self {
            id,
            mask,
            label,
            name: name.into(),
            applied: Applied::default(),
        }
    }

    fn swap(&mut self, project: &mut Project, applied: bool) -> Result<(), CommandError> {
        let layer = find_mut(project, self.id)?;
        // Checked only on a real apply, so that a second apply reports the state mismatch.
        if applied
            && !self.applied.0
            && let Some(new) = &self.mask
        {
            let current = layer.mask.as_ref();
            if let Some(entry) = new
                .effects
                .iter()
                .find(|entry| current.and_then(|mask| mask.index_of(entry.id)).is_none())
            {
                return Err(CommandError::MissingEffect(entry.id));
            }
        }
        self.applied.set(applied)?;
        std::mem::swap(&mut layer.mask, &mut self.mask);
        Ok(())
    }
}

impl Command for SetMask {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, true)
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, false)
    }

    fn label(&self) -> Label {
        Label::new(self.label).arg("name", &self.name)
    }

    fn heap_size(&self) -> usize {
        self.name.len() + self.mask.as_ref().map_or(0, |mask| mask.effects.len() * 64)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Adds an effect to a layer's mask, at `index` from the bottom.
///
/// Its identifier must be fresh (at least the project's `next_id`); `next_id` moves past
/// it.
#[derive(Debug)]
pub struct InsertMaskEffect {
    id: LayerId,
    index: usize,
    /// The effect while not applied; `None` while applied.
    entry: Option<MaskEffectEntry>,
    effect: EffectId,
    kind: &'static str,
    name: String,
    /// `next_id` before `apply`.
    previous_next_id: u64,
}

impl InsertMaskEffect {
    /// Inserts `entry` at `index` in the mask of the layer `id`. `name` is shown in the
    /// history.
    pub fn new(id: LayerId, index: usize, entry: MaskEffectEntry, name: impl Into<String>) -> Self {
        Self {
            id,
            index,
            effect: entry.id,
            kind: entry.effect.kind_id(),
            entry: Some(entry),
            name: name.into(),
            previous_next_id: 0,
        }
    }
}

impl Command for InsertMaskEffect {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        if self.entry.is_none() {
            return Err(CommandError::StateMismatch("already applied"));
        }
        if self.effect.0 < project.next_id {
            return Err(CommandError::IdNotFresh(self.effect.0));
        }
        let next_id = self.effect.0 + 1;
        let previous_next_id = project.next_id;
        let mask = mask_mut(project, self.id)?;
        if self.index > mask.effects.len() {
            return Err(CommandError::InvalidIndex {
                index: self.index,
                len: mask.effects.len(),
            });
        }
        let entry = self
            .entry
            .take()
            .ok_or(CommandError::StateMismatch("already applied"))?;
        mask.effects.insert(self.index, entry);
        self.previous_next_id = previous_next_id;
        project.next_id = next_id;
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError> {
        if self.entry.is_some() {
            return Err(CommandError::StateMismatch("not applied"));
        }
        let mask = mask_mut(project, self.id)?;
        if mask.effects.get(self.index).map(|entry| entry.id) != Some(self.effect) {
            return Err(CommandError::StateMismatch(
                "effect not at its recorded position",
            ));
        }
        self.entry = Some(mask.effects.remove(self.index));
        project.next_id = self.previous_next_id;
        Ok(())
    }

    fn label(&self) -> Label {
        Label::new("command-add-mask-effect")
            .arg("name", &self.name)
            .arg("effect", self.kind)
    }

    fn heap_size(&self) -> usize {
        self.name.len() + 64
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Removes an effect from a layer's mask.
#[derive(Debug)]
pub struct RemoveMaskEffect {
    id: LayerId,
    effect: EffectId,
    /// Its index and the effect itself, while applied.
    removed: Option<(usize, MaskEffectEntry)>,
    name: String,
}

impl RemoveMaskEffect {
    /// Removes the effect `effect` of the mask of the layer `id`. `name` is shown in the
    /// history.
    pub fn new(id: LayerId, effect: EffectId, name: impl Into<String>) -> Self {
        Self {
            id,
            effect,
            removed: None,
            name: name.into(),
        }
    }
}

impl Command for RemoveMaskEffect {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        if self.removed.is_some() {
            return Err(CommandError::StateMismatch("already applied"));
        }
        let mask = mask_mut(project, self.id)?;
        let index = effect_index(mask, self.effect)?;
        self.removed = Some((index, mask.effects.remove(index)));
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let (index, _) = self
            .removed
            .as_ref()
            .ok_or(CommandError::StateMismatch("not applied"))?;
        let index = *index;
        let mask = mask_mut(project, self.id)?;
        if index > mask.effects.len() {
            return Err(CommandError::InvalidIndex {
                index,
                len: mask.effects.len(),
            });
        }
        let (_, entry) = self
            .removed
            .take()
            .ok_or(CommandError::StateMismatch("not applied"))?;
        mask.effects.insert(index, entry);
        Ok(())
    }

    fn label(&self) -> Label {
        Label::new("command-remove-mask-effect").arg("name", &self.name)
    }

    fn heap_size(&self) -> usize {
        self.name.len() + 64
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Moves an effect within its mask.
#[derive(Debug)]
pub struct MoveMaskEffect {
    id: LayerId,
    effect: EffectId,
    /// Destination index, counted after the effect was taken out.
    to: usize,
    /// Original index, while applied.
    from: Option<usize>,
    name: String,
}

impl MoveMaskEffect {
    /// Moves the effect `effect` of the mask of the layer `id` to `to` (from the bottom,
    /// among the other effects). `name` is shown in the history.
    pub fn new(id: LayerId, effect: EffectId, to: usize, name: impl Into<String>) -> Self {
        Self {
            id,
            effect,
            to,
            from: None,
            name: name.into(),
        }
    }
}

impl Command for MoveMaskEffect {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        if self.from.is_some() {
            return Err(CommandError::StateMismatch("already applied"));
        }
        let mask = mask_mut(project, self.id)?;
        let from = effect_index(mask, self.effect)?;
        if self.to >= mask.effects.len() {
            return Err(CommandError::InvalidIndex {
                index: self.to,
                len: mask.effects.len() - 1,
            });
        }
        let entry = mask.effects.remove(from);
        mask.effects.insert(self.to, entry);
        self.from = Some(from);
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let from = self
            .from
            .ok_or(CommandError::StateMismatch("not applied"))?;
        let mask = mask_mut(project, self.id)?;
        if mask.effects.get(self.to).map(|entry| entry.id) != Some(self.effect) {
            return Err(CommandError::StateMismatch(
                "effect not at its recorded position",
            ));
        }
        let entry = mask.effects.remove(self.to);
        mask.effects.insert(from, entry);
        self.from = None;
        Ok(())
    }

    fn label(&self) -> Label {
        Label::new("command-move-mask-effect").arg("name", &self.name)
    }

    fn heap_size(&self) -> usize {
        self.name.len()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Replaces an effect of a layer's mask (visibility, blend, opacity, parameters); its
/// identifier stays. Continuous changes of the same effect merge.
#[derive(Debug)]
pub struct SetMaskEffect {
    id: LayerId,
    /// New effect before `apply`, old one after.
    entry: MaskEffectEntry,
    kind: &'static str,
    name: String,
    applied: Applied,
}

impl SetMaskEffect {
    /// Replaces the effect `entry.id` of the mask of the layer `id` by `entry`. `name` is
    /// shown in the history.
    pub fn new(id: LayerId, entry: MaskEffectEntry, name: impl Into<String>) -> Self {
        Self {
            id,
            kind: entry.effect.kind_id(),
            entry,
            name: name.into(),
            applied: Applied::default(),
        }
    }

    fn swap(&mut self, project: &mut Project, applied: bool) -> Result<(), CommandError> {
        let mask = mask_mut(project, self.id)?;
        let index = effect_index(mask, self.entry.id)?;
        self.applied.set(applied)?;
        std::mem::swap(&mut mask.effects[index], &mut self.entry);
        Ok(())
    }
}

impl Command for SetMaskEffect {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, true)
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, false)
    }

    fn label(&self) -> Label {
        Label::new("command-mask-effect")
            .arg("name", &self.name)
            .arg("effect", self.kind)
    }

    fn merge(&mut self, next: &dyn Command) -> bool {
        next.as_any()
            .downcast_ref::<SetMaskEffect>()
            .is_some_and(|next| next.id == self.id && next.entry.id == self.entry.id)
    }

    fn heap_size(&self) -> usize {
        self.name.len() + 64
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
