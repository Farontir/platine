//! Environment command.

use std::any::Any;

use super::Applied;
use crate::command::{Command, CommandError, Label};
use crate::environment::{EnvironmentSettings, EnvironmentSource};
use crate::project::Project;
use crate::resource::{Resource, ResourceHash};

/// Which setting a [`SetEnvironment`] changes; only changes of the same field merge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EnvironmentField {
    /// The image.
    Source,
    /// The rotation.
    Rotation,
    /// The exposure.
    Exposure,
}

/// Changes the environment settings.
#[derive(Debug)]
pub struct SetEnvironment {
    field: EnvironmentField,
    /// New settings before `apply`, old ones after.
    settings: EnvironmentSettings,
    /// A user HDRI to store in the project with the new source.
    resource: Option<(ResourceHash, Resource)>,
    /// Whether `apply` added the resource (it was not stored yet).
    added: bool,
    applied: Applied,
}

impl SetEnvironment {
    /// Sets the rotation, in radians, wrapped to `[0, 2π)`.
    pub fn rotation(current: &EnvironmentSettings, radians: f32) -> Self {
        let settings = EnvironmentSettings {
            rotation: radians.rem_euclid(std::f32::consts::TAU),
            ..current.clone()
        };
        Self::new(EnvironmentField::Rotation, settings, None)
    }

    /// Sets the exposure, in EV.
    pub fn exposure(current: &EnvironmentSettings, ev: f32) -> Self {
        let settings = EnvironmentSettings {
            exposure_ev: ev,
            ..current.clone()
        };
        Self::new(EnvironmentField::Exposure, settings, None)
    }

    /// Uses the studio sky or a bundled HDRI.
    pub fn source(current: &EnvironmentSettings, source: EnvironmentSource) -> Self {
        let settings = EnvironmentSettings {
            source,
            ..current.clone()
        };
        Self::new(EnvironmentField::Source, settings, None)
    }

    /// Uses an HDRI imported by the user, stored in the project with the command.
    pub fn imported(current: &EnvironmentSettings, hash: ResourceHash, resource: Resource) -> Self {
        let settings = EnvironmentSettings {
            source: EnvironmentSource::Resource(hash),
            ..current.clone()
        };
        Self::new(EnvironmentField::Source, settings, Some((hash, resource)))
    }

    fn new(
        field: EnvironmentField,
        settings: EnvironmentSettings,
        resource: Option<(ResourceHash, Resource)>,
    ) -> Self {
        Self {
            field,
            settings,
            resource,
            added: false,
            applied: Applied::default(),
        }
    }
}

impl Command for SetEnvironment {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        if let Some((hash, resource)) = &self.resource
            && ResourceHash::of(&resource.bytes) != *hash
        {
            return Err(CommandError::ResourceHash);
        }
        self.applied.set(true)?;
        if let Some((hash, resource)) = &self.resource {
            self.added = !project.resources.contains_key(hash);
            if self.added {
                project.resources.insert(*hash, resource.clone());
            }
        }
        std::mem::swap(&mut project.environment, &mut self.settings);
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.applied.set(false)?;
        std::mem::swap(&mut project.environment, &mut self.settings);
        if let Some((hash, _)) = &self.resource
            && self.added
        {
            project.resources.remove(hash);
        }
        Ok(())
    }

    fn label(&self) -> Label {
        Label::new(match self.field {
            EnvironmentField::Source => "command-environment-source",
            EnvironmentField::Rotation => "command-environment-rotation",
            EnvironmentField::Exposure => "command-environment-exposure",
        })
    }

    fn merge(&mut self, next: &dyn Command) -> bool {
        // `self` holds the settings from before the gesture, which is all revert needs.
        next.as_any()
            .downcast_ref::<SetEnvironment>()
            .is_some_and(|next| next.field == self.field && self.field != EnvironmentField::Source)
    }

    fn heap_size(&self) -> usize {
        // Shared with the project while applied, but the command may become its only
        // owner once undone.
        self.resource
            .as_ref()
            .map_or(0, |(_, resource)| resource.bytes.len())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
