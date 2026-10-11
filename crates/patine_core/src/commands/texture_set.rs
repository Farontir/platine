//! Texture set commands.

use std::any::Any;

use super::Applied;
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
