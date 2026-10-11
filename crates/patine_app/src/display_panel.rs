//! Display Settings panel: environment, exposure, tone mapping, background, overlays,
//! projection and the preview material.
//!
//! The environment belongs to the document: the panel shows it and returns the changes
//! the user asks for. HDRI files are picked and decoded on a worker thread.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use egui::Ui;
use fluent_bundle::FluentArgs;
use patine_core::channel::Channel;
use patine_core::environment::{EnvironmentSettings, EnvironmentSource};
use patine_render::camera::Projection;
use patine_render::environment::EquirectImage;
use patine_render::settings::{Background, DisplayMode, ToneMapper};

use crate::document::{EnvironmentChange, channel_name};
use crate::i18n::I18n;
use crate::viewport_panel::ViewState;

/// Width of the procedural studio sky used by the application.
pub const STUDIO_SKY_WIDTH: u32 = 1024;

/// An HDRI picked by the user: file name, contents and decoded image.
pub struct PickedHdri {
    /// File name.
    pub name: String,
    /// File contents, stored in the project.
    pub bytes: Arc<[u8]>,
    /// Decoded image, so that the viewport does not decode it again.
    pub image: EquirectImage,
}

/// Result of the import thread: `None` when the file dialog was cancelled.
type ImportOutcome = Option<Result<PickedHdri, String>>;

/// Import state of the panel, kept in [`ViewState`].
#[derive(Default)]
pub struct EnvironmentState {
    /// File being picked, read and decoded.
    task: Option<Receiver<ImportOutcome>>,
    /// Message of the last failed import.
    error: Option<String>,
    /// HDRIs shipped in `assets/environments`, found on first display.
    bundled: Option<Vec<PathBuf>>,
}

/// The environment as the panel shows it.
pub struct EnvironmentView<'a> {
    /// Current settings (document or draft).
    pub settings: &'a EnvironmentSettings,
    /// Display name of the current source.
    pub name: String,
    /// Whether the viewport is decoding the environment.
    pub loading: bool,
    /// Last decoding error of the viewport.
    pub error: Option<&'a str>,
}

/// What the panel asks for this frame.
#[derive(Default)]
pub struct DisplayOutput {
    /// Environment change.
    pub change: Option<EnvironmentChange>,
    /// A picked HDRI, already decoded; `change` imports it.
    pub picked: Option<PickedHdri>,
    /// The current gesture ended: the next change starts a new history entry.
    pub seal: bool,
}

/// The `assets/environments` directory: `PATINE_ASSETS`, next to the executable, or the
/// source tree during development.
pub fn environments_dir() -> Option<PathBuf> {
    let candidates = [
        std::env::var_os("PATINE_ASSETS").map(PathBuf::from),
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("assets"))),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")),
    ];
    candidates
        .into_iter()
        .flatten()
        .map(|assets| assets.join("environments"))
        .find(|dir| dir.is_dir())
}

/// `.hdr` and `.exr` files of `dir`, sorted by name.
fn list_environments(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("hdr") || e.eq_ignore_ascii_case("exr"))
        })
        .collect();
    files.sort();
    files
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl std::fmt::Debug for EnvironmentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentState")
            .field("importing", &self.task.is_some())
            .field("error", &self.error)
            .finish()
    }
}

/// Reads and decodes the HDRI at `path`.
pub fn read_hdri(path: &std::path::Path) -> Result<PickedHdri, String> {
    let bytes: Arc<[u8]> = std::fs::read(path)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .into();
    let image = EquirectImage::decode(&bytes).map_err(|error| error.to_string())?;
    Ok(PickedHdri {
        name: file_name(path),
        bytes,
        image,
    })
}

impl EnvironmentState {
    fn poll(&mut self) -> Option<PickedHdri> {
        let task = self.task.as_ref()?;
        match task.try_recv() {
            Ok(Some(Ok(picked))) => {
                self.error = None;
                self.task = None;
                Some(picked)
            }
            Ok(Some(Err(message))) => {
                self.error = Some(message);
                self.task = None;
                None
            }
            // Dialog cancelled, or the thread ended.
            Ok(None) | Err(TryRecvError::Disconnected) => {
                self.task = None;
                None
            }
            Err(TryRecvError::Empty) => None,
        }
    }

    /// Picks a file in a native dialog, then reads and decodes it on a worker thread.
    fn import(&mut self, ctx: &egui::Context, filter_name: String) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let path = rfd::FileDialog::new()
                .add_filter(filter_name, &["hdr", "exr"])
                .pick_file();
            let result = path.map(|path| {
                tracing::info!(path = %path.display(), "importing environment");
                read_hdri(&path)
            });
            // The panel may be gone; nobody listens then.
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.task = Some(receiver);
    }

    fn bundled(&mut self) -> &[PathBuf] {
        self.bundled.get_or_insert_with(|| {
            environments_dir()
                .map(|dir| list_environments(&dir))
                .unwrap_or_default()
        })
    }
}

/// Whether the edit of `response` ends a gesture (drag released, value typed, click).
pub fn gesture_ended(response: &egui::Response) -> bool {
    response.drag_stopped() || response.lost_focus() || (response.changed() && !response.dragged())
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
pub const TRANSLATION_KEYS: [&str; 27] = [
    "display-mode",
    "display-mode-lit",
    "display-mode-checker",
    "display-mode-channel",
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
    "display-mode-mask",
    "error-environment",
];

/// Draws the panel and returns the environment changes asked for.
/// Name of a display mode, `channel` being the inspected channel.
pub fn display_mode_label(i18n: &I18n, mode: DisplayMode, channel: Channel) -> String {
    match mode {
        DisplayMode::Lit => i18n.tr("display-mode-lit").into_owned(),
        DisplayMode::UvChecker => i18n.tr("display-mode-checker").into_owned(),
        DisplayMode::Mask => i18n.tr("display-mode-mask").into_owned(),
        DisplayMode::Channel => {
            let mut args = FluentArgs::new();
            args.set("channel", channel_name(i18n, channel));
            i18n.tr_args("display-mode-channel", &args)
        }
    }
}

/// Draws the panel and returns the environment changes asked for. `channels` are those
/// of the current texture set, offered as display modes.
pub fn show(
    ui: &mut Ui,
    view: &mut ViewState,
    i18n: &I18n,
    environment: &EnvironmentView<'_>,
    channels: &[Channel],
) -> DisplayOutput {
    let mut output = DisplayOutput::default();
    if let Some(picked) = view.environment.poll() {
        output.change = Some(EnvironmentChange::Imported {
            name: picked.name.clone(),
            bytes: picked.bytes.clone(),
        });
        output.picked = Some(picked);
        output.seal = true;
    }
    let current = environment.settings;
    let settings = &mut view.settings;

    ui.heading(i18n.tr("display-environment"));
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("environment_choice")
            .selected_text(environment.name.clone())
            .show_ui(ui, |ui| {
                let studio = current.source == EnvironmentSource::Studio;
                if ui
                    .selectable_label(studio, i18n.tr("display-environment-studio"))
                    .clicked()
                    && !studio
                {
                    output.change = Some(EnvironmentChange::Source(EnvironmentSource::Studio));
                    output.seal = true;
                }
                let bundled = view.environment.bundled().to_vec();
                for path in bundled {
                    let source = EnvironmentSource::Bundled(file_name(&path));
                    let selected = current.source == source;
                    if ui.selectable_label(selected, file_name(&path)).clicked() && !selected {
                        output.change = Some(EnvironmentChange::Source(source));
                        output.seal = true;
                    }
                }
            });
        if view.environment.task.is_some() || environment.loading {
            ui.spinner();
            ui.label(i18n.tr("display-loading"));
        } else if ui.button(i18n.tr("display-load-hdri")).clicked() {
            let filter = i18n.tr("display-hdri-filter").into_owned();
            view.environment.import(ui.ctx(), filter);
        }
    });
    for error in [view.environment.error.as_deref(), environment.error]
        .into_iter()
        .flatten()
    {
        let mut args = FluentArgs::new();
        args.set("message", error.to_owned());
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
            let mut degrees = current.rotation.to_degrees();
            let response = ui.add(egui::Slider::new(&mut degrees, 0.0..=360.0).suffix("°"));
            if response.changed() {
                output.change = Some(EnvironmentChange::Rotation(degrees.to_radians()));
            }
            output.seal |= gesture_ended(&response);
            ui.end_row();

            ui.label(i18n.tr("display-exposure"));
            let mut exposure = current.exposure_ev;
            let response = ui.add(egui::Slider::new(&mut exposure, -6.0..=6.0).step_by(0.1));
            if response.changed() {
                output.change = Some(EnvironmentChange::Exposure(exposure));
            }
            output.seal |= gesture_ended(&response);
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
    ui.horizontal(|ui| {
        ui.label(i18n.tr("display-mode"));
        let current = display_mode_label(i18n, view.settings.display, view.inspect_channel);
        egui::ComboBox::from_id_salt("display_mode")
            .selected_text(current)
            .show_ui(ui, |ui| {
                let mut choices = vec![(DisplayMode::Lit, view.inspect_channel)];
                choices.extend(channels.iter().map(|c| (DisplayMode::Channel, *c)));
                choices.push((DisplayMode::Mask, view.inspect_channel));
                choices.push((DisplayMode::UvChecker, view.inspect_channel));
                for (mode, channel) in choices {
                    let selected = view.settings.display == mode
                        && (mode != DisplayMode::Channel || view.inspect_channel == channel);
                    if ui
                        .selectable_label(selected, display_mode_label(i18n, mode, channel))
                        .clicked()
                    {
                        view.settings.display = mode;
                        view.inspect_channel = channel;
                    }
                }
            });
    });
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn bundled_environments_are_found() {
        let dir = environments_dir().expect("assets/environments exists in the source tree");
        let names: Vec<String> = list_environments(&dir)
            .iter()
            .map(|p| file_name(p))
            .collect();
        assert!(
            names.contains(&"studio_small_08_1k.hdr".to_owned()),
            "{names:?}"
        );
        assert!(
            names
                .iter()
                .all(|n| n.ends_with(".hdr") || n.ends_with(".exr"))
        );
    }

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
