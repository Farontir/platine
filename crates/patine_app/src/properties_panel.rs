//! Properties panel: the selected layer. For a uniform fill, each channel of the texture
//! set with its activation, its value, its blend mode and its opacity; for a folder, the
//! blend of each channel.
//!
//! The panel never writes into the project: it returns commands.

use egui::{RichText, Ui};
use fluent_bundle::FluentArgs;
use patine_core::channel::{Channel, ChannelDef};
use patine_core::command::{Command, CommandGroup, Label};
use patine_core::commands::{SetChannelBlend, SetFillValue};
use patine_core::ids::LayerId;
use patine_core::layer::{ChannelBlend, FillSource, Layer, LayerKind};
use patine_core::project::{Project, TextureSetId};

use crate::display_panel::gesture_ended;
use crate::document::channel_name;
use crate::i18n::I18n;
use crate::layers_panel::{LayersOutput, blend_editor};

/// Editor of a fill value: a color, a slider, or the tilt of a normal.
fn value_editor(ui: &mut Ui, channel: Channel, value: [f32; 4]) -> (Option<[f32; 4]>, bool) {
    match channel {
        Channel::BaseColor | Channel::Emissive => {
            let mut rgb = [value[0], value[1], value[2]];
            let response = ui.color_edit_button_rgb(&mut rgb);
            let changed = response
                .changed()
                .then_some([rgb[0], rgb[1], rgb[2], value[3]]);
            (changed, response.clicked())
        }
        Channel::Normal => {
            // The tilt (x, y); z follows so that the normal stays a unit vector.
            let (mut x, mut y) = (value[0], value[1]);
            let rx = ui.add(egui::Slider::new(&mut x, -1.0..=1.0).text("x"));
            let ry = ui.add(egui::Slider::new(&mut y, -1.0..=1.0).text("y"));
            let ended = gesture_ended(&rx) || gesture_ended(&ry);
            if rx.changed() || ry.changed() {
                let length = (x * x + y * y).sqrt();
                let (x, y) = if length > 0.99 {
                    (x * 0.99 / length, y * 0.99 / length)
                } else {
                    (x, y)
                };
                let z = (1.0 - x * x - y * y).max(0.0).sqrt();
                (Some([x, y, z, 0.0]), ended)
            } else {
                (None, ended)
            }
        }
        _ => {
            let mut v = value[0];
            let range = if channel == Channel::Height {
                -1.0..=1.0
            } else {
                0.0..=1.0
            };
            let response = ui.add(egui::Slider::new(&mut v, range));
            let changed = response.changed().then_some([v; 4]);
            (changed, gesture_ended(&response))
        }
    }
}

/// Activating a channel of a fill: its blend, and a value (the default) if it has none.
fn activate(layer: &Layer, def: &ChannelDef, has_value: bool) -> Box<dyn Command> {
    let mut commands: Vec<Box<dyn Command>> = vec![Box::new(SetChannelBlend::new(
        layer.id,
        def.channel,
        Some(ChannelBlend::standard(def.channel)),
        layer.name.clone(),
    ))];
    if !has_value {
        commands.push(Box::new(SetFillValue::new(
            layer.id,
            def.channel,
            Some(def.default),
            layer.name.clone(),
        )));
    }
    Box::new(CommandGroup::new(
        Label::new("command-activate-channel")
            .arg("name", &layer.name)
            .arg("channel", def.channel.id()),
        commands,
    ))
}

/// Draws the panel for the layer `selected` of texture set `set`.
pub fn show(
    ui: &mut Ui,
    i18n: &I18n,
    project: Option<&Project>,
    set: TextureSetId,
    selected: Option<LayerId>,
) -> LayersOutput {
    let mut output = LayersOutput::default();
    let found = project.and_then(|p| p.texture_set(set)).and_then(|set| {
        selected
            .and_then(|id| set.stack.find(id))
            .map(|layer| (set, layer))
    });
    let Some((set, layer)) = found else {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new(i18n.tr("properties-empty")).weak());
        });
        return output;
    };
    ui.heading(&layer.name);
    let values = match &layer.kind {
        LayerKind::Fill {
            source: FillSource::Uniform(values),
            ..
        } => Some(values),
        _ => None,
    };
    let mut args = FluentArgs::new();
    args.set("set", set.name.clone());
    ui.label(RichText::new(i18n.tr_args("properties-channels", &args)).small());
    ui.separator();
    for def in &set.channels {
        let channel = def.channel;
        let active = layer.channels.contains_key(&channel);
        ui.horizontal(|ui| {
            let mut checked = active;
            if ui
                .checkbox(&mut checked, channel_name(i18n, channel))
                .changed()
            {
                output.seal = true;
                output.commands.push(if checked {
                    activate(
                        layer,
                        def,
                        values.is_some_and(|v| v.contains_key(&channel)) || values.is_none(),
                    )
                } else {
                    Box::new(SetChannelBlend::new(
                        layer.id,
                        channel,
                        None,
                        layer.name.clone(),
                    ))
                });
            }
        });
        if !active {
            continue;
        }
        ui.indent(("channel", channel.id()), |ui| {
            if let Some(values) = values {
                let value = values.get(&channel).copied().unwrap_or(def.default);
                let (changed, ended) = value_editor(ui, channel, value);
                output.seal |= ended;
                if let Some(value) = changed {
                    output.commands.push(Box::new(SetFillValue::new(
                        layer.id,
                        channel,
                        Some(value),
                        layer.name.clone(),
                    )));
                }
            }
            ui.horizontal(|ui| blend_editor(ui, i18n, layer, channel, &mut output));
        });
    }
    output
}
