//! Editors generated from the procedural definitions: one widget per [`ParamDef`], and
//! the projection of a procedural fill. They return the edited value and whether the
//! gesture ended; the panels turn them into commands.

use egui::Ui;
use egui_phosphor::regular as icons;
use patine_core::layer::{ProceduralRef, Projection, ProjectionKind};
use patine_core::mask::ParamValue;
use patine_procedural::def::{ParamDef, ParamKind, ProceduralDef};

use crate::display_panel::gesture_ended;
use crate::i18n::I18n;

/// What an editor returns: the new value when it changed, and whether a gesture ended.
pub struct Edited<T> {
    /// New value, if changed this frame.
    pub value: Option<T>,
    /// The current gesture ended (seal the history entry).
    pub ended: bool,
}

/// A seed from the clock, for the dice button.
fn random_seed() -> i32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    (patine_procedural::reference::hash::pcg(nanos) >> 1) as i32
}

/// Translated name of an option of an enum parameter.
fn option_name(i18n: &I18n, option: &str) -> String {
    i18n.tr(&format!("option-{option}")).into_owned()
}

/// The editor of one parameter; `value` is its resolved value.
fn param_editor(
    ui: &mut Ui,
    i18n: &I18n,
    def: &ParamDef,
    value: &ParamValue,
) -> Edited<ParamValue> {
    let mut ended = false;
    let changed = match (&def.kind, value) {
        (ParamKind::Float { min, max, .. }, ParamValue::Float(v)) => {
            let mut v = *v;
            let response = ui.add(egui::Slider::new(&mut v, *min..=*max).max_decimals(3));
            ended = gesture_ended(&response);
            response.changed().then_some(ParamValue::Float(v))
        }
        (ParamKind::Int { min, max, .. }, ParamValue::Int(v)) => {
            let mut v = *v;
            let response = ui.add(egui::Slider::new(&mut v, *min..=*max));
            ended = gesture_ended(&response);
            response.changed().then_some(ParamValue::Int(v))
        }
        (ParamKind::Seed, ParamValue::Int(v)) => {
            let mut v = *v;
            let response = ui.add(egui::DragValue::new(&mut v).range(0..=i32::MAX));
            ended = gesture_ended(&response);
            let mut changed = response.changed();
            if ui
                .small_button(icons::DICE_FIVE)
                .on_hover_text(i18n.tr("param-random-seed"))
                .clicked()
            {
                v = random_seed();
                changed = true;
                ended = true;
            }
            changed.then_some(ParamValue::Int(v))
        }
        (ParamKind::Bool { .. }, ParamValue::Bool(v)) => {
            let mut v = *v;
            let changed = ui.checkbox(&mut v, "").changed();
            ended = changed;
            changed.then_some(ParamValue::Bool(v))
        }
        (ParamKind::Color { .. }, ParamValue::Color(c)) => {
            let mut rgb = [c[0], c[1], c[2]];
            let response = ui.color_edit_button_rgb(&mut rgb);
            ended = response.clicked_elsewhere() || gesture_ended(&response);
            response
                .changed()
                .then_some(ParamValue::Color([rgb[0], rgb[1], rgb[2], c[3]]))
        }
        (ParamKind::Enum { options, .. }, ParamValue::Enum(current)) => {
            let mut chosen = None;
            egui::ComboBox::from_id_salt(("param", def.name.as_str()))
                .selected_text(option_name(i18n, current))
                .show_ui(ui, |ui| {
                    for option in options {
                        if ui
                            .selectable_label(option == current, option_name(i18n, option))
                            .clicked()
                            && option != current
                        {
                            chosen = Some(ParamValue::Enum(option.clone()));
                        }
                    }
                });
            ended = chosen.is_some();
            chosen
        }
        _ => {
            ui.label(egui::RichText::new(i18n.tr("mask-no-parameter")).weak());
            None
        }
    };
    Edited {
        value: changed,
        ended,
    }
}

/// Editors of every parameter of `instance` (an instance of `def`).
pub fn params_editor(
    ui: &mut Ui,
    i18n: &I18n,
    def: &ProceduralDef,
    instance: &ProceduralRef,
) -> Edited<ProceduralRef> {
    let resolved = def.resolve(instance);
    let mut edited = None;
    let mut ended = false;
    egui::Grid::new(("params", def.id.as_str()))
        .num_columns(2)
        .show(ui, |ui| {
            for param in &def.params {
                ui.label(i18n.tr(&param.label_key));
                let value = resolved
                    .get(&param.name)
                    .cloned()
                    .unwrap_or_else(|| param.default_value());
                ui.horizontal(|ui| {
                    let result = param_editor(ui, i18n, param, &value);
                    ended |= result.ended;
                    if let Some(value) = result.value {
                        let mut changed = instance.clone();
                        changed.version = def.version;
                        changed.params.insert(param.name.clone(), value);
                        edited = Some(changed);
                    }
                });
                ui.end_row();
            }
        });
    Edited {
        value: edited,
        ended,
    }
}

const KINDS: [(&str, ProjectionKind); 5] = [
    ("projection-uv", ProjectionKind::Uv),
    (
        "projection-triplanar",
        ProjectionKind::Triplanar { blend: 0.5 },
    ),
    ("projection-planar", ProjectionKind::Planar),
    ("projection-spherical", ProjectionKind::Spherical),
    ("projection-cylindrical", ProjectionKind::Cylindrical),
];

fn kind_key(kind: ProjectionKind) -> &'static str {
    KINDS
        .iter()
        .find(|(_, k)| std::mem::discriminant(k) == std::mem::discriminant(&kind))
        .map_or("projection-uv", |(key, _)| key)
}

/// Editor of the projection of a fill: kind, scale, rotation, offset, triplanar blend.
pub fn projection_editor(ui: &mut Ui, i18n: &I18n, projection: &Projection) -> Edited<Projection> {
    let mut p = *projection;
    let mut ended = false;
    egui::Grid::new("projection").num_columns(2).show(ui, |ui| {
        ui.label(i18n.tr("projection-kind"));
        egui::ComboBox::from_id_salt("projection kind")
            .selected_text(i18n.tr(kind_key(p.kind)))
            .show_ui(ui, |ui| {
                for (key, kind) in KINDS {
                    let current = std::mem::discriminant(&kind) == std::mem::discriminant(&p.kind);
                    if ui.selectable_label(current, i18n.tr(key)).clicked() && !current {
                        p.kind = kind;
                        ended = true;
                    }
                }
            });
        ui.end_row();
        if let ProjectionKind::Triplanar { blend } = &mut p.kind {
            ui.label(i18n.tr("projection-blend"));
            let response = ui.add(egui::Slider::new(blend, 0.0..=1.0));
            ended |= gesture_ended(&response);
            ui.end_row();
        }
        ui.label(i18n.tr("projection-scale"));
        ui.horizontal(|ui| {
            for value in &mut p.scale {
                let response = ui.add(egui::DragValue::new(value).speed(0.05).range(0.01..=256.0));
                ended |= gesture_ended(&response);
            }
        });
        ui.end_row();
        ui.label(i18n.tr("projection-rotation"));
        let mut degrees = p.rotation.to_degrees();
        let response = ui.add(egui::Slider::new(&mut degrees, -180.0..=180.0).suffix("°"));
        if response.changed() {
            p.rotation = degrees.to_radians();
        }
        ended |= gesture_ended(&response);
        ui.end_row();
        ui.label(i18n.tr("projection-offset"));
        ui.horizontal(|ui| {
            for value in &mut p.offset {
                let response = ui.add(egui::DragValue::new(value).speed(0.01));
                ended |= gesture_ended(&response);
            }
        });
        ui.end_row();
    });
    Edited {
        value: (p != *projection).then_some(p),
        ended,
    }
}

/// Translation keys of the editors, for tests (the library's own keys come from
/// [`patine_procedural::library::translation_keys`]).
pub const TRANSLATION_KEYS: [&str; 11] = [
    "projection-offset",
    "param-random-seed",
    "projection-kind",
    "projection-uv",
    "projection-triplanar",
    "projection-planar",
    "projection-spherical",
    "projection-cylindrical",
    "projection-blend",
    "projection-scale",
    "projection-rotation",
];
