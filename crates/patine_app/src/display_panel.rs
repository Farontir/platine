//! Display Settings panel: environment, exposure, tone mapping, background, overlays,
//! projection and the preview material.
//!
//! HDRI files are picked and decoded on a worker thread; the application then processes
//! the image on the GPU (see [`ViewState::pending_environment`]).

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

use egui::Ui;
use fluent_bundle::FluentArgs;
use patine_render::camera::Projection;
use patine_render::environment::{EquirectImage, studio_sky};
use patine_render::settings::{Background, ToneMapper};

use crate::i18n::I18n;
use crate::viewport_panel::ViewState;

/// Width of the procedural studio sky used by the application.
pub const STUDIO_SKY_WIDTH: u32 = 1024;

/// Result of the loading thread: `None` when the file dialog was cancelled, otherwise the
/// file name and decoded image, or an error message.
type LoadOutcome = Option<Result<(String, EquirectImage), String>>;

/// Environment loading state, kept in [`ViewState`].
#[derive(Default)]
pub struct EnvironmentState {
    /// Display name of the current environment; `None` for the procedural studio sky.
    pub name: Option<String>,
    /// File being picked and decoded.
    task: Option<Receiver<LoadOutcome>>,
    /// Message of the last failure.
    pub error: Option<String>,
    /// Decoded image waiting for the application to process it on the GPU.
    pub pending: Option<EquirectImage>,
}

impl std::fmt::Debug for EnvironmentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentState")
            .field("name", &self.name)
            .field("loading", &self.task.is_some())
            .field("error", &self.error)
            .finish()
    }
}

impl EnvironmentState {
    fn poll(&mut self) {
        let Some(task) = &self.task else {
            return;
        };
        match task.try_recv() {
            Ok(Some(Ok((name, image)))) => {
                self.name = Some(name);
                self.pending = Some(image);
                self.error = None;
                self.task = None;
            }
            Ok(Some(Err(message))) => {
                self.error = Some(message);
                self.task = None;
            }
            // Dialog cancelled, or the thread ended.
            Ok(None) | Err(TryRecvError::Disconnected) => self.task = None,
            Err(TryRecvError::Empty) => {}
        }
    }

    fn load_file(&mut self, ctx: &egui::Context, filter_name: String) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = rfd::FileDialog::new()
                .add_filter(filter_name, &["hdr", "exr"])
                .pick_file()
                .map(|path: PathBuf| {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    tracing::info!(path = %path.display(), "loading environment");
                    EquirectImage::load(&path)
                        .map(|image| (name, image))
                        .map_err(|error| error.to_string())
                });
            // The panel may be gone; nobody listens then.
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.task = Some(receiver);
    }
}

fn tone_mapper_key(operator: ToneMapper) -> &'static str {
    match operator {
        ToneMapper::AgX => "tone-mapper-agx",
        ToneMapper::Aces => "tone-mapper-aces",
        ToneMapper::PbrNeutral => "tone-mapper-neutral",
        ToneMapper::Linear => "tone-mapper-linear",
    }
}

/// Every translation key used by the panel, for tests.
pub const TRANSLATION_KEYS: [&str; 26] = [
    "display-metallic",
    "display-roughness",
    "display-environment",
    "display-environment-studio",
    "display-load-hdri",
    "display-hdri-filter",
    "display-loading",
    "display-rotation",
    "display-exposure",
    "display-tone-mapper",
    "tone-mapper-agx",
    "tone-mapper-aces",
    "tone-mapper-neutral",
    "tone-mapper-linear",
    "display-background",
    "display-background-environment",
    "display-background-color",
    "display-blur",
    "display-overlays",
    "display-projection",
    "display-perspective",
    "display-orthographic",
    "display-fov",
    "display-material",
    "display-base-color",
    "error-environment",
];

/// Draws the panel.
pub fn show(ui: &mut Ui, view: &mut ViewState, i18n: &I18n) {
    view.environment.poll();
    let settings = &mut view.settings;

    ui.heading(i18n.tr("display-environment"));
    ui.horizontal(|ui| {
        let name = view
            .environment
            .name
            .clone()
            .unwrap_or_else(|| i18n.tr("display-environment-studio").into_owned());
        egui::ComboBox::from_id_salt("environment_choice")
            .selected_text(name)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(
                        view.environment.name.is_none(),
                        i18n.tr("display-environment-studio"),
                    )
                    .clicked()
                {
                    view.environment.name = None;
                    view.environment.pending = Some(studio_sky(STUDIO_SKY_WIDTH));
                }
            });
        if view.environment.task.is_some() {
            ui.spinner();
            ui.label(i18n.tr("display-loading"));
        } else if ui.button(i18n.tr("display-load-hdri")).clicked() {
            let filter = i18n.tr("display-hdri-filter").into_owned();
            view.environment.load_file(ui.ctx(), filter);
        }
    });
    if let Some(error) = &view.environment.error {
        let mut args = FluentArgs::new();
        args.set("message", error.clone());
        ui.colored_label(
            ui.visuals().error_fg_color,
            i18n.tr_args("error-environment", &args),
        );
    }

    egui::Grid::new("display_grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label(i18n.tr("display-rotation"));
            let mut degrees = settings.environment_rotation.to_degrees();
            if ui
                .add(egui::Slider::new(&mut degrees, 0.0..=360.0).suffix("°"))
                .changed()
            {
                settings.environment_rotation = degrees.to_radians();
            }
            ui.end_row();

            ui.label(i18n.tr("display-exposure"));
            ui.add(egui::Slider::new(&mut settings.exposure_ev, -6.0..=6.0).step_by(0.1));
            ui.end_row();

            ui.label(i18n.tr("display-tone-mapper"));
            egui::ComboBox::from_id_salt("tone_mapper")
                .selected_text(i18n.tr(tone_mapper_key(settings.tone_mapper)))
                .show_ui(ui, |ui| {
                    for operator in ToneMapper::ALL {
                        ui.selectable_value(
                            &mut settings.tone_mapper,
                            operator,
                            i18n.tr(tone_mapper_key(operator)),
                        );
                    }
                });
            ui.end_row();

            ui.label(i18n.tr("display-background"));
            ui.horizontal(|ui| {
                let is_environment = matches!(settings.background, Background::Environment { .. });
                if ui
                    .radio(is_environment, i18n.tr("display-background-environment"))
                    .clicked()
                    && !is_environment
                {
                    settings.background = Background::Environment { blur: 0.6 };
                }
                if ui
                    .radio(!is_environment, i18n.tr("display-background-color"))
                    .clicked()
                    && is_environment
                {
                    settings.background = Background::Solid([0.18; 3]);
                }
            });
            ui.end_row();

            match &mut settings.background {
                Background::Environment { blur } => {
                    ui.label(i18n.tr("display-blur"));
                    ui.add(egui::Slider::new(blur, 0.0..=1.0));
                }
                Background::Solid(color) => {
                    ui.label(i18n.tr("display-background-color"));
                    ui.color_edit_button_rgb(color);
                }
            }
            ui.end_row();

            ui.label(i18n.tr("display-overlays"));
            ui.horizontal(|ui| {
                ui.checkbox(&mut settings.shadows, i18n.tr("action-view-shadows"));
                ui.checkbox(&mut settings.wireframe, i18n.tr("action-view-wireframe"));
            });
            ui.end_row();

            let camera = &mut view.camera;
            ui.label(i18n.tr("display-projection"));
            ui.horizontal(|ui| {
                ui.radio_value(
                    &mut camera.projection,
                    Projection::Perspective,
                    i18n.tr("display-perspective"),
                );
                ui.radio_value(
                    &mut camera.projection,
                    Projection::Orthographic,
                    i18n.tr("display-orthographic"),
                );
            });
            ui.end_row();

            ui.label(i18n.tr("display-fov"));
            let mut fov = camera.fov_y.to_degrees();
            if ui
                .add(egui::Slider::new(&mut fov, 10.0..=100.0).suffix("°"))
                .changed()
            {
                camera.fov_y = fov.to_radians();
            }
            ui.end_row();
        });

    ui.separator();
    ui.heading(i18n.tr("display-material"));
    let material = &mut settings.material;
    egui::Grid::new("material_grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label(i18n.tr("display-base-color"));
            ui.color_edit_button_rgb(&mut material.base_color);
            ui.end_row();
            ui.label(i18n.tr("display-metallic"));
            ui.add(egui::Slider::new(&mut material.metallic, 0.0..=1.0));
            ui.end_row();
            ui.label(i18n.tr("display-roughness"));
            ui.add(egui::Slider::new(&mut material.roughness, 0.0..=1.0));
            ui.end_row();
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn every_key_is_translated() {
        for language in Language::ALL {
            let i18n = I18n::new(language);
            for key in TRANSLATION_KEYS {
                assert!(i18n.has(key), "{language:?}: {key}");
            }
        }
    }
}
