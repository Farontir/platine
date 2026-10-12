//! Layer masks in the interface: the rows of a mask and its effects under their layer in
//! the Layers panel, their context menus, and their editors in the Properties panel.
//!
//! Like the panels, these functions never write into the project: they return commands.

use egui::{RichText, Sense, Ui};
use egui_phosphor::regular as icons;
use patine_core::command::Command;
use patine_core::commands::{
    InsertMaskEffect, MoveMaskEffect, RemoveLayer, RemoveMaskEffect, SetMask, SetMaskEffect,
};
use patine_core::ids::{EffectId, IdSequence, LayerId};
use patine_core::layer::{BlendMode, FillSource, Layer};
use patine_core::mask::{
    FilterKind, LevelsParams, MaskEffect, MaskEffectEntry, MaskInitial, MaskStack,
};
use patine_core::project::{Project, TextureSet};

use crate::display_panel::gesture_ended;
use crate::i18n::I18n;
use crate::layers_panel::{LayersOutput, LayersState, mode_key, scalar_modes};
use crate::procedural_ui::params_editor;
use patine_procedural::def::{Category, ProceduralDef};
use patine_procedural::library;

/// Which part of the selected layer is selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayerPart {
    /// The layer itself (its content and blend).
    #[default]
    Content,
    /// Its mask (initial value).
    Mask,
    /// One effect of its mask.
    MaskEffect(EffectId),
}

/// What is dragged when an effect row is dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EffectDrag {
    layer: LayerId,
    effect: EffectId,
}

/// The mask effects the interface can add.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewEffect {
    /// A uniform fill (white).
    Fill,
    /// Levels (identity).
    Levels,
    /// Inversion.
    Invert,
}

impl NewEffect {
    /// Every effect, in menu order.
    pub const ALL: [NewEffect; 3] = [NewEffect::Fill, NewEffect::Levels, NewEffect::Invert];

    fn effect(self) -> MaskEffect {
        match self {
            NewEffect::Fill => MaskEffect::uniform(1.0),
            NewEffect::Levels => MaskEffect::Levels(LevelsParams::default()),
            NewEffect::Invert => MaskEffect::invert(),
        }
    }

    fn label_key(self) -> &'static str {
        match self {
            NewEffect::Fill => "mask-add-fill",
            NewEffect::Levels => "mask-add-levels",
            NewEffect::Invert => "mask-add-invert",
        }
    }
}

/// A mask fill of the noise or pattern `def`, with its default parameters.
pub fn procedural_fill(def: &ProceduralDef) -> MaskEffect {
    MaskEffect::Fill {
        source: FillSource::Procedural(def.instance()),
    }
}

/// Icon and translation key of a mask effect (a procedural fill is named by its
/// procedural).
pub fn effect_label(effect: &MaskEffect) -> (&'static str, String) {
    let (icon, key) = effect_label_key(effect);
    (icon, key.to_owned())
}

fn effect_label_key(effect: &MaskEffect) -> (&'static str, &str) {
    match effect {
        MaskEffect::Fill {
            source: FillSource::Procedural(procedural),
        } => (
            icons::CLOUD,
            library::find(&procedural.id).map_or("mask-effect-fill", |def| def.name_key.as_str()),
        ),
        MaskEffect::Fill { .. } => (icons::PAINT_BUCKET, "mask-effect-fill"),
        MaskEffect::Levels(_) => (icons::SLIDERS_HORIZONTAL, "mask-effect-levels"),
        MaskEffect::Filter {
            kind: FilterKind::Invert,
            ..
        } => (icons::SELECTION_INVERSE, "mask-effect-invert"),
        _ => (icons::CIRCLE_HALF, "mask-effect-other"),
    }
}

/// Label of the mask display mode: whose mask is shown.
pub fn mask_view_label(
    i18n: &I18n,
    project: Option<&Project>,
    selected: Option<LayerId>,
) -> String {
    let layer = selected.and_then(|id| {
        project?
            .texture_sets
            .iter()
            .find_map(|set| set.stack.find(id))
    });
    match layer {
        None => i18n.tr("display-mask-no-layer").into_owned(),
        Some(layer) => {
            let mut args = fluent_bundle::FluentArgs::new();
            args.set("name", layer.name.clone());
            let key = if layer.mask.is_some() {
                "display-mask-of"
            } else {
                "display-mask-none"
            };
            i18n.tr_args(key, &args)
        }
    }
}

/// Gives `layer` a black or white mask.
pub fn add_mask(layer: &Layer, initial: MaskInitial) -> SetMask {
    SetMask::new(layer.id, Some(MaskStack::new(initial)), layer.name.clone())
}

/// A new effect in the mask of `layer`: just above `above`, or at the top of the mask.
pub fn new_effect(
    project: &Project,
    layer: &Layer,
    kind: NewEffect,
    above: Option<EffectId>,
) -> Option<InsertMaskEffect> {
    insert_effect(project, layer, kind.effect(), BlendMode::Normal, 1.0, above)
}

/// Inserts `effect` with `blend` and `opacity` in the mask of `layer`: just above `above`,
/// or at the top of the mask. Its identifier is the project's next one.
pub fn insert_effect(
    project: &Project,
    layer: &Layer,
    effect: MaskEffect,
    blend: BlendMode,
    opacity: f32,
    above: Option<EffectId>,
) -> Option<InsertMaskEffect> {
    let mask = layer.mask.as_ref()?;
    let index = above
        .and_then(|id| mask.index_of(id))
        .map_or(mask.effects.len(), |index| index + 1);
    let mut ids = IdSequence::starting_at(project.next_id);
    let entry = MaskEffectEntry {
        blend,
        opacity,
        ..MaskEffectEntry::new(ids.effect(), effect)
    };
    Some(InsertMaskEffect::new(
        layer.id,
        index,
        entry,
        layer.name.clone(),
    ))
}

/// Deletes the selected part: an effect, the mask, or the layer.
pub fn delete_selection(
    set: &TextureSet,
    selected: LayerId,
    part: LayerPart,
) -> Option<Box<dyn Command>> {
    let layer = set.stack.find(selected)?;
    let name = layer.name.clone();
    Some(match part {
        LayerPart::Content => Box::new(RemoveLayer::new(selected, name)),
        LayerPart::Mask => Box::new(SetMask::new(selected, None, name)),
        LayerPart::MaskEffect(effect) => {
            layer.mask.as_ref()?.index_of(effect)?;
            Box::new(RemoveMaskEffect::new(selected, effect, name))
        }
    })
}

/// Index from the bottom where the effect at `from` goes when dropped on the effect at
/// `target` (just above it), or on the mask row (`None`: at the top); counted among the
/// other effects. `None` when nothing moves.
pub fn effect_drop_index(from: usize, target: Option<usize>, len: usize) -> Option<usize> {
    let to = match target {
        None => len - 1,
        Some(target) if target == from => return None,
        Some(target) if from < target => target,
        Some(target) => target + 1,
    };
    (to != from).then_some(to)
}

/// Items of the context menu of a layer row.
pub fn layer_menu(ui: &mut Ui, i18n: &I18n, layer: &Layer, output: &mut LayersOutput) {
    if layer.mask.is_none() {
        for (initial, key) in [
            (MaskInitial::Black, "mask-add-black"),
            (MaskInitial::White, "mask-add-white"),
        ] {
            if ui.button(i18n.tr(key)).clicked() {
                output.commands.push(Box::new(add_mask(layer, initial)));
                output.seal = true;
                ui.close();
            }
        }
    } else if ui.button(i18n.tr("mask-remove")).clicked() {
        output
            .commands
            .push(Box::new(SetMask::new(layer.id, None, layer.name.clone())));
        output.seal = true;
        ui.close();
    }
}

/// Items of the context menu of the mask row or of an effect row (`effect`).
fn mask_menu(
    ui: &mut Ui,
    i18n: &I18n,
    project: &Project,
    layer: &Layer,
    effect: Option<EffectId>,
    state: &mut LayersState,
    output: &mut LayersOutput,
) {
    for kind in NewEffect::ALL {
        if ui.button(i18n.tr(kind.label_key())).clicked() {
            if let Some(command) = new_effect(project, layer, kind, effect) {
                output.commands.push(Box::new(command));
                state.selected = Some(layer.id);
                state.part = LayerPart::MaskEffect(EffectId(project.next_id));
            }
            output.seal = true;
            ui.close();
        }
    }
    ui.menu_button(i18n.tr("mask-add-procedural"), |ui| {
        for def in library::all()
            .iter()
            .filter(|def| matches!(def.category, Category::Noise | Category::Pattern))
        {
            if ui.button(i18n.tr(&def.name_key)).clicked() {
                let fill = procedural_fill(def);
                if let Some(command) =
                    insert_effect(project, layer, fill, BlendMode::Normal, 1.0, effect)
                {
                    output.commands.push(Box::new(command));
                    state.selected = Some(layer.id);
                    state.part = LayerPart::MaskEffect(EffectId(project.next_id));
                }
                output.seal = true;
                ui.close();
            }
        }
    });
    ui.separator();
    let (key, part) = match effect {
        Some(effect) => ("mask-remove-effect", LayerPart::MaskEffect(effect)),
        None => ("mask-remove", LayerPart::Mask),
    };
    if ui.button(i18n.tr(key)).clicked() {
        let command = match part {
            LayerPart::MaskEffect(effect) => {
                Box::new(RemoveMaskEffect::new(layer.id, effect, layer.name.clone()))
                    as Box<dyn Command>
            }
            _ => Box::new(SetMask::new(layer.id, None, layer.name.clone())),
        };
        output.commands.push(command);
        state.part = LayerPart::Content;
        output.seal = true;
        ui.close();
    }
}

fn is_selected(state: &LayersState, layer: LayerId, part: LayerPart) -> bool {
    state.selected == Some(layer) && state.part == part
}

/// Draws the rows of the mask of `layer` (the mask, then its effects from the top), at
/// indentation `depth`.
pub fn mask_rows(
    ui: &mut Ui,
    i18n: &I18n,
    project: &Project,
    layer: &Layer,
    depth: usize,
    state: &mut LayersState,
    output: &mut LayersOutput,
) {
    let Some(mask) = &layer.mask else {
        return;
    };
    let mut dropped: Option<(EffectDrag, Option<usize>)> = None;

    let header = ui
        .horizontal(|ui| {
            ui.add_space(16.0 * depth as f32);
            let key = match mask.initial {
                MaskInitial::Black => "mask-row-black",
                MaskInitial::White => "mask-row-white",
            };
            ui.add(egui::Button::selectable(
                is_selected(state, layer.id, LayerPart::Mask),
                format!("{} {}", icons::CIRCLE_HALF, i18n.tr(key)),
            ))
        })
        .inner;
    if header.clicked() {
        state.selected = Some(layer.id);
        state.part = LayerPart::Mask;
    }
    header.context_menu(|ui| mask_menu(ui, i18n, project, layer, None, state, output));
    if let Some(payload) = header.dnd_release_payload::<EffectDrag>() {
        dropped = Some((*payload, None));
    }

    for (index, entry) in mask.effects.iter().enumerate().rev() {
        let row = ui
            .horizontal(|ui| {
                ui.add_space(16.0 * (depth + 1) as f32);
                let eye = if entry.visible {
                    icons::EYE
                } else {
                    icons::EYE_SLASH
                };
                if ui.small_button(eye).clicked() {
                    let mut changed = entry.clone();
                    changed.visible = !entry.visible;
                    output.commands.push(Box::new(SetMaskEffect::new(
                        layer.id,
                        changed,
                        layer.name.clone(),
                    )));
                    output.seal = true;
                }
                let (icon, key) = effect_label(&entry.effect);
                let mut text = format!("{icon} {}", i18n.tr(&key));
                if let Some(value) = entry.effect.fill_value() {
                    text.push_str(&format!(" {:.0} %", value * 100.0));
                }
                if entry.blend != BlendMode::Normal || entry.opacity < 1.0 {
                    text.push_str(&format!(
                        " · {} {:.0} %",
                        i18n.tr(mode_key(entry.blend)),
                        entry.opacity * 100.0
                    ));
                }
                let text = if entry.visible {
                    RichText::new(text)
                } else {
                    RichText::new(text).weak()
                };
                ui.add(
                    egui::Button::selectable(
                        is_selected(state, layer.id, LayerPart::MaskEffect(entry.id)),
                        text,
                    )
                    .sense(Sense::click_and_drag()),
                )
            })
            .inner;
        row.dnd_set_drag_payload(EffectDrag {
            layer: layer.id,
            effect: entry.id,
        });
        if row.clicked() {
            state.selected = Some(layer.id);
            state.part = LayerPart::MaskEffect(entry.id);
        }
        row.context_menu(|ui| mask_menu(ui, i18n, project, layer, Some(entry.id), state, output));
        if let Some(payload) = row.dnd_hover_payload::<EffectDrag>()
            && payload.layer == layer.id
            && payload.effect != entry.id
        {
            ui.painter().hline(
                row.rect.x_range(),
                row.rect.top(),
                egui::Stroke::new(2.0, ui.visuals().selection.bg_fill),
            );
        }
        if let Some(payload) = row.dnd_release_payload::<EffectDrag>() {
            dropped = Some((*payload, Some(index)));
        }
    }

    if let Some((drag, target)) = dropped
        && drag.layer == layer.id
        && let Some(from) = mask.index_of(drag.effect)
        && let Some(to) = effect_drop_index(from, target, mask.effects.len())
    {
        output.commands.push(Box::new(MoveMaskEffect::new(
            layer.id,
            drag.effect,
            to,
            layer.name.clone(),
        )));
        output.seal = true;
    }
}

/// Percent slider for a value in 0..1.
fn percent(value: &mut f32) -> egui::Slider<'_> {
    egui::Slider::new(value, 0.0..=1.0)
        .custom_formatter(|v, _| format!("{:.0} %", v * 100.0))
        .custom_parser(|text| {
            text.trim_end_matches(['%', ' '])
                .parse::<f64>()
                .ok()
                .map(|v| v / 100.0)
        })
}

/// Properties of the mask of `layer`: its initial value.
pub fn mask_properties(ui: &mut Ui, i18n: &I18n, layer: &Layer, output: &mut LayersOutput) {
    let Some(mask) = &layer.mask else {
        return;
    };
    ui.heading(i18n.tr("mask-properties"));
    ui.horizontal(|ui| {
        ui.label(i18n.tr("mask-initial"));
        for (initial, key) in [
            (MaskInitial::Black, "mask-initial-black"),
            (MaskInitial::White, "mask-initial-white"),
        ] {
            if ui
                .selectable_label(mask.initial == initial, i18n.tr(key))
                .clicked()
                && mask.initial != initial
            {
                let mut changed = mask.clone();
                changed.initial = initial;
                output.commands.push(Box::new(SetMask::new(
                    layer.id,
                    Some(changed),
                    layer.name.clone(),
                )));
                output.seal = true;
            }
        }
    });
    ui.label(RichText::new(i18n.tr("mask-hint")).weak().small());
}

/// Draws the curve of `levels` (input → output) in a small square.
fn levels_curve(ui: &mut Ui, levels: &LevelsParams) {
    let size = egui::vec2(120.0, 120.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);
    let point = |x: f32, y: f32| {
        egui::pos2(
            rect.left() + x * rect.width(),
            rect.bottom() - y.clamp(0.0, 1.0) * rect.height(),
        )
    };
    let points: Vec<egui::Pos2> = (0..=64)
        .map(|i| {
            let x = i as f32 / 64.0;
            point(x, patine_compositor::reference::levels(levels, x))
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.5, visuals.selection.stroke.color),
    ));
}

/// Properties of the mask effect `effect` of `layer`: visibility, blend, then its own
/// parameters.
pub fn effect_properties(
    ui: &mut Ui,
    i18n: &I18n,
    layer: &Layer,
    effect: EffectId,
    output: &mut LayersOutput,
) {
    let Some(entry) = layer
        .mask
        .as_ref()
        .and_then(|mask| mask.index_of(effect).map(|index| &mask.effects[index]))
    else {
        return;
    };
    let mut changed = entry.clone();
    let (icon, key) = effect_label(&entry.effect);
    ui.heading(format!("{icon} {}", i18n.tr(&key)));
    if ui
        .checkbox(&mut changed.visible, i18n.tr("mask-visible"))
        .changed()
    {
        output.seal = true;
    }
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(("mask blend", entry.id.0))
            .selected_text(i18n.tr(mode_key(entry.blend)))
            .show_ui(ui, |ui| {
                for mode in scalar_modes() {
                    if ui
                        .selectable_label(mode == entry.blend, i18n.tr(mode_key(mode)))
                        .clicked()
                    {
                        changed.blend = mode;
                        output.seal = true;
                    }
                }
            });
        let response = ui.add(percent(&mut changed.opacity));
        output.seal |= gesture_ended(&response);
    });
    ui.separator();
    match &mut changed.effect {
        MaskEffect::Fill {
            source: FillSource::Procedural(procedural),
        } => match library::find(&procedural.id) {
            Some(def) => {
                let edited = params_editor(ui, i18n, def, procedural);
                output.seal |= edited.ended;
                if let Some(value) = edited.value {
                    *procedural = value;
                }
            }
            None => {
                ui.label(RichText::new(i18n.tr("procedural-unknown")).weak());
            }
        },
        effect @ MaskEffect::Fill { .. } => {
            let mut value = effect.fill_value().unwrap_or(0.0);
            ui.horizontal(|ui| {
                ui.label(i18n.tr("mask-value"));
                let response = ui.add(percent(&mut value));
                output.seal |= gesture_ended(&response);
                if response.changed() {
                    *effect = MaskEffect::uniform(value);
                }
            });
        }
        MaskEffect::Levels(levels) => {
            egui::Grid::new(("levels", entry.id.0))
                .num_columns(2)
                .show(ui, |ui| {
                    let rows: [(&str, &mut f32, std::ops::RangeInclusive<f32>); 5] = [
                        ("mask-levels-in-low", &mut levels.in_low, 0.0..=1.0),
                        ("mask-levels-in-high", &mut levels.in_high, 0.0..=1.0),
                        ("mask-levels-gamma", &mut levels.gamma, 0.1..=10.0),
                        ("mask-levels-out-low", &mut levels.out_low, 0.0..=1.0),
                        ("mask-levels-out-high", &mut levels.out_high, 0.0..=1.0),
                    ];
                    for (key, value, range) in rows {
                        ui.label(i18n.tr(key));
                        let logarithmic = key == "mask-levels-gamma";
                        let response = ui.add(
                            egui::Slider::new(value, range)
                                .logarithmic(logarithmic)
                                .max_decimals(3),
                        );
                        output.seal |= gesture_ended(&response);
                        ui.end_row();
                    }
                });
            levels_curve(ui, levels);
        }
        _ => {
            ui.label(RichText::new(i18n.tr("mask-no-parameter")).weak());
        }
    }
    if changed != *entry {
        output.commands.push(Box::new(SetMaskEffect::new(
            layer.id,
            changed,
            layer.name.clone(),
        )));
    }
}

/// Every translation key of the masks, for tests.
pub const TRANSLATION_KEYS: [&str; 32] = [
    "mask-add-procedural",
    "procedural-unknown",
    "display-mask-no-layer",
    "display-mask-of",
    "display-mask-none",
    "action-view-toggle-mask",
    "mask-add-black",
    "mask-add-white",
    "mask-remove",
    "mask-add-fill",
    "mask-add-levels",
    "mask-add-invert",
    "mask-remove-effect",
    "mask-row-black",
    "mask-row-white",
    "mask-effect-fill",
    "mask-effect-levels",
    "mask-effect-invert",
    "mask-effect-other",
    "mask-properties",
    "mask-initial",
    "mask-initial-black",
    "mask-initial-white",
    "mask-hint",
    "mask-visible",
    "mask-value",
    "mask-levels-in-low",
    "mask-levels-in-high",
    "mask-levels-gamma",
    "mask-levels-out-low",
    "mask-levels-out-high",
    "mask-no-parameter",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn texts_are_translated() {
        for language in Language::ALL {
            let i18n = I18n::new(language);
            for key in TRANSLATION_KEYS {
                assert!(i18n.has(key), "{language:?}: {key}");
            }
        }
    }

    #[test]
    fn dropped_effects_land_above_their_target() {
        // Four effects, indices from the bottom.
        assert_eq!(effect_drop_index(0, Some(2), 4), Some(2));
        assert_eq!(effect_drop_index(3, Some(0), 4), Some(1));
        assert_eq!(effect_drop_index(1, Some(0), 4), None, "already above it");
        assert_eq!(effect_drop_index(2, Some(2), 4), None);
        // On the mask row: at the top.
        assert_eq!(effect_drop_index(0, None, 4), Some(3));
        assert_eq!(effect_drop_index(3, None, 4), None);
    }

    #[test]
    fn new_effects_go_above_the_selection() {
        let mut project = patine_core::project::Project::new(
            patine_core::mesh::MeshAsset {
                materials: vec!["m".into()],
                ..Default::default()
            },
            "m.glb".into(),
            Default::default(),
        );
        let mut layer = Layer::folder(LayerId(1), "folder", &[]);
        project.next_id = 2;
        assert!(new_effect(&project, &layer, NewEffect::Fill, None).is_none());
        layer.mask = Some(MaskStack::new(MaskInitial::Black));
        project.texture_sets[0].stack.layers = vec![layer.clone()];
        let mut insert = new_effect(&project, &layer, NewEffect::Fill, None).unwrap();
        insert.apply(&mut project).unwrap();
        let layer = project.texture_sets[0].stack.layers[0].clone();
        let mut insert = new_effect(&project, &layer, NewEffect::Invert, None).unwrap();
        insert.apply(&mut project).unwrap();
        let layer = project.texture_sets[0].stack.layers[0].clone();
        // Above the fill (id 2), below the inversion (id 3).
        let mut insert =
            new_effect(&project, &layer, NewEffect::Levels, Some(EffectId(2))).unwrap();
        insert.apply(&mut project).unwrap();
        let ids: Vec<u64> = project.texture_sets[0].stack.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .effects
            .iter()
            .map(|entry| entry.id.0)
            .collect();
        assert_eq!(ids, [2, 4, 3]);

        let set = &project.texture_sets[0];
        let delete = |part| delete_selection(set, LayerId(1), part).map(|c| c.label().key);
        assert_eq!(
            delete(LayerPart::MaskEffect(EffectId(4))),
            Some("command-remove-mask-effect")
        );
        assert_eq!(delete(LayerPart::Mask), Some("command-remove-mask"));
        assert_eq!(delete(LayerPart::Content), Some("command-remove-layer"));
        assert!(delete(LayerPart::MaskEffect(EffectId(99))).is_none());
    }
}
