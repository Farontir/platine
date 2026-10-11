//! The open document: the project, its undo history and its file.
//!
//! The interface never writes into the project: it hands commands to
//! [`Document::execute`]. Creating, opening or recovering a project replaces the whole
//! document.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use fluent_bundle::FluentArgs;
use patine_core::command::{Command, Label};
use patine_core::commands::SetEnvironment;
use patine_core::environment::{EnvironmentSettings, EnvironmentSource};
use patine_core::history::History;
use patine_core::project::Project;
use patine_core::resource::{Resource, ResourceHash, ResourceKind};

use crate::i18n::I18n;

/// A project being edited.
#[derive(Debug)]
pub struct Document {
    /// The document data.
    pub project: Project,
    /// Undo history.
    pub history: History,
    /// File the project was opened from or last saved to.
    pub path: Option<PathBuf>,
}

impl Document {
    /// A document for a project that has never been saved.
    pub fn new(project: Project) -> Self {
        Self {
            project,
            history: History::default(),
            path: None,
        }
    }

    /// A document opened from `path`.
    pub fn opened(project: Project, path: PathBuf) -> Self {
        Self {
            project,
            history: History::default(),
            path: Some(path),
        }
    }

    /// Applies `command` and records it. A refused command is logged and changes nothing.
    pub fn execute(&mut self, command: Box<dyn Command>) {
        let label = command.label();
        if let Err(error) = self.history.push(&mut self.project, command) {
            tracing::warn!(command = label.key, %error, "command refused");
        }
    }

    /// Undoes the last command.
    pub fn undo(&mut self) {
        if let Err(error) = self.history.undo(&mut self.project) {
            tracing::error!(%error, "undo failed; the history was cleared");
        }
    }

    /// Redoes the last undone command.
    pub fn redo(&mut self) {
        if let Err(error) = self.history.redo(&mut self.project) {
            tracing::error!(%error, "redo failed; the history was cleared");
        }
    }

    /// Name shown in the title bar: the file name, or the mesh name before the first save.
    pub fn name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|path| path.file_stem())
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.project.mesh.name.clone())
    }
}

/// Translates a command label.
pub fn translate(i18n: &I18n, label: &Label) -> String {
    let mut args = FluentArgs::new();
    for (name, value) in &label.args {
        args.set(*name, value.clone());
    }
    i18n.tr_args(label.key, &args)
}

/// A change of the environment asked by the interface.
#[derive(Clone, Debug)]
pub enum EnvironmentChange {
    /// New rotation, radians.
    Rotation(f32),
    /// New exposure, EV.
    Exposure(f32),
    /// The studio sky or a bundled HDRI.
    Source(EnvironmentSource),
    /// An HDRI file picked by the user.
    Imported {
        /// File name.
        name: String,
        /// File contents.
        bytes: Arc<[u8]>,
    },
}

/// Environment used while no project is open; a new project starts with it.
#[derive(Clone, Debug, Default)]
pub struct DraftEnvironment {
    /// Settings.
    pub settings: EnvironmentSettings,
    /// The imported HDRI, when the source is one.
    pub resources: BTreeMap<ResourceHash, Resource>,
}

impl DraftEnvironment {
    /// Applies `change` directly (there is no history without a project).
    pub fn apply(&mut self, change: EnvironmentChange) {
        match change {
            EnvironmentChange::Rotation(radians) => {
                self.settings.rotation = radians.rem_euclid(std::f32::consts::TAU);
            }
            EnvironmentChange::Exposure(ev) => self.settings.exposure_ev = ev,
            EnvironmentChange::Source(source) => {
                self.settings.source = source;
                self.resources.clear();
            }
            EnvironmentChange::Imported { name, bytes } => {
                let (hash, resource) = Resource::new(name, ResourceKind::Environment, bytes);
                self.settings.source = EnvironmentSource::Resource(hash);
                self.resources = BTreeMap::from([(hash, resource)]);
            }
        }
    }

    /// Gives `project` this environment, with the HDRI it needs.
    pub fn install(&self, project: &mut Project) {
        project.environment = self.settings.clone();
        project.resources.extend(
            self.resources
                .iter()
                .map(|(hash, resource)| (*hash, resource.clone())),
        );
    }
}

/// The command for `change` on `current`.
pub fn environment_command(
    current: &EnvironmentSettings,
    change: EnvironmentChange,
) -> Box<dyn Command> {
    Box::new(match change {
        EnvironmentChange::Rotation(radians) => SetEnvironment::rotation(current, radians),
        EnvironmentChange::Exposure(ev) => SetEnvironment::exposure(current, ev),
        EnvironmentChange::Source(source) => SetEnvironment::source(current, source),
        EnvironmentChange::Imported { name, bytes } => {
            let (hash, resource) = Resource::new(name, ResourceKind::Environment, bytes);
            SetEnvironment::imported(current, hash, resource)
        }
    })
}

/// Every translation key of command labels, for tests.
pub const COMMAND_LABEL_KEYS: [&str; 14] = [
    "command-rename-texture-set",
    "command-texture-set-resolution",
    "command-environment-source",
    "command-environment-rotation",
    "command-environment-exposure",
    "command-add-layer",
    "command-remove-layer",
    "command-move-layer",
    "command-rename-layer",
    "command-show-layer",
    "command-hide-layer",
    "command-channel-blend",
    "history-initial",
    "history-truncated",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;
    use patine_core::commands::RenameTextureSet;
    use patine_core::project::{ProjectSettings, TextureSetId};

    fn project() -> Project {
        let mesh = patine_core::mesh::MeshAsset {
            materials: vec!["body".into()],
            ..Default::default()
        };
        Project::new(mesh, "mesh.glb".into(), ProjectSettings::default())
    }

    #[test]
    fn every_command_label_is_translated() {
        for language in Language::ALL {
            let i18n = I18n::new(language);
            for key in COMMAND_LABEL_KEYS {
                assert!(i18n.has(key), "{language:?}: {key}");
            }
        }
        let label = RenameTextureSet::new(TextureSetId(0), "Coque").label();
        let text = translate(&I18n::new(Language::French), &label);
        assert!(text.contains("Coque"), "{text}");
    }

    #[test]
    fn draft_environment_moves_into_a_new_project() {
        let mut draft = DraftEnvironment::default();
        draft.apply(EnvironmentChange::Rotation(-1.0));
        draft.apply(EnvironmentChange::Imported {
            name: "sky.hdr".into(),
            bytes: Arc::from(&b"radiance"[..]),
        });
        let mut project = project();
        draft.install(&mut project);
        assert!((project.environment.rotation - (std::f32::consts::TAU - 1.0)).abs() < 1e-6);
        assert_eq!(project.resources.len(), 1);
        assert_eq!(project.validate(), Ok(()));
        draft.apply(EnvironmentChange::Source(EnvironmentSource::Studio));
        assert!(draft.resources.is_empty());
    }

    #[test]
    fn commands_go_through_the_history() {
        let mut document = Document::new(project());
        let command = environment_command(
            &document.project.environment,
            EnvironmentChange::Exposure(2.0),
        );
        document.execute(command);
        assert_eq!(document.project.environment.exposure_ev, 2.0);
        assert!(document.history.is_dirty());
        document.undo();
        assert_eq!(document.project.environment.exposure_ev, 0.0);
        document.redo();
        assert_eq!(document.project.environment.exposure_ev, 2.0);
        assert_eq!(document.name(), "");
    }
}
