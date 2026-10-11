//! Layers panel: the stack of the current texture set, top layer first.
//!
//! Rows select (click), rename (double click), show or hide (eye) and move by drag and
//! drop: dropped on a layer, the dragged layer goes just above it; on a folder, inside it
//! at the top; below the last row, at the bottom of the stack. The header sets the blend
//! mode and opacity of the current channel for the selected layer. A masked layer shows
//! its mask and the mask's effects below it (see [`crate::masks`]); right-clicking a
//! layer adds or removes its mask.
//!
//! The panel never writes into the project: it returns commands.

use std::collections::HashSet;

use egui::{RichText, Sense, Ui};
use egui_phosphor::regular as icons;
use fluent_bundle::FluentArgs;
use patine_compositor::reference::ALL_MODES;
use patine_core::channel::Channel;
use patine_core::command::{Command, CommandGroup, Label};
use patine_core::commands::{
    InsertLayer, MoveLayer, RemoveLayer, RenameLayer, SetChannelBlend, SetLayerVisible,
};
use patine_core::ids::{IdSequence, LayerId};
use patine_core::layer::{BlendMode, Layer, LayerKind, LayerPosition};
use patine_core::project::{Project, TextureSet, TextureSetId};

use crate::display_panel::gesture_ended;
use crate::document::channel_name;
use crate::i18n::I18n;
use crate::masks::{self, LayerPart};

/// Interface state of the panel.
#[derive(Debug)]
pub struct LayersState {
    /// Selected layer.
    pub selected: Option<LayerId>,
    /// Layer being renamed, with the edited name.
    pub renaming: Option<(LayerId, String)>,
    /// Channel whose blend mode and opacity the header edits.
    pub channel: Channel,
    /// Folders shown closed.
    pub collapsed: HashSet<LayerId>,
    /// Part of the selected layer that is selected (content, mask or mask effect).
    pub part: LayerPart,
}

impl Default for LayersState {
    fn default() -> Self {
        Self {
            selected: None,
            renaming: None,
            channel: Channel::BaseColor,
            collapsed: HashSet::new(),
            part: LayerPart::Content,
        }
    }
}

/// What the panel asks for this frame.
#[derive(Default)]
pub struct LayersOutput {
    /// Commands to execute, in order.
    pub commands: Vec<Box<dyn Command>>,
    /// The current gesture ended.
    pub seal: bool,
}

/// A row of the panel.
struct Row<'a> {
    layer: &'a Layer,
    depth: usize,
    position: LayerPosition,
}

/// Rows from the top of the stack, folders followed by their (open) children.
fn rows<'a>(
    layers: &'a [Layer],
    parent: Option<LayerId>,
    depth: usize,
    collapsed: &HashSet<LayerId>,
    out: &mut Vec<Row<'a>>,
) {
    for (index, layer) in layers.iter().enumerate().rev() {
        out.push(Row {
            layer,
            depth,
            position: LayerPosition { parent, index },
        });
        if let LayerKind::Folder { children } = &layer.kind
            && !collapsed.contains(&layer.id)
        {
            rows(children, Some(layer.id), depth + 1, collapsed, out);
        }
    }
}

/// Translation key of a blend mode.
pub fn mode_key(mode: BlendMode) -> &'static str {
    match mode {
        BlendMode::Normal => "blend-normal",
        BlendMode::Replace => "blend-replace",
        BlendMode::Passthrough => "blend-passthrough",
        BlendMode::Multiply => "blend-multiply",
        BlendMode::Divide => "blend-divide",
        BlendMode::Screen => "blend-screen",
        BlendMode::Overlay => "blend-overlay",
        BlendMode::SoftLight => "blend-soft-light",
        BlendMode::HardLight => "blend-hard-light",
        BlendMode::LinearDodge => "blend-linear-dodge",
        BlendMode::LinearBurn => "blend-linear-burn",
        BlendMode::Subtract => "blend-subtract",
        BlendMode::Difference => "blend-difference",
        BlendMode::Exclusion => "blend-exclusion",
        BlendMode::Darken => "blend-darken",
        BlendMode::Lighten => "blend-lighten",
        BlendMode::ColorDodge => "blend-color-dodge",
        BlendMode::ColorBurn => "blend-color-burn",
        BlendMode::Hue => "blend-hue",
        BlendMode::Saturation => "blend-saturation",
        BlendMode::Color => "blend-color",
        BlendMode::Luminosity => "blend-luminosity",
        BlendMode::Disable => "blend-disable",
        BlendMode::NormalMapCombine => "blend-normal-map-combine",
        BlendMode::NormalMapDetail => "blend-normal-map-detail",
    }
}

/// Modes offered for `channel` of `layer`: folders blend as Normal (or are disabled),
/// normals have their own modes.
pub fn modes_for(layer: &Layer, channel: Channel) -> Vec<BlendMode> {
    if matches!(layer.kind, LayerKind::Folder { .. }) {
        return vec![
            BlendMode::Normal,
            BlendMode::Passthrough,
            BlendMode::Disable,
        ];
    }
    if channel == Channel::Normal {
        return vec![
            BlendMode::NormalMapCombine,
            BlendMode::NormalMapDetail,
            BlendMode::Normal,
            BlendMode::Replace,
            BlendMode::Disable,
        ];
    }
    scalar_modes()
}

/// Modes of color and scalar channels, and of mask effects: every mode but those of
/// folders and normals.
pub fn scalar_modes() -> Vec<BlendMode> {
    ALL_MODES
        .into_iter()
        .filter(|mode| {
            !matches!(
                mode,
                BlendMode::Passthrough | BlendMode::NormalMapCombine | BlendMode::NormalMapDetail
            )
        })
        .collect()
}

/// Edits the blend of `channel` of `layer`: mode list and opacity slider.
pub fn blend_editor(
    ui: &mut Ui,
    i18n: &I18n,
    layer: &Layer,
    channel: Channel,
    output: &mut LayersOutput,
) {
    let Some(blend) = layer.channels.get(&channel).copied() else {
        ui.label(RichText::new(i18n.tr("layers-channel-unused")).weak());
        return;
    };
    let mut changed = blend;
    egui::ComboBox::from_id_salt(("blend mode", layer.id.0, channel.id()))
        .selected_text(i18n.tr(mode_key(blend.mode)))
        .show_ui(ui, |ui| {
            for mode in modes_for(layer, channel) {
                if ui
                    .selectable_label(mode == blend.mode, i18n.tr(mode_key(mode)))
                    .clicked()
                {
                    changed.mode = mode;
                    output.seal = true;
                }
            }
        });
    let response = ui.add(
        egui::Slider::new(&mut changed.opacity, 0.0..=1.0)
            .custom_formatter(|v, _| format!("{:.0} %", v * 100.0))
            .custom_parser(|text| {
                text.trim_end_matches(['%', ' '])
                    .parse::<f64>()
                    .ok()
                    .map(|v| v / 100.0)
            }),
    );
    output.seal |= gesture_ended(&response);
    if changed != blend {
        output.commands.push(Box::new(SetChannelBlend::new(
            layer.id,
            channel,
            Some(changed),
            layer.name.clone(),
        )));
    }
}

/// Where a new layer goes: above the selected layer, or at the top of the stack.
fn insertion(set: &TextureSet, selected: Option<LayerId>) -> LayerPosition {
    selected
        .and_then(|id| set.stack.position(id))
        .map(|position| LayerPosition {
            parent: position.parent,
            index: position.index + 1,
        })
        .unwrap_or(LayerPosition {
            parent: None,
            index: set.stack.layers.len(),
        })
}

/// Number for the name of a new layer: one more than the layers of the stack.
fn next_number(set: &TextureSet) -> usize {
    let mut count = 0;
    set.stack.visit(&mut |_| count += 1);
    count + 1
}

/// A new fill layer above the selection.
pub fn new_fill(
    i18n: &I18n,
    project: &Project,
    set: &TextureSet,
    selected: Option<LayerId>,
) -> InsertLayer {
    let mut ids = IdSequence::starting_at(project.next_id);
    let mut args = FluentArgs::new();
    args.set("number", next_number(set));
    let layer = Layer::fill_for(
        ids.layer(),
        i18n.tr_args("layers-new-fill-name", &args),
        &set.channels,
    );
    InsertLayer::new(set.id, insertion(set, selected), layer)
}

fn folder_layer(i18n: &I18n, project: &Project, set: &TextureSet) -> Layer {
    let mut ids = IdSequence::starting_at(project.next_id);
    let mut args = FluentArgs::new();
    args.set("number", next_number(set));
    let channels: Vec<Channel> = set.channels.iter().map(|def| def.channel).collect();
    Layer::folder(
        ids.layer(),
        i18n.tr_args("layers-new-folder-name", &args),
        &channels,
    )
}

/// A new empty folder above the selection.
pub fn new_folder(
    i18n: &I18n,
    project: &Project,
    set: &TextureSet,
    selected: Option<LayerId>,
) -> InsertLayer {
    InsertLayer::new(
        set.id,
        insertion(set, selected),
        folder_layer(i18n, project, set),
    )
}

/// Puts the selected layer into a new folder at its place (one history entry).
pub fn group(
    i18n: &I18n,
    project: &Project,
    set: &TextureSet,
    selected: LayerId,
) -> Option<CommandGroup> {
    let position = set.stack.position(selected)?;
    let layer = set.stack.find(selected)?;
    let folder = folder_layer(i18n, project, set);
    let folder_id = folder.id;
    Some(CommandGroup::new(
        Label::new("command-group-layers").arg("name", &layer.name),
        vec![
            Box::new(InsertLayer::new(
                set.id,
                LayerPosition {
                    parent: position.parent,
                    index: position.index + 1,
                },
                folder,
            )),
            Box::new(MoveLayer::new(
                selected,
                LayerPosition {
                    parent: Some(folder_id),
                    index: 0,
                },
                layer.name.clone(),
            )),
        ],
    ))
}

/// Deletes the selected layer.
pub fn delete(set: &TextureSet, selected: LayerId) -> Option<RemoveLayer> {
    let layer = set.stack.find(selected)?;
    Some(RemoveLayer::new(selected, layer.name.clone()))
}

/// Where `dragged` goes when dropped on `target` (`None`: below every row).
fn drop_position(
    set: &TextureSet,
    dragged: LayerId,
    target: Option<&Row<'_>>,
) -> Option<LayerPosition> {
    let from = set.stack.position(dragged)?;
    let (parent, index) = match target {
        None => (None, 0),
        Some(row) if row.layer.id == dragged => return None,
        Some(row) => match &row.layer.kind {
            LayerKind::Folder { children } => (Some(row.layer.id), children.len()),
            _ => (row.position.parent, row.position.index + 1),
        },
    };
    // The index counts the remaining layers once the dragged one is taken out.
    let index = if from.parent == parent && from.index < index {
        index - 1
    } else {
        index
    };
    Some(LayerPosition { parent, index })
}

/// Draws the panel.
pub fn show(
    ui: &mut Ui,
    i18n: &I18n,
    project: Option<&Project>,
    set: TextureSetId,
    state: &mut LayersState,
) -> LayersOutput {
    let mut output = LayersOutput::default();
    let Some((project, set)) = project.and_then(|p| p.texture_set(set).map(|s| (p, s))) else {
        state.selected = None;
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new(i18n.tr("layers-empty")).weak());
        });
        return output;
    };
    match state.selected.and_then(|id| set.stack.find(id)) {
        None => {
            state.selected = None;
            state.part = LayerPart::Content;
        }
        Some(layer) => {
            let mask = layer.mask.as_ref();
            let valid = match state.part {
                LayerPart::Content => true,
                LayerPart::Mask => mask.is_some(),
                LayerPart::MaskEffect(id) => mask.and_then(|mask| mask.index_of(id)).is_some(),
            };
            if !valid {
                state.part = LayerPart::Content;
            }
        }
    }
    if !set.channels.iter().any(|def| def.channel == state.channel)
        && let Some(first) = set.channels.first()
    {
        state.channel = first.channel;
    }

    // Header: current channel, blend of the selection.
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("layers channel")
            .selected_text(channel_name(i18n, state.channel))
            .show_ui(ui, |ui| {
                for def in &set.channels {
                    ui.selectable_value(
                        &mut state.channel,
                        def.channel,
                        channel_name(i18n, def.channel),
                    );
                }
            });
        if let Some(layer) = state.selected.and_then(|id| set.stack.find(id)) {
            blend_editor(ui, i18n, layer, state.channel, &mut output);
        }
    });
    ui.horizontal(|ui| {
        if ui
            .button(format!(
                "{} {}",
                icons::PAINT_BUCKET,
                i18n.tr("layers-new-fill")
            ))
            .clicked()
        {
            output
                .commands
                .push(Box::new(new_fill(i18n, project, set, state.selected)));
            state.selected = Some(patine_core::ids::LayerId(project.next_id));
            state.part = LayerPart::Content;
            output.seal = true;
        }
        if ui
            .button(format!(
                "{} {}",
                icons::FOLDER_PLUS,
                i18n.tr("layers-new-folder")
            ))
            .clicked()
        {
            output
                .commands
                .push(Box::new(new_folder(i18n, project, set, state.selected)));
            state.selected = Some(patine_core::ids::LayerId(project.next_id));
            state.part = LayerPart::Content;
            output.seal = true;
        }
        let selected = state.selected;
        if ui
            .add_enabled(selected.is_some(), egui::Button::new(icons::TRASH))
            .on_hover_text(i18n.tr("action-layer-delete"))
            .clicked()
            && let Some(command) =
                selected.and_then(|id| masks::delete_selection(set, id, state.part))
        {
            output.commands.push(command);
            if state.part == LayerPart::Content {
                state.selected = None;
            }
            state.part = LayerPart::Content;
            output.seal = true;
        }
    });
    ui.separator();

    let mut all = Vec::new();
    rows(&set.stack.layers, None, 0, &state.collapsed, &mut all);
    if all.is_empty() {
        ui.label(RichText::new(i18n.tr("layers-no-layer")).weak());
    }
    let mut dropped: Option<(LayerId, Option<usize>)> = None;
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            for (index, row) in all.iter().enumerate() {
                let layer = row.layer;
                let response = ui
                    .horizontal(|ui| {
                        ui.add_space(16.0 * row.depth as f32);
                        if let LayerKind::Folder { .. } = layer.kind {
                            let open = !state.collapsed.contains(&layer.id);
                            let icon = if open {
                                icons::CARET_DOWN
                            } else {
                                icons::CARET_RIGHT
                            };
                            if ui.small_button(icon).clicked() {
                                if open {
                                    state.collapsed.insert(layer.id);
                                } else {
                                    state.collapsed.remove(&layer.id);
                                }
                            }
                        }
                        let eye = if layer.visible {
                            icons::EYE
                        } else {
                            icons::EYE_SLASH
                        };
                        if ui.small_button(eye).clicked() {
                            output.commands.push(Box::new(SetLayerVisible::new(
                                layer.id,
                                !layer.visible,
                                layer.name.clone(),
                            )));
                            output.seal = true;
                        }
                        let kind = match layer.kind {
                            LayerKind::Folder { .. } => icons::FOLDER,
                            _ => icons::PAINT_BUCKET,
                        };
                        match &mut state.renaming {
                            Some((id, name)) if *id == layer.id => {
                                let edit =
                                    ui.add(egui::TextEdit::singleline(name).desired_width(160.0));
                                if !edit.has_focus() && !edit.lost_focus() {
                                    edit.request_focus();
                                }
                                if edit.lost_focus() {
                                    let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
                                    let name = name.trim().to_owned();
                                    if !cancelled && !name.is_empty() && name != layer.name {
                                        output
                                            .commands
                                            .push(Box::new(RenameLayer::new(layer.id, name)));
                                        output.seal = true;
                                    }
                                    state.renaming = None;
                                }
                                None
                            }
                            _ => {
                                let text = format!("{kind} {}", layer.name);
                                let text = if layer.visible {
                                    RichText::new(text)
                                } else {
                                    RichText::new(text).weak()
                                };
                                let response = ui.add(
                                    egui::Button::selectable(
                                        state.selected == Some(layer.id)
                                            && state.part == LayerPart::Content,
                                        text,
                                    )
                                    .sense(Sense::click_and_drag()),
                                );
                                response.dnd_set_drag_payload(layer.id);
                                if response.clicked() {
                                    state.selected = Some(layer.id);
                                    state.part = LayerPart::Content;
                                }
                                if response.double_clicked() {
                                    state.renaming = Some((layer.id, layer.name.clone()));
                                }
                                Some(response)
                            }
                        }
                    })
                    .inner;
                if let Some(response) = response {
                    response.context_menu(|ui| masks::layer_menu(ui, i18n, layer, &mut output));
                    if let Some(payload) = response.dnd_hover_payload::<LayerId>()
                        && *payload != layer.id
                    {
                        let rect = response.rect;
                        ui.painter().hline(
                            rect.x_range(),
                            rect.top(),
                            egui::Stroke::new(2.0, ui.visuals().selection.bg_fill),
                        );
                    }
                    if let Some(payload) = response.dnd_release_payload::<LayerId>() {
                        dropped = Some((*payload, Some(index)));
                    }
                }
                masks::mask_rows(ui, i18n, project, layer, row.depth + 1, state, &mut output);
            }
            // The space below the rows drops at the bottom of the stack.
            let rest = ui.allocate_response(
                ui.available_size().max(egui::vec2(1.0, 24.0)),
                Sense::hover(),
            );
            if let Some(payload) = rest.dnd_release_payload::<LayerId>() {
                dropped = Some((*payload, None));
            }
        });
    if let Some((dragged, target)) = dropped
        && let Some(to) = drop_position(set, dragged, target.map(|i| &all[i]))
        && let Some(layer) = set.stack.find(dragged)
    {
        output
            .commands
            .push(Box::new(MoveLayer::new(dragged, to, layer.name.clone())));
        output.seal = true;
    }
    output
}

/// Every translation key of the panel, for tests.
pub const TRANSLATION_KEYS: [&str; 9] = [
    "layers-empty",
    "layers-no-layer",
    "layers-new-fill",
    "layers-new-folder",
    "layers-new-fill-name",
    "layers-new-folder-name",
    "layers-channel-unused",
    "action-layer-delete",
    "command-group-layers",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;
    use patine_core::layer::{FillSource, Projection};

    fn fill(id: u64) -> Layer {
        Layer::new(
            LayerId(id),
            "fill",
            LayerKind::Fill {
                source: FillSource::Uniform(Default::default()),
                projection: Projection::default(),
            },
            &[Channel::BaseColor],
        )
    }

    #[test]
    fn texts_and_modes_are_translated() {
        for language in Language::ALL {
            let i18n = I18n::new(language);
            for key in TRANSLATION_KEYS {
                assert!(i18n.has(key), "{language:?}: {key}");
            }
            for mode in ALL_MODES {
                assert!(i18n.has(mode_key(mode)), "{language:?}: {mode:?}");
            }
        }
    }

    #[test]
    fn rows_go_from_the_top_and_drops_land_above_their_target() {
        let mut folder = Layer::folder(LayerId(2), "folder", &[Channel::BaseColor]);
        folder.kind = LayerKind::Folder {
            children: vec![fill(3), fill(4)],
        };
        let mut set = patine_core::project::Project::new(
            patine_core::mesh::MeshAsset {
                materials: vec!["m".into()],
                ..Default::default()
            },
            "m.glb".into(),
            Default::default(),
        )
        .texture_sets
        .remove(0);
        set.stack.layers = vec![fill(1), folder, fill(5)];
        let mut all = Vec::new();
        rows(&set.stack.layers, None, 0, &HashSet::new(), &mut all);
        let ids: Vec<u64> = all.iter().map(|r| r.layer.id.0).collect();
        assert_eq!(ids, [5, 2, 4, 3, 1]);
        assert_eq!(all[2].depth, 1);

        // Layer 1 dropped on layer 5: above it, at the top (index counted without 1).
        assert_eq!(
            drop_position(&set, LayerId(1), Some(&all[0])),
            Some(LayerPosition {
                parent: None,
                index: 2
            })
        );
        // Layer 5 dropped on the folder: inside, at the top.
        assert_eq!(
            drop_position(&set, LayerId(5), Some(&all[1])),
            Some(LayerPosition {
                parent: Some(LayerId(2)),
                index: 2
            })
        );
        // Layer 3 dropped below everything: bottom of the stack.
        assert_eq!(
            drop_position(&set, LayerId(3), None),
            Some(LayerPosition {
                parent: None,
                index: 0
            })
        );
        // On itself: nothing.
        assert_eq!(drop_position(&set, LayerId(5), Some(&all[0])), None);

        let mut collapsed = HashSet::new();
        collapsed.insert(LayerId(2));
        let mut closed = Vec::new();
        rows(&set.stack.layers, None, 0, &collapsed, &mut closed);
        assert_eq!(closed.len(), 3);
    }
}
