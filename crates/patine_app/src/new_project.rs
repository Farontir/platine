//! New Project dialog: mesh file, resolution and normal convention; the import runs on a
//! worker thread so large meshes never freeze the interface.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

use fluent_bundle::FluentArgs;
use patine_core::project::{NormalConvention, Project, ProjectSettings, Resolution};
use patine_mesh::{BuildOptions, ImportReport, Imported, MeshError, MeshFormat};

use crate::i18n::I18n;

/// A project ready to replace the document, with the report of its import.
pub struct CreatedProject {
    /// The new project.
    pub project: Project,
    /// Import report, for the console.
    pub report: ImportReport,
}

enum Task {
    Idle,
    Browsing(Receiver<Option<PathBuf>>),
    Importing(Receiver<Result<Imported, MeshError>>),
}

/// State of the open dialog.
pub struct NewProjectDialog {
    path: String,
    settings: ProjectSettings,
    task: Task,
    error: Option<String>,
}

impl Default for NewProjectDialog {
    fn default() -> Self {
        Self {
            path: String::new(),
            settings: ProjectSettings::default(),
            task: Task::Idle,
            error: None,
        }
    }
}

/// What the dialog asks of the application after a frame.
pub enum DialogOutcome {
    /// Keep the dialog open.
    Open,
    /// Close it without creating anything.
    Cancelled,
    /// Close it and replace the document.
    Created(Box<CreatedProject>),
}

/// User-facing, translated message of an import error.
pub fn error_message(i18n: &I18n, error: &MeshError) -> String {
    let mut args = FluentArgs::new();
    match error {
        MeshError::Io { path, source } => {
            args.set("path", path.display().to_string());
            args.set("message", source.to_string());
        }
        MeshError::UnsupportedFormat { extension } => args.set("extension", extension.clone()),
        MeshError::Parse { format, message } => {
            args.set("format", *format);
            args.set("message", message.clone());
        }
        MeshError::MissingUvs { objects } => args.set("objects", objects.join(", ")),
        MeshError::TooManyVertices(count) => args.set("count", *count),
        MeshError::Tangents(message) => args.set("message", message.clone()),
        MeshError::Invalid(error) => args.set("message", error.to_string()),
        MeshError::NoGeometry => {}
    }
    i18n.tr_args(error.translation_key(), &args)
}

fn spawn<T: Send + 'static>(
    ctx: &egui::Context,
    work: impl FnOnce() -> T + Send + 'static,
) -> Receiver<T> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        // The dialog may have been closed meanwhile: then nobody listens.
        let _ = sender.send(work());
        ctx.request_repaint();
    });
    receiver
}

impl NewProjectDialog {
    fn busy(&self) -> bool {
        !matches!(self.task, Task::Idle)
    }

    /// Polls the worker threads.
    fn poll(&mut self, i18n: &I18n) -> Option<CreatedProject> {
        match &self.task {
            Task::Idle => None,
            Task::Browsing(receiver) => match receiver.try_recv() {
                Ok(picked) => {
                    if let Some(path) = picked {
                        self.path = path.display().to_string();
                        self.error = None;
                    }
                    self.task = Task::Idle;
                    None
                }
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    self.task = Task::Idle;
                    None
                }
            },
            Task::Importing(receiver) => match receiver.try_recv() {
                Ok(Ok(imported)) => {
                    self.task = Task::Idle;
                    let project =
                        Project::new(imported.mesh, PathBuf::from(&self.path), self.settings);
                    Some(CreatedProject {
                        project,
                        report: imported.report,
                    })
                }
                Ok(Err(error)) => {
                    tracing::warn!(%error, "mesh import failed");
                    self.error = Some(error_message(i18n, &error));
                    self.task = Task::Idle;
                    None
                }
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    self.task = Task::Idle;
                    None
                }
            },
        }
    }

    /// Draws the dialog.
    pub fn show(&mut self, ctx: &egui::Context, i18n: &I18n) -> DialogOutcome {
        if let Some(created) = self.poll(i18n) {
            return DialogOutcome::Created(Box::new(created));
        }
        let mut outcome = DialogOutcome::Open;
        let response = egui::Modal::new(egui::Id::new("new_project")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading(i18n.tr("new-project-title"));
            ui.add_space(8.0);
            let busy = self.busy();
            egui::Grid::new("new_project_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label(i18n.tr("new-project-mesh"));
                    ui.horizontal(|ui| {
                        ui.add_enabled(
                            !busy,
                            egui::TextEdit::singleline(&mut self.path).desired_width(340.0),
                        );
                        if ui
                            .add_enabled(!busy, egui::Button::new(i18n.tr("new-project-browse")))
                            .clicked()
                        {
                            let filter_name = i18n.tr("new-project-filter").into_owned();
                            self.task = Task::Browsing(spawn(ctx, move || {
                                rfd::FileDialog::new()
                                    .add_filter(filter_name, &MeshFormat::EXTENSIONS)
                                    .pick_file()
                            }));
                        }
                    });
                    ui.end_row();

                    ui.label(i18n.tr("new-project-resolution"));
                    egui::ComboBox::from_id_salt("new_project_resolution")
                        .selected_text(format!("{0} × {0}", self.settings.resolution.pixels()))
                        .show_ui(ui, |ui| {
                            for resolution in Resolution::ALL {
                                ui.selectable_value(
                                    &mut self.settings.resolution,
                                    resolution,
                                    format!("{0} × {0}", resolution.pixels()),
                                );
                            }
                        });
                    ui.end_row();

                    ui.label(i18n.tr("new-project-normal-convention"));
                    ui.horizontal(|ui| {
                        ui.radio_value(
                            &mut self.settings.normal_convention,
                            NormalConvention::OpenGl,
                            i18n.tr("normal-convention-opengl"),
                        );
                        ui.radio_value(
                            &mut self.settings.normal_convention,
                            NormalConvention::DirectX,
                            i18n.tr("normal-convention-directx"),
                        );
                    });
                    ui.end_row();
                });

            if let Some(error) = &self.error {
                ui.add_space(8.0);
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            if matches!(self.task, Task::Importing(_)) {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    let mut args = FluentArgs::new();
                    args.set("file", self.path.clone());
                    ui.label(i18n.tr_args("new-project-importing", &args));
                });
            }

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let can_create = !busy && !self.path.trim().is_empty();
                if ui
                    .add_enabled(can_create, egui::Button::new(i18n.tr("new-project-create")))
                    .clicked()
                {
                    self.error = None;
                    let path = PathBuf::from(self.path.trim());
                    tracing::info!(path = %path.display(), "importing mesh");
                    self.task = Task::Importing(spawn(ctx, move || {
                        patine_mesh::import(&path, BuildOptions::default())
                    }));
                }
                if ui.button(i18n.tr("new-project-cancel")).clicked() {
                    outcome = DialogOutcome::Cancelled;
                }
            });
        });
        if response.should_close() && !self.busy() {
            outcome = DialogOutcome::Cancelled;
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    fn fixture(name: &str) -> String {
        format!(
            "{}/../../tests/fixtures/meshes/{name}",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    /// Starts the import of `path` as the Create button does, and polls until it ends.
    fn run_import(path: &str, i18n: &I18n) -> (NewProjectDialog, Option<CreatedProject>) {
        let ctx = egui::Context::default();
        let mut dialog = NewProjectDialog {
            path: path.to_owned(),
            ..NewProjectDialog::default()
        };
        let import_path = PathBuf::from(path);
        dialog.task = Task::Importing(spawn(&ctx, move || {
            patine_mesh::import(&import_path, BuildOptions::default())
        }));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while dialog.busy() {
            assert!(
                std::time::Instant::now() < deadline,
                "import did not finish"
            );
            if let Some(created) = dialog.poll(i18n) {
                return (dialog, Some(created));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        (dialog, None)
    }

    #[test]
    fn import_creates_one_texture_set_per_material() {
        let i18n = I18n::new(Language::English);
        let (_, created) = run_import(&fixture("cube_materials.fbx"), &i18n);
        let created = created.expect("the fixture imports");
        let names: Vec<_> = created
            .project
            .texture_sets
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"Painted") && names.contains(&"Metal"));
        assert!(
            created
                .project
                .texture_sets
                .iter()
                .all(|t| t.resolution == Resolution::DEFAULT)
        );
    }

    #[test]
    fn mesh_without_uvs_shows_a_translated_error() {
        let i18n = I18n::new(Language::French);
        let (dialog, created) = run_import(&fixture("no_uv.glb"), &i18n);
        assert!(created.is_none());
        let error = dialog.error.expect("an error is shown");
        assert!(error.contains("Bare") && error.contains("UV"), "{error}");
        assert!(error.starts_with("Ces objets"), "{error}");
    }

    #[test]
    fn every_mesh_error_is_translated() {
        for language in Language::ALL {
            let i18n = I18n::new(language);
            for key in MeshError::TRANSLATION_KEYS {
                assert!(i18n.has(key), "{language:?}: {key}");
            }
        }
    }

    #[test]
    fn missing_uv_message_names_the_objects() {
        let error = MeshError::MissingUvs {
            objects: vec!["Body".into(), "Wheel".into()],
        };
        for language in Language::ALL {
            let message = error_message(&I18n::new(language), &error);
            assert!(message.contains("Body, Wheel"), "{message}");
            assert!(!message.contains('{'), "unresolved argument: {message}");
        }
    }

    #[test]
    fn every_variant_formats_without_missing_arguments() {
        let errors = [
            MeshError::Io {
                path: "a.fbx".into(),
                source: std::io::Error::other("denied"),
            },
            MeshError::UnsupportedFormat {
                extension: "blend".into(),
            },
            MeshError::Parse {
                format: "FBX",
                message: "bad header".into(),
            },
            MeshError::NoGeometry,
            MeshError::TooManyVertices(5_000_000_000),
            MeshError::Tangents("x".into()),
        ];
        for language in Language::ALL {
            let i18n = I18n::new(language);
            for error in &errors {
                let message = error_message(&i18n, error);
                assert!(
                    !message.contains("{$") && !message.starts_with("error-"),
                    "{message}"
                );
            }
        }
    }
}
