//! 2D view panel: the current texture set in UV space (image rendered by
//! `patine_render::uv_view`), with pan and zoom. The brush paints there too, in UV
//! space.

use egui::{PointerButton, RichText, Sense, Ui};
use patine_render::uv_view::UvTransform;

use crate::i18n::I18n;
use crate::tablet::{PaintSource, PenEvent};
use crate::viewport_panel::{PaintEvent, view_paint_events};

/// Zoom factor exponent per scrolled point.
const WHEEL_ZOOM_SPEED: f32 = 0.0015;

/// State of the 2D view.
#[derive(Debug)]
pub struct View2dState {
    /// Mapping between UVs and pixels.
    pub transform: UvTransform,
    /// Size of the panel in physical pixels, measured during the last layout.
    pub size: [u32; 2],
    /// egui texture showing the rendered image, registered by the application.
    pub texture: Option<egui::TextureId>,
    /// Whether the panel was drawn this frame (it is not rendered otherwise).
    pub visible: bool,
    /// Whether the cursor is over the panel (F frames the UVs instead of the mesh).
    pub hovered: bool,
    /// Whether the transform was fitted to the panel since the document was opened.
    pub framed: bool,
    /// Whether the left button or the pen paints.
    pub painting: bool,
    /// Device of the stroke in progress.
    pub paint_source: Option<PaintSource>,
    /// Painting gestures not handled yet by the application, in the panel's pixels.
    pub paint_events: Vec<PaintEvent>,
    /// Brush cursor: radius in pixels, and whether the selection can be painted.
    pub cursor: Option<(f32, bool)>,
}

impl Default for View2dState {
    fn default() -> Self {
        Self {
            transform: UvTransform::default(),
            size: [1, 1],
            texture: None,
            visible: false,
            hovered: false,
            framed: false,
            painting: false,
            paint_source: None,
            paint_events: Vec::new(),
            cursor: None,
        }
    }
}

impl View2dState {
    /// Fits the 0–1 square in the panel.
    pub fn frame(&mut self) {
        self.transform = UvTransform::framing(self.size[0], self.size[1]);
        self.framed = true;
    }
}

/// Draws the panel and applies the mouse and pen input of this frame (`pen`: the pen
/// gestures of the frame, `pen_near`: whether a pen is in proximity).
pub fn show(
    ui: &mut Ui,
    state: &mut View2dState,
    i18n: &I18n,
    has_project: bool,
    pen: &[PenEvent],
    pen_near: bool,
) {
    state.visible = true;
    if !has_project {
        state.hovered = false;
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new(i18n.tr("view-2d-empty")).weak());
        });
        return;
    }
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let pixels_per_point = ui.ctx().pixels_per_point();
    state.size = [
        ((rect.width() * pixels_per_point).round() as u32).max(1),
        ((rect.height() * pixels_per_point).round() as u32).max(1),
    ];
    if let Some(texture) = state.texture {
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter().image(texture, rect, uv, egui::Color32::WHITE);
    }
    ui.painter().text(
        rect.left_bottom() + egui::vec2(8.0, -6.0),
        egui::Align2::LEFT_BOTTOM,
        i18n.tr("view-2d-hint"),
        egui::FontId::proportional(12.0),
        egui::Color32::from_gray(170),
    );

    state.hovered = response.hovered();
    let events = view_paint_events(
        ui,
        &response,
        rect,
        &mut state.painting,
        &mut state.paint_source,
        pen,
        pen_near,
    );
    state.paint_events.extend(events);
    if let (Some(pointer), Some((radius, paintable))) = (response.hover_pos(), state.cursor) {
        let color = if paintable {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_gray(120)
        };
        let radius = radius / pixels_per_point;
        ui.painter().circle_stroke(
            pointer,
            radius,
            egui::Stroke::new(2.5, egui::Color32::from_black_alpha(140)),
        );
        ui.painter()
            .circle_stroke(pointer, radius, egui::Stroke::new(1.2, color));
    }
    if response.dragged_by(PointerButton::Middle) {
        let delta = response.drag_delta() * pixels_per_point;
        state.transform.pan([delta.x, delta.y]);
    }
    if response.hovered() {
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll != 0.0
            && let Some(pointer) = response.hover_pos()
        {
            let local = (pointer - rect.min) * pixels_per_point;
            state.transform.zoom_at(
                (scroll * WHEEL_ZOOM_SPEED).exp(),
                [local.x, local.y],
                state.size,
            );
        }
    }
}
