//! The 3D viewport panel: shows the image rendered by `patine_render` and turns mouse
//! input into camera moves. The camera and display settings are view state, not
//! document data.

use egui::{PointerButton, Sense, Ui};
use patine_render::camera::OrbitCamera;
use patine_render::settings::ViewSettings;

/// Radians of orbit per pixel dragged.
const ORBIT_SPEED: f32 = 0.008;
/// Zoom factor exponent per pixel dragged (Alt + right button).
const DRAG_ZOOM_SPEED: f32 = 0.006;
/// Zoom factor exponent per scrolled point.
const WHEEL_ZOOM_SPEED: f32 = 0.0015;
/// Radians of environment rotation per pixel dragged.
const ENVIRONMENT_SPEED: f32 = 0.006;

/// State of the 3D view.
#[derive(Clone, Debug)]
pub struct ViewState {
    /// Camera.
    pub camera: OrbitCamera,
    /// Display settings.
    pub settings: ViewSettings,
    /// Size of the panel in physical pixels, measured during the last layout.
    pub size: [u32; 2],
    /// egui texture showing the rendered image, registered by the application.
    pub texture: Option<egui::TextureId>,
    /// Cursor position in the image, in physical pixels, while hovered.
    pub hover_pixel: Option<[u32; 2]>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            camera: OrbitCamera::default(),
            settings: ViewSettings::default(),
            size: [1, 1],
            texture: None,
            hover_pixel: None,
        }
    }
}

impl ViewState {
    /// Aspect ratio of the panel.
    pub fn aspect(&self) -> f32 {
        self.size[0].max(1) as f32 / self.size[1].max(1) as f32
    }
}

/// Camera gesture derived from the mouse state of a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    /// Rotate around the target.
    Orbit(egui::Vec2),
    /// Move the target in the view plane.
    Pan(egui::Vec2),
    /// Zoom by a factor.
    Zoom(f32),
    /// Rotate the environment.
    RotateEnvironment(f32),
}

/// Gestures for a drag of `delta` points with `button` held and `modifiers`.
///
/// Alt + left orbits, Alt + middle pans, Alt + right zooms, Shift + right rotates the
/// environment. Plain drags are reserved for painting tools.
pub fn drag_gesture(
    button: PointerButton,
    modifiers: egui::Modifiers,
    delta: egui::Vec2,
) -> Option<Gesture> {
    if delta == egui::Vec2::ZERO {
        return None;
    }
    match (button, modifiers.alt, modifiers.shift) {
        (PointerButton::Primary, true, _) => Some(Gesture::Orbit(delta)),
        (PointerButton::Middle, true, _) => Some(Gesture::Pan(delta)),
        (PointerButton::Secondary, true, _) => {
            Some(Gesture::Zoom((delta.y * DRAG_ZOOM_SPEED).exp()))
        }
        (PointerButton::Secondary, false, true) => Some(Gesture::RotateEnvironment(delta.x)),
        _ => None,
    }
}

impl ViewState {
    /// Applies a gesture; `pixels_per_point` converts egui points to image pixels.
    pub fn apply(&mut self, gesture: Gesture, pixels_per_point: f32) {
        match gesture {
            Gesture::Orbit(delta) => self
                .camera
                .orbit(delta.x * ORBIT_SPEED, delta.y * ORBIT_SPEED),
            Gesture::Pan(delta) => self.camera.pan(
                delta.x * pixels_per_point,
                delta.y * pixels_per_point,
                self.size[1] as f32,
            ),
            Gesture::Zoom(factor) => self.camera.zoom(factor),
            Gesture::RotateEnvironment(dx) => {
                self.settings.environment_rotation = (self.settings.environment_rotation
                    + dx * ENVIRONMENT_SPEED)
                    .rem_euclid(std::f32::consts::TAU);
            }
        }
    }
}

/// Draws the viewport panel and applies the mouse input of this frame.
pub fn show(ui: &mut Ui, view: &mut ViewState) {
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let pixels_per_point = ui.ctx().pixels_per_point();
    view.size = [
        ((rect.width() * pixels_per_point).round() as u32).max(1),
        ((rect.height() * pixels_per_point).round() as u32).max(1),
    ];

    if let Some(texture) = view.texture {
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter().image(texture, rect, uv, egui::Color32::WHITE);
    }

    let modifiers = ui.input(|input| input.modifiers);
    for button in [
        PointerButton::Primary,
        PointerButton::Middle,
        PointerButton::Secondary,
    ] {
        if response.dragged_by(button)
            && let Some(gesture) = drag_gesture(button, modifiers, response.drag_delta())
        {
            view.apply(gesture, pixels_per_point);
        }
    }
    if response.hovered() {
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll != 0.0 {
            view.apply(
                Gesture::Zoom((-scroll * WHEEL_ZOOM_SPEED).exp()),
                pixels_per_point,
            );
        }
    }
    view.hover_pixel = response.hover_pos().map(|pos| {
        let local = (pos - rect.min) * pixels_per_point;
        [local.x.max(0.0) as u32, local.y.max(0.0) as u32]
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gestures_follow_the_documented_bindings() {
        let alt = egui::Modifiers::ALT;
        let shift = egui::Modifiers::SHIFT;
        let none = egui::Modifiers::NONE;
        let d = egui::vec2(3.0, -2.0);
        assert_eq!(
            drag_gesture(PointerButton::Primary, alt, d),
            Some(Gesture::Orbit(d))
        );
        assert_eq!(
            drag_gesture(PointerButton::Middle, alt, d),
            Some(Gesture::Pan(d))
        );
        assert!(matches!(
            drag_gesture(PointerButton::Secondary, alt, d),
            Some(Gesture::Zoom(_))
        ));
        assert_eq!(
            drag_gesture(PointerButton::Secondary, shift, d),
            Some(Gesture::RotateEnvironment(3.0))
        );
        // Plain drags belong to the painting tools.
        assert_eq!(drag_gesture(PointerButton::Primary, none, d), None);
        assert_eq!(
            drag_gesture(PointerButton::Primary, alt, egui::Vec2::ZERO),
            None
        );
    }

    #[test]
    fn dragging_down_with_alt_right_zooms_out() {
        let mut view = ViewState::default();
        let start = view.camera.distance;
        let gesture = drag_gesture(
            PointerButton::Secondary,
            egui::Modifiers::ALT,
            egui::vec2(0.0, 50.0),
        );
        view.apply(gesture.unwrap(), 1.0);
        assert!(view.camera.distance > start);
    }

    #[test]
    fn environment_rotation_wraps() {
        let mut view = ViewState::default();
        view.apply(Gesture::RotateEnvironment(-10.0), 1.0);
        assert!((0.0..std::f32::consts::TAU).contains(&view.settings.environment_rotation));
    }
}
