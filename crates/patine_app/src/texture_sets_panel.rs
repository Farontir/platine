//! Texture Sets panel: the texture sets of the project (current one, name, resolution)
//! and the channels of the current one (default values, adding and removing).
//!
//! The panel never writes into the project: it returns commands.

use egui::{RichText, Ui};
use egui_phosphor::regular as icons;
use fluent_bundle::FluentArgs;
use patine_core::channel::{Channel, ChannelDef};
use patine_core::command::Command;
use patine_core::commands::{RenameTextureSet, SetChannelDef, SetTextureSetResolution};
use patine_core::project::{Project, Resolution, TextureSet, TextureSetId};

use crate::display_panel::gesture_ended;
use crate::document::channel_name;
use crate::i18n::I18n;

/// Interface state of the panel.
#[derive(Debug)]
pub struct TextureSetsState {
    /// Texture set being renamed, with the edited name.
    pub renaming: Option<(TextureSetId, String)>,
    /// Current texture set: its channels are listed, the 2D view shows it.
    pub selected: TextureSetId,
}

impl Default for TextureSetsState {
    fn default() -> Self {
        Self {
            renaming: None,
            selected: TextureSetId(0),
        }
    }
}

/// What the panel asks for this frame.
#[derive(Default)]
pub struct TextureSetsOutput {
    /// Command to execute.
    pub command: Option<Box<dyn Command>>,
    /// The current gesture ended.
    pub seal: bool,
}

/// Channels that can be added: the standard ones and four user channels.
fn addable() -> impl Iterator<Item = Channel> {
    Channel::STANDARD
        .into_iter()
        .chain((0..4).map(Channel::User))
}

/// Draws the panel. `gpu_memory` is the memory of the channel textures, when known.
pub fn show(
    ui: &mut Ui,
    i18n: &I18n,
    project: Option<&Project>,
    state: &mut TextureSetsState,
    gpu_memory: Option<u64>,
) -> TextureSetsOutput {
    let mut output = TextureSetsOutput::default();
    let Some(project) = project else {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new(i18n.tr("texture-sets-empty")).weak());
        });
        state.renaming = None;
        return output;
    };
    if project.texture_set(state.selected).is_none() {
        state.selected = TextureSetId(0);
    }
    let mut args = FluentArgs::new();
    args.set("name", project.mesh.name.clone());
    args.set("triangles", project.mesh.triangle_count());
    args.set("vertices", project.mesh.vertex_count());
    ui.label(i18n.tr_args("texture-sets-mesh", &args));
    ui.separator();

    egui::Grid::new("texture_sets_grid")
        .num_columns(3)
        .striped(true)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            ui.strong(i18n.tr("texture-sets-name"));
            ui.strong(i18n.tr("texture-sets-resolution"));
            ui.strong(i18n.tr("texture-sets-triangles"));
            ui.end_row();
            for set in &project.texture_sets {
                name_cell(ui, i18n, set, state, &mut output);
                resolution_cell(ui, set, &mut output);
                ui.label(
                    project
                        .mesh
                        .material_triangle_count(set.material_index)
                        .to_string(),
                );
                ui.end_row();
            }
        });

    if let Some(set) = project.texture_set(state.selected) {
        ui.separator();
        channels(ui, i18n, set, &mut output);
    }

    if let Some(bytes) = gpu_memory {
        ui.separator();
        let mut args = FluentArgs::new();
        args.set(
            "document",
            format!("{:.0}", bytes as f64 / (1 << 20) as f64),
        );
        ui.label(RichText::new(i18n.tr_args("texture-sets-memory", &args)).small());
    }
    output
}

/// Name of a texture set: a click selects it, a double click renames it.
fn name_cell(
    ui: &mut Ui,
    i18n: &I18n,
    set: &TextureSet,
    state: &mut TextureSetsState,
    output: &mut TextureSetsOutput,
) {
    match &mut state.renaming {
        Some((id, name)) if *id == set.id => {
            let response = ui.add(egui::TextEdit::singleline(name).desired_width(140.0));
            if !response.has_focus() && !response.lost_focus() {
                response.request_focus();
            }
            if response.lost_focus() {
                let cancelled = ui.input(|input| input.key_pressed(egui::Key::Escape));
                let name = name.trim().to_owned();
                if !cancelled && !name.is_empty() && name != set.name {
                    output.command = Some(Box::new(RenameTextureSet::new(set.id, name)));
                    output.seal = true;
                }
                state.renaming = None;
            }
        }
        _ => {
            let label = ui
                .selectable_label(state.selected == set.id, &set.name)
                .on_hover_text(i18n.tr("texture-sets-select-hint"));
            if label.clicked() {
                state.selected = set.id;
            }
            if label.double_clicked() {
                state.renaming = Some((set.id, set.name.clone()));
            }
        }
    }
}

/// Resolution of a texture set, as a list of the supported sizes.
fn resolution_cell(ui: &mut Ui, set: &TextureSet, output: &mut TextureSetsOutput) {
    let label = |r: Resolution| format!("{0} × {0}", r.pixels());
    egui::ComboBox::from_id_salt(("resolution", set.id.0))
        .selected_text(label(set.resolution))
        .show_ui(ui, |ui| {
            for resolution in Resolution::ALL {
                if ui
                    .selectable_label(resolution == set.resolution, label(resolution))
                    .clicked()
                    && resolution != set.resolution
                {
                    output.command =
                        Some(Box::new(SetTextureSetResolution::new(set.id, resolution)));
                    output.seal = true;
                }
            }
        });
}

/// Channels of the current texture set: default values, removal, addition.
fn channels(ui: &mut Ui, i18n: &I18n, set: &TextureSet, output: &mut TextureSetsOutput) {
    let mut args = FluentArgs::new();
    args.set("name", set.name.clone());
    ui.strong(i18n.tr_args("texture-sets-channels", &args));
    egui::Grid::new(("channels", set.id.0))
        .num_columns(3)
        .striped(true)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            for def in &set.channels {
                ui.label(channel_name(i18n, def.channel));
                default_editor(ui, i18n, set.id, def, output);
                if ui
                    .small_button(icons::TRASH)
                    .on_hover_text(i18n.tr("texture-sets-remove-channel"))
                    .clicked()
                {
                    output.command = Some(Box::new(SetChannelDef::remove(set.id, def.channel)));
                    output.seal = true;
                }
                ui.end_row();
            }
        });
    let absent: Vec<Channel> = addable()
        .filter(|channel| !set.channels.iter().any(|def| def.channel == *channel))
        .collect();
    ui.add_enabled_ui(!absent.is_empty(), |ui| {
        ui.menu_button(
            format!("{} {}", icons::PLUS, i18n.tr("texture-sets-add-channel")),
            |ui| {
                for channel in absent {
                    if ui.button(channel_name(i18n, channel)).clicked() {
                        output.command = Some(Box::new(SetChannelDef::set(
                            set.id,
                            ChannelDef::standard(channel),
                        )));
                        output.seal = true;
                        ui.close();
                    }
                }
            },
        );
    });
}

/// Editor of a channel's default value: a color, a 0–1 slider, or the fixed flat normal.
fn default_editor(
    ui: &mut Ui,
    i18n: &I18n,
    set: TextureSetId,
    def: &ChannelDef,
    output: &mut TextureSetsOutput,
) {
    let mut changed = def.clone();
    match def.channel {
        Channel::BaseColor | Channel::Emissive => {
            let mut rgb = [def.default[0], def.default[1], def.default[2]];
            let response = ui.color_edit_button_rgb(&mut rgb);
            // Opening the picker starts a new gesture.
            output.seal |= response.clicked();
            if response.changed() {
                changed.default[..3].copy_from_slice(&rgb);
            }
        }
        Channel::Normal => {
            ui.label(RichText::new(i18n.tr("texture-sets-normal-default")).weak());
        }
        _ => {
            let mut value = def.default[0];
            let response = ui.add(egui::Slider::new(&mut value, 0.0..=1.0));
            output.seal |= gesture_ended(&response);
            if response.changed() {
                changed.default[0] = value;
            }
        }
    }
    if changed != *def {
        output.command = Some(Box::new(SetChannelDef::set(set, changed)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn panel_texts_are_translated() {
        for language in Language::ALL {
            let i18n = I18n::new(language);
            for key in [
                "texture-sets-channels",
                "texture-sets-add-channel",
                "texture-sets-remove-channel",
                "texture-sets-select-hint",
                "texture-sets-memory",
                "texture-sets-normal-default",
            ] {
                assert!(i18n.has(key), "{language:?}: {key}");
            }
            for channel in addable() {
                assert!(!channel_name(&i18n, channel).is_empty());
            }
        }
    }
}
