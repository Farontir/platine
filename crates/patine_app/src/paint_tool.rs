//! Painting in the interface: the active tool and its brush settings, what the selection
//! lets the brush paint, the brush section of the Properties panel, and the brush cursor
//! drawn over the viewport.
//!
//! The brush is interface state: each stroke records a snapshot of it in the document.

use std::collections::BTreeMap;

use egui::{Color32, RichText, Stroke as Line, Ui};
use egui_phosphor::regular as icons;
use fluent_bundle::FluentArgs;
use glam::{Mat4, Vec2, Vec3};
use patine_core::channel::{Channel, ChannelDef};
use patine_core::commands::PaintTarget;
use patine_core::layer::{FillSource, LayerKind, ProceduralRef, Projection};
use patine_core::mask::{MASK_VALUE_CHANNEL, MaskEffect};
use patine_core::project::TextureSet;
use patine_core::resource::{Resource, ResourceHash};
use patine_core::stroke::{
    BrushSnapshot, Curve, PressureCurves, SizeSpace, StrokeLog, Symmetry, Tool as StrokeTool,
};
use patine_paint::alphas::{ALPHAS, find as find_alpha};
use patine_paint::sampler::SamplerSettings;
use patine_procedural::def::Category;

use crate::brush_presets::{
    BrushPreset, BrushSettings, PresetLibrary, PresetMaterial, SHIPPED_KEYS, file_name, shipped,
};

use crate::display_panel::gesture_ended;
use crate::document::channel_name;
use crate::i18n::I18n;
use crate::masks::LayerPart;

/// The painting tool in use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    /// Deposits the brush's material.
    #[default]
    Brush,
    /// Removes coverage from the checked channels.
    Eraser,
    /// Reads the values under the cursor into the brush, then gives the hand back.
    Picker,
}

/// Smallest and largest brush sizes, in viewport pixels.
pub const SIZE_RANGE: (f32, f32) = (1.0, 1000.0);

/// The preset the brush was last set from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresetChoice {
    /// A shipped preset, by its name's translation key.
    Shipped(&'static str),
    /// A preset of the user, by name.
    User(String),
}

/// Brush settings and the active tool.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolState {
    /// Active tool.
    pub tool: Tool,
    /// Diameter, in viewport pixels.
    pub size: f32,
    /// Paint deposited per dab, 0 to 1.
    pub flow: f32,
    /// Maximum opacity of a stroke, 0 to 1.
    pub opacity: f32,
    /// Edge hardness, 0 to 1.
    pub hardness: f32,
    /// Distance between dabs, as a fraction of the size.
    pub spacing: f32,
    /// Rotation of the dabs, radians.
    pub angle: f32,
    /// Length of the lazy mouse string, in pixels.
    pub smoothing: f32,
    /// Shipped alpha, by identifier; `None` for a round dab.
    pub alpha: Option<&'static str>,
    /// Channels the brush paints (checked) and their values.
    pub channels: BTreeMap<Channel, (bool, [f32; 4])>,
    /// Gray painted into masks.
    pub mask_value: f32,
    /// Random size variation, 0 to 1.
    pub size_jitter: f32,
    /// Random rotation, radians.
    pub angle_jitter: f32,
    /// Random flow variation, 0 to 1.
    pub flow_jitter: f32,
    /// Random offset across the stroke, as a fraction of the size.
    pub scatter: f32,
    /// Dabs turn along the stroke.
    pub follow_path: bool,
    /// The pen tilt turns the dabs.
    pub tilt_angle: bool,
    /// What the pen pressure changes.
    pub pressure: PressureCurves,
    /// Procedural material painted on layers instead of the channel values.
    pub procedural: Option<ProceduralRef>,
    /// Procedural pattern painted into masks instead of the gray.
    pub mask_procedural: Option<ProceduralRef>,
    /// Projection of the procedurals.
    pub projection: Projection,
    /// Symmetry of the strokes painted in the 3D view.
    pub symmetry: Symmetry,
    /// Shipped and user presets.
    pub presets: PresetLibrary,
    /// The preset the brush was last set from.
    pub preset: Option<PresetChoice>,
    /// Strokes started, for their seeds.
    pub strokes_started: u64,
    /// The tool the picker gives the hand back to.
    pub previous_tool: Tool,
    /// Whether the dynamics and symmetry sections start open (captures).
    pub expand_sections: bool,
}

impl Default for ToolState {
    fn default() -> Self {
        let mut state = Self {
            tool: Tool::Brush,
            size: 0.0,
            flow: 0.0,
            opacity: 0.0,
            hardness: 0.0,
            spacing: 0.0,
            angle: 0.0,
            smoothing: 0.0,
            alpha: None,
            channels: BTreeMap::new(),
            mask_value: 1.0,
            size_jitter: 0.0,
            angle_jitter: 0.0,
            flow_jitter: 0.0,
            scatter: 0.0,
            follow_path: false,
            tilt_angle: false,
            pressure: PressureCurves::default(),
            procedural: None,
            mask_procedural: None,
            projection: BrushSnapshot::default_projection(),
            symmetry: Symmetry::default(),
            presets: PresetLibrary::default(),
            preset: None,
            strokes_started: 0,
            previous_tool: Tool::Brush,
            expand_sections: false,
        };
        state.apply_settings(&BrushSettings::default());
        state
    }
}

/// The value and checkbox of `def`'s channel for a brush that never set it: the base
/// color and the roughness are painted, in a terracotta red; the other channels keep
/// their defaults, unchecked.
fn initial(def: &ChannelDef) -> (bool, [f32; 4]) {
    match def.channel {
        Channel::BaseColor => (true, [0.62, 0.22, 0.12, 1.0]),
        Channel::Roughness => (true, [0.45; 4]),
        _ => (false, def.default),
    }
}

impl ToolState {
    /// The brush settings, as a preset records them.
    pub fn settings(&self) -> BrushSettings {
        BrushSettings {
            size: self.size,
            flow: self.flow,
            opacity: self.opacity,
            hardness: self.hardness,
            spacing: self.spacing,
            angle: self.angle,
            smoothing: self.smoothing,
            alpha: self.alpha.map(str::to_owned),
            size_jitter: self.size_jitter,
            angle_jitter: self.angle_jitter,
            flow_jitter: self.flow_jitter,
            scatter: self.scatter,
            follow_path: self.follow_path,
            tilt_angle: self.tilt_angle,
            pressure: self.pressure.clone(),
        }
    }

    /// Sets the brush from `settings` (an unknown alpha gives round dabs).
    pub fn apply_settings(&mut self, settings: &BrushSettings) {
        self.size = settings.size.clamp(SIZE_RANGE.0, SIZE_RANGE.1);
        self.flow = settings.flow;
        self.opacity = settings.opacity;
        self.hardness = settings.hardness;
        self.spacing = settings.spacing;
        self.angle = settings.angle;
        self.smoothing = settings.smoothing;
        self.alpha = settings
            .alpha
            .as_deref()
            .and_then(find_alpha)
            .map(|alpha| alpha.id);
        self.size_jitter = settings.size_jitter;
        self.angle_jitter = settings.angle_jitter;
        self.flow_jitter = settings.flow_jitter;
        self.scatter = settings.scatter;
        self.follow_path = settings.follow_path;
        self.tilt_angle = settings.tilt_angle;
        self.pressure = settings.pressure.clone();
    }

    /// The painted material, as a user preset records it.
    pub fn material(&self) -> PresetMaterial {
        PresetMaterial {
            channels: self
                .channels
                .iter()
                .map(|(channel, (checked, value))| (*channel, *checked, *value))
                .collect(),
            mask_value: self.mask_value,
            procedural: self.procedural.clone(),
            mask_procedural: self.mask_procedural.clone(),
            projection: self.projection,
        }
    }

    /// Sets the painted material from `material`.
    pub fn apply_material(&mut self, material: &PresetMaterial) {
        self.channels = material
            .channels
            .iter()
            .map(|(channel, checked, value)| (*channel, (*checked, *value)))
            .collect();
        self.mask_value = material.mask_value;
        self.procedural = material.procedural.clone();
        self.mask_procedural = material.mask_procedural.clone();
        self.projection = material.projection;
    }

    /// Sets the brush from `preset` (and its material, if it has one).
    pub fn apply_preset(&mut self, preset: &BrushPreset, choice: PresetChoice) {
        self.apply_settings(&preset.brush);
        if let Some(material) = &preset.material {
            self.apply_material(material);
        }
        self.preset = Some(choice);
    }

    /// The brush and its material as a user preset called `name`.
    pub fn user_preset(&self, name: &str) -> BrushPreset {
        BrushPreset {
            name: name.to_owned(),
            brush: self.settings(),
            material: Some(self.material()),
        }
    }

    /// The seed of a new stroke: a different one for each stroke, the same sequence in
    /// every session (captures are reproducible).
    pub fn next_seed(&mut self) -> u64 {
        self.strokes_started += 1;
        // SplitMix64 of the stroke count.
        let mut z = self.strokes_started.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Takes the picker; the next pick gives the hand back to the current tool.
    pub fn start_picker(&mut self) {
        if self.tool != Tool::Picker {
            self.previous_tool = self.tool;
            self.tool = Tool::Picker;
        }
    }

    /// Gives the brush the values picked for `target` of `set` (by channel; the mask
    /// value under [`MASK_VALUE_CHANNEL`]): the brush then paints values, not a
    /// procedural. The picker gives the hand back to the previous tool.
    pub fn apply_picked(
        &mut self,
        set: &TextureSet,
        target: PaintTarget,
        values: &[(Channel, [f32; 4])],
    ) {
        match target {
            PaintTarget::Layer(_) => {
                for (channel, value) in values {
                    if let Some(def) = set.channels.iter().find(|def| def.channel == *channel) {
                        let checked = self.channel(def).0;
                        self.channels.insert(*channel, (checked, *value));
                    }
                }
                self.procedural = None;
            }
            PaintTarget::Mask(..) => {
                if let Some((_, value)) = values.iter().find(|(c, _)| *c == MASK_VALUE_CHANNEL) {
                    self.mask_value = value[0].clamp(0.0, 1.0);
                }
                self.mask_procedural = None;
            }
        }
        if self.tool == Tool::Picker {
            self.tool = self.previous_tool;
        }
    }

    /// Switches the mirror symmetry across X on or off.
    pub fn toggle_mirror_x(&mut self) {
        self.symmetry.mirror[0] = !self.symmetry.mirror[0];
    }

    /// Makes the size bigger (`factor` > 1) or smaller, within [`SIZE_RANGE`].
    pub fn scale_size(&mut self, factor: f32) {
        self.size = (self.size * factor).clamp(SIZE_RANGE.0, SIZE_RANGE.1);
    }

    /// Changes the hardness by `delta`, within 0 to 1.
    pub fn add_hardness(&mut self, delta: f32) {
        self.hardness = (self.hardness + delta).clamp(0.0, 1.0);
    }

    /// Paints `channel` with `value` (`Some`), or stops painting it (`None`).
    pub fn set_channel(&mut self, def: &ChannelDef, value: Option<[f32; 4]>) {
        let current = self.channel(def);
        let entry = match value {
            Some(value) => (true, value),
            None => (false, current.1),
        };
        self.channels.insert(def.channel, entry);
    }

    /// The checkbox and value of `def`'s channel.
    pub fn channel(&self, def: &ChannelDef) -> (bool, [f32; 4]) {
        self.channels
            .get(&def.channel)
            .copied()
            .unwrap_or_else(|| initial(def))
    }

    /// Settings of the stroke sampler.
    pub fn sampler(&self) -> SamplerSettings {
        SamplerSettings {
            size: self.size,
            spacing: self.spacing,
            smoothing: self.smoothing,
        }
    }

    /// The brush as recorded by a stroke into `target` of `set`, and the image of its
    /// alpha (stored in the project with the stroke).
    pub fn snapshot(
        &self,
        set: &TextureSet,
        target: PaintTarget,
    ) -> (BrushSnapshot, StrokeTool, Option<(ResourceHash, Resource)>) {
        let material = match (target, &self.procedural, &self.mask_procedural) {
            (PaintTarget::Layer(_), Some(procedural), _)
            | (PaintTarget::Mask(..), _, Some(procedural)) => {
                FillSource::Procedural(procedural.clone())
            }
            (PaintTarget::Layer(_), None, _) => FillSource::Uniform(
                set.channels
                    .iter()
                    .filter_map(|def| {
                        let (checked, value) = self.channel(def);
                        checked.then_some((def.channel, value))
                    })
                    .collect(),
            ),
            (PaintTarget::Mask(..), _, None) => {
                FillSource::Uniform([(MASK_VALUE_CHANNEL, [self.mask_value; 4])].into())
            }
        };
        let alpha = self
            .alpha
            .and_then(find_alpha)
            .map(|alpha| alpha.resource());
        let brush = BrushSnapshot {
            size: self.size,
            size_space: SizeSpace::Screen,
            flow: self.flow,
            opacity: self.opacity,
            hardness: self.hardness,
            spacing: self.spacing,
            angle: self.angle,
            size_jitter: self.size_jitter,
            angle_jitter: self.angle_jitter,
            flow_jitter: self.flow_jitter,
            scatter: self.scatter,
            follow_path: self.follow_path,
            alpha: alpha.as_ref().map(|(hash, _)| *hash),
            material,
            pressure: self.pressure.clone(),
            tilt_angle: self.tilt_angle,
            projection: self.projection,
        };
        let tool = match self.tool {
            Tool::Brush | Tool::Picker => StrokeTool::Brush,
            Tool::Eraser => StrokeTool::Eraser,
        };
        (brush, tool, alpha)
    }
}

/// What the selection lets the brush paint: the selected paint layer, or the selected
/// paint effect of a mask.
pub fn paint_target(
    set: &TextureSet,
    selected: Option<LayerPart>,
    layer: Option<patine_core::ids::LayerId>,
) -> Option<PaintTarget> {
    let layer = set.stack.find(layer?)?;
    match selected? {
        LayerPart::Content => {
            matches!(layer.kind, LayerKind::Paint { .. }).then_some(PaintTarget::Layer(layer.id))
        }
        LayerPart::MaskEffect(effect) => {
            let mask = layer.mask.as_ref()?;
            let entry = &mask.effects[mask.index_of(effect)?];
            matches!(entry.effect, MaskEffect::Paint { .. })
                .then_some(PaintTarget::Mask(layer.id, effect))
        }
        LayerPart::Mask => None,
    }
}

/// The stroke log of `target`.
pub fn stroke_log(set: &TextureSet, target: PaintTarget) -> Option<&StrokeLog> {
    let layer = set.stack.find(target.layer())?;
    match target {
        PaintTarget::Layer(_) => match &layer.kind {
            LayerKind::Paint { strokes } => Some(strokes),
            _ => None,
        },
        PaintTarget::Mask(_, effect) => {
            let mask = layer.mask.as_ref()?;
            match &mask.effects[mask.index_of(effect)?].effect {
                MaskEffect::Paint { strokes } => Some(strokes),
                _ => None,
            }
        }
    }
}

/// The brush section of the Properties panel, for `target` in `set`. Returns whether a
/// slider gesture ended.
pub fn brush_section(
    ui: &mut Ui,
    i18n: &I18n,
    state: &mut ToolState,
    set: &TextureSet,
    target: PaintTarget,
) -> bool {
    let mut ended = false;
    ui.horizontal(|ui| {
        for (tool, icon, key) in [
            (Tool::Brush, icons::PAINT_BRUSH, "tool-brush"),
            (Tool::Eraser, icons::ERASER, "tool-eraser"),
            (Tool::Picker, icons::EYEDROPPER, "tool-picker"),
        ] {
            let label = format!("{icon} {}", i18n.tr(key));
            if ui.selectable_label(state.tool == tool, label).clicked() {
                match tool {
                    Tool::Picker => state.start_picker(),
                    _ => state.tool = tool,
                }
            }
        }
    });
    ui.label(RichText::new(i18n.tr("paint-line-hint")).small().weak());
    if let Some(log) = stroke_log(set, target) {
        let mut args = FluentArgs::new();
        args.set("count", log.strokes.len());
        ui.label(
            RichText::new(i18n.tr_args("brush-strokes", &args))
                .small()
                .weak(),
        );
    }
    presets_row(ui, i18n, state);
    ui.separator();
    let mut slider = |ui: &mut Ui,
                      value: &mut f32,
                      range: std::ops::RangeInclusive<f32>,
                      key: &str,
                      logarithmic: bool| {
        let response = ui.add(
            egui::Slider::new(value, range)
                .logarithmic(logarithmic)
                .text(i18n.tr(key)),
        );
        ended |= gesture_ended(&response);
    };
    slider(
        ui,
        &mut state.size,
        SIZE_RANGE.0..=SIZE_RANGE.1,
        "brush-size",
        true,
    );
    slider(ui, &mut state.flow, 0.0..=1.0, "brush-flow", false);
    slider(ui, &mut state.opacity, 0.0..=1.0, "brush-opacity", false);
    slider(ui, &mut state.hardness, 0.0..=1.0, "brush-hardness", false);
    slider(ui, &mut state.spacing, 0.02..=2.0, "brush-spacing", true);
    let mut degrees = state.angle.to_degrees();
    slider(ui, &mut degrees, -180.0..=180.0, "brush-angle", false);
    state.angle = degrees.to_radians();
    slider(
        ui,
        &mut state.smoothing,
        0.0..=100.0,
        "brush-smoothing",
        false,
    );

    ui.label(RichText::new(i18n.tr("brush-alpha")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut state.alpha, None, i18n.tr("alpha-round"));
        for alpha in &ALPHAS {
            ui.selectable_value(&mut state.alpha, Some(alpha.id), i18n.tr(alpha.name_key));
        }
    });
    ui.separator();
    ended |= dynamics_section(ui, i18n, state);
    ended |= symmetry_section(ui, i18n, &mut state.symmetry, state.expand_sections);
    ui.separator();

    // The material: values, or a procedural.
    let (procedural, categories, values_key, procedural_key) = match target {
        PaintTarget::Layer(_) => (
            &mut state.procedural,
            &[Category::Material][..],
            "brush-material-values",
            "brush-material-procedural",
        ),
        PaintTarget::Mask(..) => (
            &mut state.mask_procedural,
            &[Category::Noise, Category::Pattern][..],
            "brush-mask-value",
            "brush-mask-pattern",
        ),
    };
    ui.label(RichText::new(i18n.tr("brush-material")).strong());
    ui.horizontal(|ui| {
        if ui
            .selectable_label(procedural.is_none(), i18n.tr(values_key))
            .clicked()
        {
            *procedural = None;
        }
        if ui
            .selectable_label(procedural.is_some(), i18n.tr(procedural_key))
            .clicked()
            && procedural.is_none()
        {
            *procedural = patine_procedural::library::all()
                .iter()
                .find(|def| categories.contains(&def.category))
                .map(|def| def.instance());
        }
    });
    if let Some(instance) = procedural {
        ui.push_id("brush procedural", |ui| {
            ended |= procedural_editor(ui, i18n, instance, &mut state.projection, categories);
        });
        return ended;
    }

    match target {
        PaintTarget::Mask(..) => {
            let response = ui.add(
                egui::Slider::new(&mut state.mask_value, 0.0..=1.0).text(i18n.tr("mask-value")),
            );
            ended |= gesture_ended(&response);
        }
        PaintTarget::Layer(_) => {
            ui.label(RichText::new(i18n.tr("brush-channels")).strong());
            for def in &set.channels {
                let (mut checked, mut value) = state.channel(def);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut checked, channel_name(i18n, def.channel));
                    match def.channel {
                        Channel::BaseColor | Channel::Emissive => {
                            let mut rgb = [value[0], value[1], value[2]];
                            if ui.color_edit_button_rgb(&mut rgb).changed() {
                                value = [rgb[0], rgb[1], rgb[2], value[3]];
                            }
                        }
                        Channel::Normal => {
                            ui.label(RichText::new(i18n.tr("brush-normal-flat")).small().weak());
                        }
                        channel => {
                            let range = if channel == Channel::Height {
                                -1.0..=1.0
                            } else {
                                0.0..=1.0
                            };
                            let response = ui.add(egui::Slider::new(&mut value[0], range));
                            ended |= gesture_ended(&response);
                            value = [value[0]; 4];
                        }
                    }
                });
                state.channels.insert(def.channel, (checked, value));
            }
        }
    }
    ended
}

/// The preset list, and saving or deleting the user's presets.
fn presets_row(ui: &mut Ui, i18n: &I18n, state: &mut ToolState) {
    let shipped = shipped();
    let selected_text = match &state.preset {
        Some(PresetChoice::Shipped(key)) => i18n.tr(key).into_owned(),
        Some(PresetChoice::User(name)) => name.clone(),
        None => i18n.tr("brush-preset-none").into_owned(),
    };
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.label(i18n.tr("brush-preset"));
        egui::ComboBox::from_id_salt("brush preset")
            .selected_text(selected_text)
            .show_ui(ui, |ui| {
                for (preset, key) in shipped.iter().zip(SHIPPED_KEYS) {
                    let choice = PresetChoice::Shipped(key);
                    if ui
                        .selectable_label(state.preset.as_ref() == Some(&choice), i18n.tr(key))
                        .clicked()
                    {
                        chosen = Some((preset.clone(), choice));
                    }
                }
                if !state.presets.user.is_empty() {
                    ui.separator();
                }
                for preset in &state.presets.user {
                    let choice = PresetChoice::User(preset.name.clone());
                    if ui
                        .selectable_label(state.preset.as_ref() == Some(&choice), &preset.name)
                        .clicked()
                    {
                        chosen = Some((preset.clone(), choice));
                    }
                }
            });
    });
    if let Some((preset, choice)) = chosen {
        state.apply_preset(&preset, choice);
    }

    let name_id = ui.id().with("brush preset name");
    let error_id = ui.id().with("brush preset error");
    let mut name: String = ui.data(|d| d.get_temp(name_id)).unwrap_or_default();
    let mut error: Option<String> = ui.data(|d| d.get_temp(error_id)).flatten();
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut name)
                .hint_text(i18n.tr("brush-preset-name"))
                .desired_width(110.0),
        );
        let can_save = state.presets.dir.is_some() && file_name(&name).is_some();
        if ui
            .add_enabled(can_save, egui::Button::new(i18n.tr("brush-preset-save")))
            .clicked()
        {
            let name_ = name.trim().to_owned();
            match state.presets.save(state.user_preset(&name_)) {
                Ok(()) => {
                    state.preset = Some(PresetChoice::User(name_));
                    name.clear();
                    error = None;
                }
                Err(e) => {
                    let mut args = FluentArgs::new();
                    args.set("error", e.to_string());
                    error = Some(i18n.tr_args("brush-preset-error", &args));
                }
            }
        }
        if let Some(PresetChoice::User(user)) = state.preset.clone()
            && ui
                .button(format!(
                    "{} {}",
                    icons::TRASH,
                    i18n.tr("brush-preset-delete")
                ))
                .clicked()
        {
            match state.presets.delete(&user) {
                Ok(()) => state.preset = None,
                Err(e) => error = Some(e.to_string()),
            }
        }
    });
    if let Some(text) = &error {
        ui.colored_label(ui.visuals().error_fg_color, text);
    }
    ui.data_mut(|d| {
        d.insert_temp(name_id, name);
        d.insert_temp(error_id, error);
    });
}

/// A slider labelled with the translation of `key`; returns whether its gesture ended.
fn slider(
    ui: &mut Ui,
    i18n: &I18n,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    key: &str,
) -> bool {
    let response = ui.add(egui::Slider::new(value, range).text(i18n.tr(key)));
    gesture_ended(&response)
}

/// Pressure curves, by the translation key of what they change.
const PRESSURE_CURVES: [&str; 3] = ["pressure-size", "pressure-flow", "pressure-opacity"];

/// Jitters, scatter, path and tilt, and the pressure curves. Returns whether a gesture
/// ended.
fn dynamics_section(ui: &mut Ui, i18n: &I18n, state: &mut ToolState) -> bool {
    let mut ended = false;
    egui::CollapsingHeader::new(i18n.tr("brush-dynamics"))
        .id_salt("brush dynamics")
        .default_open(state.expand_sections)
        .show(ui, |ui| {
            ended |= slider(
                ui,
                i18n,
                &mut state.size_jitter,
                0.0..=1.0,
                "brush-size-jitter",
            );
            let mut degrees = state.angle_jitter.to_degrees();
            ended |= slider(ui, i18n, &mut degrees, 0.0..=180.0, "brush-angle-jitter");
            state.angle_jitter = degrees.to_radians();
            ended |= slider(
                ui,
                i18n,
                &mut state.flow_jitter,
                0.0..=1.0,
                "brush-flow-jitter",
            );
            ended |= slider(ui, i18n, &mut state.scatter, 0.0..=3.0, "brush-scatter");
            ended |= ui
                .checkbox(&mut state.follow_path, i18n.tr("brush-follow-path"))
                .changed();
            ended |= ui
                .checkbox(&mut state.tilt_angle, i18n.tr("brush-tilt-angle"))
                .changed();

            ui.label(RichText::new(i18n.tr("brush-pressure")).strong());
            let which_id = ui.id().with("pressure curve");
            let mut which: usize = ui.data(|d| d.get_temp(which_id)).unwrap_or(0);
            ui.horizontal(|ui| {
                for (i, key) in PRESSURE_CURVES.iter().enumerate() {
                    ui.selectable_value(&mut which, i, i18n.tr(key));
                }
            });
            ui.data_mut(|d| d.insert_temp(which_id, which));
            let curve = match which {
                0 => &mut state.pressure.size,
                1 => &mut state.pressure.flow,
                _ => &mut state.pressure.opacity,
            };
            ui.horizontal(|ui| {
                let (_, curve_ended) =
                    crate::curve_editor::curve_editor(ui, which_id.with(which), curve, 120.0);
                ended |= curve_ended;
                ui.vertical(|ui| {
                    if ui.button(i18n.tr("pressure-curve-reset")).clicked() {
                        *curve = Curve::identity();
                        ended = true;
                    }
                    if ui.button(i18n.tr("pressure-curve-off")).clicked() {
                        *curve = Curve::constant(1.0);
                        ended = true;
                    }
                });
            });
            ui.label(RichText::new(i18n.tr("pressure-curve-hint")).small().weak());
        });
    ended
}

/// Mirrors and radial copies. Returns whether a setting changed.
fn symmetry_section(ui: &mut Ui, i18n: &I18n, symmetry: &mut Symmetry, open: bool) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new(i18n.tr("brush-symmetry"))
        .id_salt("brush symmetry")
        .default_open(open)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(i18n.tr("symmetry-mirror"));
                for (axis, name) in ["X", "Y", "Z"].iter().enumerate() {
                    changed |= ui.checkbox(&mut symmetry.mirror[axis], *name).changed();
                }
            });
            ui.horizontal(|ui| {
                let mut radial = symmetry.radial.max(1);
                changed |= ui
                    .add(egui::DragValue::new(&mut radial).range(1..=16))
                    .changed();
                symmetry.radial = radial;
                ui.label(i18n.tr("symmetry-radial"));
            });
            let mirrors = 1usize << symmetry.mirror.iter().filter(|m| **m).count();
            let wanted = mirrors * symmetry.radial.max(1) as usize;
            if wanted > patine_paint::symmetry::MAX_COPIES {
                let mut args = FluentArgs::new();
                args.set("max", patine_paint::symmetry::MAX_COPIES);
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    i18n.tr_args("symmetry-limit", &args),
                );
            }
            ui.label(RichText::new(i18n.tr("symmetry-2d")).small().weak());
        });
    changed
}

/// The choice of a procedural among `categories`, its parameters and the projection.
/// Returns whether a gesture ended.
fn procedural_editor(
    ui: &mut Ui,
    i18n: &I18n,
    instance: &mut ProceduralRef,
    projection: &mut Projection,
    categories: &[Category],
) -> bool {
    let mut ended = false;
    let current = patine_procedural::library::find(&instance.id);
    egui::ComboBox::from_id_salt("brush procedural choice")
        .selected_text(current.map_or_else(
            || instance.id.clone(),
            |def| i18n.tr(&def.name_key).into_owned(),
        ))
        .show_ui(ui, |ui| {
            for def in patine_procedural::library::all()
                .iter()
                .filter(|def| categories.contains(&def.category))
            {
                if ui
                    .selectable_label(def.id == instance.id, i18n.tr(&def.name_key))
                    .clicked()
                    && def.id != instance.id
                {
                    *instance = def.instance();
                    ended = true;
                }
            }
        });
    if let Some(def) = patine_procedural::library::find(&instance.id) {
        let edited = crate::procedural_ui::params_editor(ui, i18n, def, instance);
        if let Some(value) = edited.value {
            *instance = value;
        }
        ended |= edited.ended;
    }
    let edited = crate::procedural_ui::projection_editor(ui, i18n, projection);
    if let Some(value) = edited.value {
        *projection = value;
    }
    ended | edited.ended
}

/// The material and the UV of point `position` of triangle `triangle` of `mesh`, by
/// barycentric interpolation (clamped to the triangle). `None` without UVs or for a
/// degenerate triangle.
pub fn surface_uv(
    mesh: &patine_core::mesh::MeshAsset,
    triangle: u32,
    position: Vec3,
) -> Option<(u32, [f32; 2])> {
    let corners = mesh.triangles.get(triangle as usize)?;
    let uvs = mesh.uv0.as_ref()?;
    let [a, b, c] = corners.map(|v| Vec3::from(mesh.positions[v as usize]));
    let (e0, e1, p) = (b - a, c - a, position - a);
    let (d00, d01, d11) = (e0.dot(e0), e0.dot(e1), e1.dot(e1));
    let (d20, d21) = (p.dot(e0), p.dot(e1));
    let denominator = d00 * d11 - d01 * d01;
    if denominator.abs() <= f32::EPSILON * d00 * d11 {
        return None;
    }
    let v = ((d11 * d20 - d01 * d21) / denominator).max(0.0);
    let w = ((d00 * d21 - d01 * d20) / denominator).max(0.0);
    let u = (1.0 - v - w).max(0.0);
    let sum = u + v + w;
    let [ua, ub, uc] = corners.map(|i| Vec2::from(uvs[i as usize]));
    let uv = (ua * u + ub * v + uc * w) / sum;
    let material = mesh
        .submeshes
        .iter()
        .find(|submesh| submesh.triangles.contains(&triangle))?
        .material;
    Some((material, uv.to_array()))
}

/// The camera of a stroke painted in the 2D view: it maps UVs `(u, v, 0, 1)` to the
/// pixels of a `size` view showing them with `transform`.
pub fn uv_camera(
    transform: &patine_render::uv_view::UvTransform,
    size: [u32; 2],
) -> patine_core::stroke::StrokeCamera {
    let (w, h) = (size[0].max(1) as f32, size[1].max(1) as f32);
    let (z, [cx, cy]) = (transform.zoom, transform.center);
    let view_proj = Mat4::from_cols(
        glam::Vec4::new(2.0 * z / w, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, -2.0 * z / h, 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, 1.0, 0.0),
        glam::Vec4::new(-2.0 * z * cx / w, 2.0 * z * cy / h, 0.0, 1.0),
    );
    patine_core::stroke::StrokeCamera {
        view_proj: view_proj.to_cols_array_2d(),
        viewport: size,
    }
}

/// The texel of a `size`² texture under `uv` (clamped to the texture).
pub fn texel_of(uv: [f32; 2], size: u32) -> [u32; 2] {
    let last = size.saturating_sub(1) as f32;
    uv.map(|t| (t * size as f32).floor().clamp(0.0, last) as u32)
}

/// A point of the mesh under the cursor, for the brush cursor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfacePoint {
    /// World position.
    pub position: Vec3,
    /// Unit normal of the triangle.
    pub normal: Vec3,
}

/// Points of the brush cursor in viewport pixels: a circle on the surface at `point`,
/// around its normal, whose radius in the surface plane is the brush radius `radius`
/// (pixels) at that depth. Empty when the point is behind the camera.
pub fn cursor_outline(
    view_proj: Mat4,
    viewport: [u32; 2],
    point: SurfacePoint,
    radius: f32,
    camera_right: Vec3,
) -> Vec<Vec2> {
    let size = Vec2::new(viewport[0] as f32, viewport[1] as f32);
    let pixel = |p: Vec3| -> Option<Vec2> {
        let clip = view_proj * p.extend(1.0);
        (clip.w > 1e-6).then(|| {
            let ndc = Vec2::new(clip.x, clip.y) / clip.w;
            Vec2::new((ndc.x * 0.5 + 0.5) * size.x, (0.5 - ndc.y * 0.5) * size.y)
        })
    };
    let Some(center) = pixel(point.position) else {
        return Vec::new();
    };
    // World size of a pixel at the point's depth, measured along the screen.
    let probe = point.position + camera_right * 1e-3;
    let Some(moved) = pixel(probe) else {
        return Vec::new();
    };
    let pixels_per_unit = (moved - center).length() / 1e-3;
    if pixels_per_unit <= 1e-6 {
        return Vec::new();
    }
    let world_radius = radius / pixels_per_unit;
    let n = point.normal.normalize_or(Vec3::Y);
    let u = n.any_orthonormal_vector();
    let v = n.cross(u);
    (0..=48)
        .filter_map(|i| {
            let a = i as f32 / 48.0 * std::f32::consts::TAU;
            pixel(point.position + (u * a.cos() + v * a.sin()) * world_radius)
        })
        .collect()
}

/// Draws the brush cursor `outline` (viewport pixels) over the viewport at `rect`.
pub fn draw_cursor(
    painter: &egui::Painter,
    rect: egui::Rect,
    pixels_per_point: f32,
    outline: &[Vec2],
    paintable: bool,
) {
    let points: Vec<egui::Pos2> = outline
        .iter()
        .map(|p| rect.min + egui::vec2(p.x, p.y) / pixels_per_point)
        .collect();
    let color = if paintable {
        Color32::WHITE
    } else {
        Color32::from_gray(120)
    };
    painter.add(egui::Shape::line(
        points.clone(),
        Line::new(2.5, Color32::from_black_alpha(140)),
    ));
    painter.add(egui::Shape::line(points, Line::new(1.2, color)));
}

/// Translation keys of the brush's dynamics, presets and symmetry, for tests.
pub const DYNAMICS_TRANSLATION_KEYS: [&str; 33] = [
    "brush-preset",
    "brush-preset-none",
    "brush-preset-name",
    "brush-preset-save",
    "brush-preset-delete",
    "brush-preset-error",
    "brush-dynamics",
    "brush-size-jitter",
    "brush-angle-jitter",
    "brush-flow-jitter",
    "brush-scatter",
    "brush-follow-path",
    "brush-tilt-angle",
    "brush-pressure",
    "pressure-size",
    "pressure-flow",
    "pressure-opacity",
    "pressure-curve-reset",
    "pressure-curve-off",
    "pressure-curve-hint",
    "brush-symmetry",
    "symmetry-mirror",
    "symmetry-radial",
    "symmetry-limit",
    "symmetry-2d",
    "brush-material",
    "brush-material-values",
    "brush-material-procedural",
    "brush-mask-pattern",
    "tool-picker",
    "picker-nothing",
    "paint-line-hint",
    "texture-sets-height-force",
];

/// Every translation key of painting, for tests.
pub const TRANSLATION_KEYS: [&str; 23] = [
    "menu-paint",
    "tool-brush",
    "tool-eraser",
    "brush-strokes",
    "brush-size",
    "brush-flow",
    "brush-opacity",
    "brush-hardness",
    "brush-spacing",
    "brush-angle",
    "brush-smoothing",
    "brush-alpha",
    "alpha-round",
    "alpha-square",
    "alpha-splatter",
    "alpha-grain",
    "alpha-chisel",
    "brush-mask-value",
    "brush-channels",
    "brush-normal-flat",
    "paint-not-paintable",
    "paint-no-channel",
    "paint-heading",
];

#[cfg(test)]
mod tests {
    use patine_core::ids::{EffectId, LayerId};
    use patine_core::layer::Layer;
    use patine_core::mask::{MaskEffectEntry, MaskInitial, MaskStack};
    use patine_core::project::{Project, ProjectSettings};

    use super::*;
    use crate::i18n::Language;

    #[test]
    fn texts_are_translated() {
        for language in Language::ALL {
            let i18n = I18n::new(language);
            for key in TRANSLATION_KEYS.iter().chain(&DYNAMICS_TRANSLATION_KEYS) {
                assert!(i18n.has(key), "{language:?}: {key}");
            }
        }
    }

    fn project() -> Project {
        let mut project = Project::new(
            patine_mesh::primitives::cube_cross(),
            "cube.glb".into(),
            ProjectSettings::default(),
        );
        let channels: Vec<Channel> = project.texture_sets[0]
            .channels
            .iter()
            .map(|d| d.channel)
            .collect();
        let paint = Layer::paint(LayerId(1), "paint", &channels);
        let mut fill = Layer::fill_for(LayerId(2), "fill", &project.texture_sets[0].channels);
        let mut mask = MaskStack::new(MaskInitial::Black);
        mask.effects
            .push(MaskEffectEntry::new(EffectId(3), MaskEffect::paint()));
        mask.effects
            .push(MaskEffectEntry::new(EffectId(4), MaskEffect::invert()));
        fill.mask = Some(mask);
        project.texture_sets[0].stack.layers = vec![paint, fill];
        project.next_id = 5;
        project
    }

    #[test]
    fn only_paint_layers_and_paint_effects_are_painted() {
        let project = project();
        let set = &project.texture_sets[0];
        let target = |layer: u64, part| paint_target(set, Some(part), Some(LayerId(layer)));
        assert_eq!(
            target(1, LayerPart::Content),
            Some(PaintTarget::Layer(LayerId(1)))
        );
        assert_eq!(target(2, LayerPart::Content), None, "a fill");
        assert_eq!(target(2, LayerPart::Mask), None);
        assert_eq!(
            target(2, LayerPart::MaskEffect(EffectId(3))),
            Some(PaintTarget::Mask(LayerId(2), EffectId(3)))
        );
        assert_eq!(
            target(2, LayerPart::MaskEffect(EffectId(4))),
            None,
            "an inversion"
        );
        assert_eq!(target(9, LayerPart::Content), None);
        assert_eq!(paint_target(set, Some(LayerPart::Content), None), None);
        assert_eq!(
            stroke_log(set, PaintTarget::Layer(LayerId(1))).map(|l| l.strokes.len()),
            Some(0)
        );
    }

    #[test]
    fn snapshots_record_the_checked_channels_and_the_alpha() {
        let project = project();
        let set = &project.texture_sets[0];
        let mut state = ToolState::default();
        let (brush, tool, alpha) = state.snapshot(set, PaintTarget::Layer(LayerId(1)));
        let FillSource::Uniform(values) = &brush.material else {
            panic!("uniform");
        };
        assert_eq!(
            values.keys().copied().collect::<Vec<_>>(),
            [Channel::BaseColor, Channel::Roughness]
        );
        assert_eq!(tool, StrokeTool::Brush);
        assert!(alpha.is_none() && brush.alpha.is_none());

        state.tool = Tool::Eraser;
        state.alpha = Some("alpha.grain");
        state.mask_value = 0.3;
        let (brush, tool, alpha) = state.snapshot(set, PaintTarget::Mask(LayerId(2), EffectId(3)));
        assert_eq!(tool, StrokeTool::Eraser);
        let (hash, resource) = alpha.expect("the alpha's image");
        assert_eq!(brush.alpha, Some(hash));
        assert_eq!(ResourceHash::of(&resource.bytes), hash);
        let FillSource::Uniform(values) = &brush.material else {
            panic!("uniform");
        };
        assert_eq!(values.get(&MASK_VALUE_CHANNEL), Some(&[0.3; 4]));

        state.scale_size(1000.0);
        assert_eq!(state.size, SIZE_RANGE.1);
        state.add_hardness(-5.0);
        assert_eq!(state.hardness, 0.0);
    }

    #[test]
    fn snapshots_record_the_dynamics_and_the_procedural_material() {
        let project = project();
        let set = &project.texture_sets[0];
        let mut state = ToolState {
            size_jitter: 0.3,
            angle_jitter: 1.0,
            flow_jitter: 0.2,
            scatter: 0.5,
            follow_path: true,
            tilt_angle: true,
            ..ToolState::default()
        };
        state.pressure.opacity = Curve::identity();
        let copper = patine_procedural::library::find("material.copper")
            .expect("a shipped material")
            .instance();
        state.procedural = Some(copper.clone());
        let (brush, _, _) = state.snapshot(set, PaintTarget::Layer(LayerId(1)));
        assert_eq!(brush.size_jitter, 0.3);
        assert_eq!(brush.angle_jitter, 1.0);
        assert_eq!(brush.flow_jitter, 0.2);
        assert_eq!(brush.scatter, 0.5);
        assert!(brush.follow_path && brush.tilt_angle);
        assert_eq!(brush.pressure.opacity, Curve::identity());
        assert_eq!(brush.material, FillSource::Procedural(copper));
        assert_eq!(brush.projection, state.projection);
        // The layer's procedural is not painted into masks.
        let (brush, _, _) = state.snapshot(set, PaintTarget::Mask(LayerId(2), EffectId(3)));
        assert!(matches!(brush.material, FillSource::Uniform(_)));
    }

    #[test]
    fn presets_set_the_brush_and_user_presets_the_material() {
        let mut state = ToolState::default();
        let chalk = &shipped()[3];
        state
            .channels
            .insert(Channel::BaseColor, (true, [0.0, 1.0, 0.0, 1.0]));
        let before = state.material();
        state.apply_preset(chalk, PresetChoice::Shipped(SHIPPED_KEYS[3]));
        assert_eq!(state.settings(), chalk.brush);
        assert_eq!(state.alpha, Some("alpha.grain"));
        assert_eq!(
            state.material(),
            before,
            "a shipped preset keeps the material"
        );

        let saved = state.user_preset("mine");
        let mut other = ToolState::default();
        other.apply_preset(&saved, PresetChoice::User("mine".into()));
        assert_eq!(other.settings(), state.settings());
        assert_eq!(other.material(), state.material());
        assert_eq!(other.preset, Some(PresetChoice::User("mine".into())));
        // The default brush is the Standard preset.
        assert_eq!(ToolState::default().settings(), shipped()[0].brush);
    }

    #[test]
    fn each_stroke_gets_a_new_seed_and_l_mirrors_x() {
        let mut a = ToolState::default();
        let mut b = ToolState::default();
        let seeds: Vec<u64> = (0..4).map(|_| a.next_seed()).collect();
        assert_eq!(seeds, (0..4).map(|_| b.next_seed()).collect::<Vec<_>>());
        for pair in seeds.windows(2) {
            assert_ne!(pair[0], pair[1]);
        }
        a.toggle_mirror_x();
        assert_eq!(a.symmetry.mirror, [true, false, false]);
        a.toggle_mirror_x();
        assert_eq!(a.symmetry, Symmetry::default());
    }

    #[test]
    fn the_picker_fills_the_brush_and_gives_the_hand_back() {
        let project = project();
        let set = &project.texture_sets[0];
        let mut state = ToolState {
            tool: Tool::Eraser,
            procedural: patine_procedural::library::find("material.copper")
                .map(|def| def.instance()),
            ..ToolState::default()
        };
        state.start_picker();
        state.start_picker();
        assert_eq!(state.tool, Tool::Picker);
        state.apply_picked(
            set,
            PaintTarget::Layer(LayerId(1)),
            &[
                (Channel::BaseColor, [0.1, 0.2, 0.3, 1.0]),
                (Channel::Metallic, [0.7; 4]),
                (Channel::User(9), [0.5; 4]),
            ],
        );
        assert_eq!(state.tool, Tool::Eraser);
        assert_eq!(state.procedural, None);
        let def = |channel| {
            set.channels
                .iter()
                .find(|d| d.channel == channel)
                .expect("a channel of the set")
        };
        // Values change; what is painted does not.
        assert_eq!(
            state.channel(def(Channel::BaseColor)),
            (true, [0.1, 0.2, 0.3, 1.0])
        );
        assert_eq!(state.channel(def(Channel::Metallic)), (false, [0.7; 4]));
        state.start_picker();
        state.apply_picked(
            set,
            PaintTarget::Mask(LayerId(2), EffectId(3)),
            &[(MASK_VALUE_CHANNEL, [1.4; 4])],
        );
        assert_eq!(state.mask_value, 1.0);
    }

    #[test]
    fn the_uv_camera_puts_uvs_where_the_2d_view_shows_them() {
        let transform = patine_render::uv_view::UvTransform {
            center: [0.3, 0.6],
            zoom: 700.0,
        };
        let size = [900, 500];
        let camera = uv_camera(&transform, size);
        let projection = patine_paint::projection::StrokeProjection::new(&camera);
        for uv in [[0.0, 0.0], [0.3, 0.6], [1.0, 0.25], [0.5, 1.0]] {
            let pixel = projection
                .pixel(projection.clip(Vec3::new(uv[0], uv[1], 0.0)))
                .expect("in front");
            let expected = Vec2::from(transform.pixel_of(uv, size));
            assert!(
                pixel.distance(expected) < 1e-3,
                "{uv:?}: {pixel} vs {expected}"
            );
        }
    }

    #[test]
    fn surface_points_give_their_uv_and_texel() {
        let mesh = patine_mesh::primitives::cube_cross();
        let uvs = mesh.uv0.as_ref().expect("the cube has UVs");
        for triangle in [0u32, 5, 11] {
            let corners = mesh.triangles[triangle as usize];
            let [a, b, c] = corners.map(|v| Vec3::from(mesh.positions[v as usize]));
            let [ua, ub, uc] = corners.map(|v| Vec2::from(uvs[v as usize]));
            let weights = [0.2, 0.5, 0.3];
            let position = a * weights[0] + b * weights[1] + c * weights[2];
            let expected = ua * weights[0] + ub * weights[1] + uc * weights[2];
            let (material, uv) = surface_uv(&mesh, triangle, position).expect("a point");
            assert_eq!(material, 0);
            assert!(Vec2::from(uv).distance(expected) < 1e-5, "{triangle}");
        }
        assert_eq!(surface_uv(&mesh, 9999, Vec3::ZERO), None);
        assert_eq!(texel_of([0.0, 0.0], 512), [0, 0]);
        assert_eq!(texel_of([0.5, 0.999], 512), [256, 511]);
        assert_eq!(texel_of([1.0, -0.2], 512), [511, 0]);
    }

    #[test]
    fn the_cursor_has_the_brush_radius_on_a_facing_surface() {
        // Orthographic-like camera looking down -Z: one world unit is 100 pixels.
        let view_proj = Mat4::from_cols_array(&[
            0.5, 0.0, 0.0, 0.0, //
            0.0, 0.5, 0.0, 0.0, //
            0.0, 0.0, -0.1, 0.0, //
            0.0, 0.0, 0.5, 1.0,
        ]);
        let point = SurfacePoint {
            position: Vec3::ZERO,
            normal: Vec3::Z,
        };
        let outline = cursor_outline(view_proj, [400, 400], point, 20.0, Vec3::X);
        assert_eq!(outline.len(), 49);
        for p in &outline {
            assert!(
                (p.distance(Vec2::new(200.0, 200.0)) - 20.0).abs() < 0.01,
                "{p}"
            );
        }
        // Seen edge-on, the circle flattens into a line.
        let edge = SurfacePoint {
            normal: Vec3::X,
            ..point
        };
        let outline = cursor_outline(view_proj, [400, 400], edge, 20.0, Vec3::X);
        assert!(outline.iter().all(|p| (p.x - 200.0).abs() < 0.01));
    }
}
