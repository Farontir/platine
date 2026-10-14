//! The 3D viewport panel: shows the image rendered by `patine_render` and turns mouse
//! input into camera moves and painting gestures. The camera and display settings are
//! view state, not document data; rotating the environment changes the document, so it
//! is returned to the application as a change. Painting gestures are queued for the
//! application, which paints them after syncing the document.

use egui::{PointerButton, Sense, Ui};
use patine_core::channel::Channel;
use patine_render::camera::OrbitCamera;
use patine_render::settings::ViewSettings;

use crate::display_panel::EnvironmentState;

/// Radians of orbit per pixel dragged.
const ORBIT_SPEED: f32 = 0.008;
/// Zoom factor exponent per pixel dragged (Alt + right button).
const DRAG_ZOOM_SPEED: f32 = 0.006;
/// Zoom factor exponent per scrolled point.
const WHEEL_ZOOM_SPEED: f32 = 0.0015;
/// Radians of environment rotation per pixel dragged.
const ENVIRONMENT_SPEED: f32 = 0.006;

/// A painting gesture, in viewport pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaintEvent {
    /// The left button went down over the mesh: a stroke starts there.
    Press([f32; 2]),
    /// The pointer moved while painting.
    Move([f32; 2]),
    /// The left button was released: the stroke ends.
    Release,
}

/// The painting gestures of a frame: a press starts painting (when `press` is allowed,
/// i.e. over the viewport without Alt), moves follow while painting, a release ends it.
/// Returns whether painting goes on, and the events.
pub fn paint_events(
    painting: bool,
    press: Option<[f32; 2]>,
    moves: &[[f32; 2]],
    released: bool,
) -> (bool, Vec<PaintEvent>) {
    let mut events = Vec::new();
    let mut painting = painting;
    if !painting && let Some(position) = press {
        painting = true;
        events.push(PaintEvent::Press(position));
    }
    if painting {
        events.extend(moves.iter().map(|p| PaintEvent::Move(*p)));
        if released {
            events.push(PaintEvent::Release);
            painting = false;
        }
    }
    (painting, events)
}

/// Seconds a notice stays over the viewport.
const NOTICE_SECONDS: f64 = 3.0;

/// State of the 3D view.
#[derive(Debug)]
pub struct ViewState {
    /// Camera.
    pub camera: OrbitCamera,
    /// Display settings.
    pub settings: ViewSettings,
    /// Environment choice and loading.
    pub environment: EnvironmentState,
    /// Size of the panel in physical pixels, measured during the last layout.
    pub size: [u32; 2],
    /// egui texture showing the rendered image, registered by the application.
    pub texture: Option<egui::TextureId>,
    /// Cursor position in the image, in physical pixels, while hovered.
    pub hover_pixel: Option<[u32; 2]>,
    /// Triangle under the cursor, from the last finished pick.
    pub hovered: Option<u32>,
    /// Frame the mesh at the next render, once the panel size is known.
    pub frame_pending: bool,
    /// Panel size at the previous render, to detect a settled layout.
    pub previous_size: [u32; 2],
    /// Channel shown by the channel display mode.
    pub inspect_channel: Channel,
    /// Whether the left button paints (pressed over the viewport without Alt).
    pub painting: bool,
    /// Painting gestures not handled yet by the application.
    pub paint_events: Vec<PaintEvent>,
    /// Point of the mesh under the cursor, from the last finished pick.
    pub hovered_point: Option<crate::paint_tool::SurfacePoint>,
    /// Brush cursor in viewport pixels, and whether the selection can be painted.
    pub cursor: Option<(Vec<glam::Vec2>, bool)>,
    /// Message shown over the viewport, and when it was set (egui time, seconds).
    pub notice: Option<(String, f64)>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            camera: OrbitCamera::default(),
            settings: ViewSettings::default(),
            environment: EnvironmentState::default(),
            size: [1, 1],
            texture: None,
            hover_pixel: None,
            hovered: None,
            frame_pending: false,
            previous_size: [0, 0],
            inspect_channel: Channel::BaseColor,
            painting: false,
            paint_events: Vec::new(),
            hovered_point: None,
            cursor: None,
            notice: None,
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

/// Environment rotation asked by the viewport this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewportOutput {
    /// New environment rotation, radians.
    pub environment_rotation: Option<f32>,
    /// A drag ended: the next change starts a new history entry.
    pub seal: bool,
}

impl ViewState {
    /// Applies a gesture; `pixels_per_point` converts egui points to image pixels. Camera
    /// gestures move the camera; an environment rotation is returned (the environment
    /// belongs to the document), and also shown at once.
    pub fn apply(&mut self, gesture: Gesture, pixels_per_point: f32) -> Option<f32> {
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
                let rotation = (self.settings.environment_rotation + dx * ENVIRONMENT_SPEED)
                    .rem_euclid(std::f32::consts::TAU);
                self.settings.environment_rotation = rotation;
                return Some(rotation);
            }
        }
        None
    }
}

/// Draws the viewport panel and applies the mouse input of this frame. `overlay` is
/// written in the top-left corner (the display mode when it is not the lit material).
pub fn show(ui: &mut Ui, view: &mut ViewState, overlay: Option<&str>) -> ViewportOutput {
    let mut output = ViewportOutput::default();
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
    if let Some(text) = overlay {
        let position = rect.min + egui::vec2(10.0, 8.0);
        let font = egui::FontId::proportional(14.0);
        let galley = ui
            .painter()
            .layout_no_wrap(text.to_owned(), font, egui::Color32::WHITE);
        let background = egui::Rect::from_min_size(position, galley.size()).expand(4.0);
        ui.painter()
            .rect_filled(background, 4.0, egui::Color32::from_black_alpha(160));
        ui.painter().galley(position, galley, egui::Color32::WHITE);
    }

    let modifiers = ui.input(|input| input.modifiers);
    for button in [
        PointerButton::Primary,
        PointerButton::Middle,
        PointerButton::Secondary,
    ] {
        if response.dragged_by(button)
            && let Some(gesture) = drag_gesture(button, modifiers, response.drag_delta())
            && let Some(rotation) = view.apply(gesture, pixels_per_point)
        {
            output.environment_rotation = Some(rotation);
        }
    }
    output.seal = response.drag_stopped();

    // Painting: the left button without Alt.
    let to_pixels = |pos: egui::Pos2| {
        let local = (pos - rect.min) * pixels_per_point;
        [local.x, local.y]
    };
    let (pressed, released, moves, origin) = ui.input(|input| {
        let moves: Vec<egui::Pos2> = input
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::PointerMoved(pos) => Some(*pos),
                _ => None,
            })
            .collect();
        (
            input.pointer.primary_pressed(),
            input.pointer.primary_released(),
            moves,
            input.pointer.press_origin(),
        )
    });
    let press = (pressed && response.hovered() && !modifiers.alt)
        .then_some(origin)
        .flatten()
        .map(to_pixels);
    let moves: Vec<[f32; 2]> = moves.into_iter().map(to_pixels).collect();
    let (painting, events) = paint_events(view.painting, press, &moves, released);
    view.painting = painting;
    view.paint_events.extend(events);

    if response.hovered()
        && let Some((outline, paintable)) = &view.cursor
    {
        crate::paint_tool::draw_cursor(ui.painter(), rect, pixels_per_point, outline, *paintable);
    }
    let now = ui.input(|input| input.time);
    if let Some((text, since)) = &view.notice {
        if now - since < NOTICE_SECONDS {
            let font = egui::FontId::proportional(14.0);
            let galley = ui
                .painter()
                .layout_no_wrap(text.clone(), font, egui::Color32::WHITE);
            let position = egui::pos2(rect.center().x - galley.size().x / 2.0, rect.max.y - 40.0);
            let background = egui::Rect::from_min_size(position, galley.size()).expand(6.0);
            ui.painter()
                .rect_filled(background, 4.0, egui::Color32::from_black_alpha(190));
            ui.painter().galley(position, galley, egui::Color32::WHITE);
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        } else {
            view.notice = None;
        }
    }

    if response.hovered() {
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll != 0.0 {
            // Zooming never returns an environment rotation.
            let _ = view.apply(
                Gesture::Zoom((-scroll * WHEEL_ZOOM_SPEED).exp()),
                pixels_per_point,
            );
        }
    }
    view.hover_pixel = response.hover_pos().map(|pos| {
        let local = (pos - rect.min) * pixels_per_point;
        [local.x.max(0.0) as u32, local.y.max(0.0) as u32]
    });
    output
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
        assert_eq!(view.apply(gesture.unwrap(), 1.0), None);
        assert!(view.camera.distance > start);
    }

    #[test]
    fn presses_start_strokes_and_releases_end_them() {
        // Not painting, no press: moves are ignored.
        assert_eq!(
            paint_events(false, None, &[[1.0, 1.0]], false),
            (false, vec![])
        );
        let (painting, events) = paint_events(false, Some([5.0, 6.0]), &[[7.0, 8.0]], false);
        assert!(painting);
        assert_eq!(
            events,
            [PaintEvent::Press([5.0, 6.0]), PaintEvent::Move([7.0, 8.0])]
        );
        let (painting, events) = paint_events(true, None, &[[9.0, 9.0]], true);
        assert!(!painting);
        assert_eq!(events, [PaintEvent::Move([9.0, 9.0]), PaintEvent::Release]);
        // A click: press and release in the same frame.
        let (painting, events) = paint_events(false, Some([1.0, 2.0]), &[], true);
        assert!(!painting);
        assert_eq!(events, [PaintEvent::Press([1.0, 2.0]), PaintEvent::Release]);
    }

    #[test]
    fn environment_rotation_wraps() {
        let mut view = ViewState::default();
        let rotation = view.apply(Gesture::RotateEnvironment(-10.0), 1.0);
        let rotation = rotation.expect("an environment rotation is returned");
        assert!((0.0..std::f32::consts::TAU).contains(&rotation));
        assert_eq!(view.settings.environment_rotation, rotation);
    }
}
