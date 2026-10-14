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
use patine_core::layer::{FillSource, LayerKind};
use patine_core::mask::{MASK_VALUE_CHANNEL, MaskEffect};
use patine_core::project::TextureSet;
use patine_core::resource::{Resource, ResourceHash};
use patine_core::stroke::{BrushSnapshot, SizeSpace, StrokeLog, Tool as StrokeTool};
use patine_paint::alphas::{ALPHAS, find as find_alpha};
use patine_paint::sampler::SamplerSettings;

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
}

/// Smallest and largest brush sizes, in viewport pixels.
pub const SIZE_RANGE: (f32, f32) = (1.0, 1000.0);

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
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            tool: Tool::Brush,
            size: 40.0,
            flow: 0.8,
            opacity: 1.0,
            hardness: 0.6,
            spacing: 0.1,
            angle: 0.0,
            smoothing: 0.0,
            alpha: None,
            channels: BTreeMap::new(),
            mask_value: 1.0,
        }
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
        let material = match target {
            PaintTarget::Layer(_) => set
                .channels
                .iter()
                .filter_map(|def| {
                    let (checked, value) = self.channel(def);
                    checked.then_some((def.channel, value))
                })
                .collect(),
            PaintTarget::Mask(..) => [(MASK_VALUE_CHANNEL, [self.mask_value; 4])].into(),
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
            size_jitter: 0.0,
            angle_jitter: 0.0,
            flow_jitter: 0.0,
            scatter: 0.0,
            follow_path: false,
            alpha: alpha.as_ref().map(|(hash, _)| *hash),
            material: FillSource::Uniform(material),
        };
        let tool = match self.tool {
            Tool::Brush => StrokeTool::Brush,
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
        ] {
            ui.selectable_value(&mut state.tool, tool, format!("{icon} {}", i18n.tr(key)));
        }
    });
    if let Some(log) = stroke_log(set, target) {
        let mut args = FluentArgs::new();
        args.set("count", log.strokes.len());
        ui.label(
            RichText::new(i18n.tr_args("brush-strokes", &args))
                .small()
                .weak(),
        );
    }
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

    match target {
        PaintTarget::Mask(..) => {
            ui.label(RichText::new(i18n.tr("brush-mask-value")).strong());
            slider(ui, &mut state.mask_value, 0.0..=1.0, "mask-value", false);
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
            for key in TRANSLATION_KEYS {
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
