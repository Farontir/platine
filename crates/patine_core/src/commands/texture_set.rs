//! Texture set commands.

use std::any::Any;

use super::Applied;
use crate::channel::{Channel, ChannelDef};
use crate::command::{Command, CommandError, Label};
use crate::project::{Project, Resolution, TextureSet, TextureSetId};

fn texture_set(project: &mut Project, id: TextureSetId) -> Result<&mut TextureSet, CommandError> {
    project
        .texture_set_mut(id)
        .ok_or(CommandError::MissingTextureSet(id.0))
}

/// Renames a texture set.
#[derive(Debug)]
pub struct RenameTextureSet {
    id: TextureSetId,
    /// New name before `apply`, old name after.
    name: String,
    label_name: String,
    applied: Applied,
}

impl RenameTextureSet {
    /// Renames the texture set `id` to `name`.
    pub fn new(id: TextureSetId, name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            id,
            label_name: name.clone(),
            name,
            applied: Applied::default(),
        }
    }

    fn swap(&mut self, project: &mut Project, applied: bool) -> Result<(), CommandError> {
        let set = texture_set(project, self.id)?;
        self.applied.set(applied)?;
        std::mem::swap(&mut set.name, &mut self.name);
        Ok(())
    }
}

impl Command for RenameTextureSet {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, true)
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, false)
    }

    fn label(&self) -> Label {
        Label::new("command-rename-texture-set").arg("name", &self.label_name)
    }

    fn heap_size(&self) -> usize {
        self.name.len() + self.label_name.len()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Changes the working resolution of a texture set.
#[derive(Debug)]
pub struct SetTextureSetResolution {
    id: TextureSetId,
    /// New resolution before `apply`, old one after.
    resolution: Resolution,
    label_resolution: Resolution,
    applied: Applied,
}

impl SetTextureSetResolution {
    /// Sets the resolution of the texture set `id`.
    pub fn new(id: TextureSetId, resolution: Resolution) -> Self {
        Self {
            id,
            resolution,
            label_resolution: resolution,
            applied: Applied::default(),
        }
    }

    fn swap(&mut self, project: &mut Project, applied: bool) -> Result<(), CommandError> {
        let set = texture_set(project, self.id)?;
        self.applied.set(applied)?;
        std::mem::swap(&mut set.resolution, &mut self.resolution);
        Ok(())
    }
}

impl Command for SetTextureSetResolution {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, true)
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, false)
    }

    fn label(&self) -> Label {
        Label::new("command-texture-set-resolution")
            .arg("resolution", self.label_resolution.pixels())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Changes the height force of a texture set. Successive changes of the same texture set
/// merge (a slider drag).
#[derive(Debug)]
pub struct SetHeightForce {
    id: TextureSetId,
    /// New force before `apply`, old one after.
    force: f32,
    applied: Applied,
}

impl SetHeightForce {
    /// Sets the height force of the texture set `id` (negative values become 0).
    pub fn new(id: TextureSetId, force: f32) -> Self {
        Self {
            id,
            force: if force.is_finite() {
                force.max(0.0)
            } else {
                0.0
            },
            applied: Applied::default(),
        }
    }

    fn swap(&mut self, project: &mut Project, applied: bool) -> Result<(), CommandError> {
        let set = texture_set(project, self.id)?;
        self.applied.set(applied)?;
        std::mem::swap(&mut set.height_force, &mut self.force);
        Ok(())
    }
}

impl Command for SetHeightForce {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, true)
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, false)
    }

    fn label(&self) -> Label {
        Label::new("command-height-force")
    }

    fn merge(&mut self, next: &dyn Command) -> bool {
        // `self` holds the force from before the gesture.
        next.as_any()
            .downcast_ref::<SetHeightForce>()
            .is_some_and(|next| next.id == self.id)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// What a [`SetChannelDef`] did, known once applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChannelChange {
    Added,
    Removed,
    Changed,
    Nothing,
}

/// Adds a channel to a texture set, removes it, or changes its definition (format and
/// default value). Successive changes of the same channel's definition merge.
///
/// Layers keep their settings for a removed channel, which serve again if it comes back.
#[derive(Debug)]
pub struct SetChannelDef {
    set: TextureSetId,
    channel: Channel,
    /// New definition before `apply` (`None` removes), old one after.
    def: Option<ChannelDef>,
    change: Option<ChannelChange>,
    applied: Applied,
}

impl SetChannelDef {
    /// Adds `def` to the texture set `set`, or replaces the definition of its channel.
    pub fn set(set: TextureSetId, def: ChannelDef) -> Self {
        Self {
            set,
            channel: def.channel,
            def: Some(def),
            change: None,
            applied: Applied::default(),
        }
    }

    /// Removes `channel` from the texture set `set`.
    pub fn remove(set: TextureSetId, channel: Channel) -> Self {
        Self {
            set,
            channel,
            def: None,
            change: None,
            applied: Applied::default(),
        }
    }

    fn swap(&mut self, project: &mut Project, applied: bool) -> Result<(), CommandError> {
        let set = texture_set(project, self.set)?;
        self.applied.set(applied)?;
        let channels = &mut set.channels;
        let existing = channels.iter().position(|def| def.channel == self.channel);
        let change = match (existing, self.def.take()) {
            (Some(index), Some(def)) => {
                self.def = Some(std::mem::replace(&mut channels[index], def));
                ChannelChange::Changed
            }
            (Some(index), None) => {
                self.def = Some(channels.remove(index));
                ChannelChange::Removed
            }
            (None, Some(def)) => {
                let index = channels
                    .iter()
                    .position(|other| other.channel > def.channel)
                    .unwrap_or(channels.len());
                channels.insert(index, def);
                ChannelChange::Added
            }
            (None, None) => ChannelChange::Nothing,
        };
        // The label describes the first application.
        self.change.get_or_insert(change);
        Ok(())
    }
}

impl Command for SetChannelDef {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, true)
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, false)
    }

    fn label(&self) -> Label {
        let key = match self.change {
            Some(ChannelChange::Added) => "command-add-channel",
            Some(ChannelChange::Removed) => "command-remove-channel",
            _ => "command-channel-default",
        };
        Label::new(key).arg("channel", self.channel.id())
    }

    fn merge(&mut self, next: &dyn Command) -> bool {
        // `self` holds the definition from before the gesture.
        next.as_any()
            .downcast_ref::<SetChannelDef>()
            .is_some_and(|next| {
                next.set == self.set
                    && next.channel == self.channel
                    && self.change == Some(ChannelChange::Changed)
                    && next.change == Some(ChannelChange::Changed)
            })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
