//! 2D view panel: the current texture set in UV space (image rendered by
//! `patine_render::uv_view`), with pan and zoom.

use egui::{PointerButton, RichText, Sense, Ui};
use patine_render::uv_view::UvTransform;

use crate::i18n::I18n;

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

/// Draws the panel and applies the mouse input of this frame.
pub fn show(ui: &mut Ui, state: &mut View2dState, i18n: &I18n, has_project: bool) {
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
