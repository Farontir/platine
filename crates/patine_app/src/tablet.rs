//! Pen tablets, through octotablet (Windows Ink RealTimeStylus, Wayland tablets).
//!
//! The pen also moves the system mouse, which egui sees: while a pen is in proximity,
//! painting follows the pen events (with pressure and tilt) and ignores the mouse ones,
//! so a stroke is never painted twice. Without a tablet, or when octotablet cannot
//! start, the mouse paints alone at full pressure.

use std::sync::Arc;

use octotablet::events::{Event, ToolEvent};
use winit::window::Window;

/// A pen sample: position in physical window pixels, pressure from 0 to 1, tilt X and Y
/// in radians (`[+, +]` towards the right and the bottom of the screen).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenSample {
    /// Position in physical pixels from the top left of the window.
    pub position: [f32; 2],
    /// Pressure, 0 to 1.
    pub pressure: f32,
    /// Tilt from perpendicular, X and Y, radians.
    pub tilt: [f32; 2],
}

/// A pen gesture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PenEvent {
    /// The pen touched the tablet.
    Down(PenSample),
    /// The pen moved while touching.
    Move(PenSample),
    /// The pen left the surface (or the sensing range while touching).
    Up,
}

/// What octotablet reports about a tool, reduced to what painting needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PenInput {
    /// The tool entered the sensing range over the window.
    In,
    /// The tool touched the surface.
    Down,
    /// Axes of the tool.
    Pose {
        /// Position in logical pixels from the top left of the window.
        position: [f32; 2],
        /// Pressure, when reported.
        pressure: Option<f32>,
        /// Tilt X and Y in radians, when reported.
        tilt: Option<[f32; 2]>,
    },
    /// The events since the previous frame happened together.
    Frame,
    /// The tool left the surface.
    Up,
    /// The tool left the sensing range.
    Out,
}

/// A pressure reading clamped to 0 to 1 (drivers may exceed the axis limits).
pub fn normalize_pressure(pressure: f32) -> f32 {
    if pressure.is_finite() {
        pressure.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// A position in logical pixels converted to physical pixels.
pub fn logical_to_physical(position: [f32; 2], scale_factor: f32) -> [f32; 2] {
    [position[0] * scale_factor, position[1] * scale_factor]
}

/// Which device paints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaintSource {
    /// The mouse, through egui, at full pressure.
    Mouse,
    /// The pen, through octotablet.
    Pen,
}

/// The device whose events paint now: the one that started the stroke in progress,
/// otherwise the pen while it is in proximity (its emulated mouse events are then
/// ignored), otherwise the mouse.
pub fn paint_source(stroke: Option<PaintSource>, pen_near: bool) -> PaintSource {
    stroke.unwrap_or(if pen_near {
        PaintSource::Pen
    } else {
        PaintSource::Mouse
    })
}

/// Changes of a frame waiting for its `Frame` event.
#[derive(Clone, Copy, Debug, Default)]
struct Pending {
    down: bool,
    up: bool,
    moved: bool,
    out: bool,
}

/// Turns the tool events of octotablet into pen gestures. Events of a frame arrive in
/// any order and happen together: they are applied at the `Frame` event (or at the end
/// of a pump).
#[derive(Clone, Debug, Default)]
pub struct PenTracker {
    near: bool,
    down: bool,
    sample: Option<PenSample>,
    pending: Pending,
    events: Vec<PenEvent>,
}

impl PenTracker {
    /// Whether a pen is in proximity of the window.
    pub fn near(&self) -> bool {
        self.near
    }

    /// Applies an input; `scale_factor` converts logical pixels to physical ones.
    pub fn feed(&mut self, input: PenInput, scale_factor: f32) {
        match input {
            PenInput::In => self.near = true,
            PenInput::Down => self.pending.down = true,
            PenInput::Pose {
                position,
                pressure,
                tilt,
            } => {
                // A tool already in range at startup sends poses without `In`.
                self.near = true;
                let previous = self.sample.map_or(1.0, |sample| sample.pressure);
                self.sample = Some(PenSample {
                    position: logical_to_physical(position, scale_factor),
                    pressure: pressure.map_or(previous, normalize_pressure),
                    tilt: tilt.unwrap_or([0.0, 0.0]),
                });
                self.pending.moved = true;
            }
            PenInput::Frame => self.flush(),
            PenInput::Up => self.pending.up = true,
            PenInput::Out => self.pending.out = true,
        }
    }

    /// Applies the changes waiting for a frame.
    pub fn flush(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        if let Some(sample) = self.sample {
            if pending.down && !self.down {
                self.down = true;
                self.events.push(PenEvent::Down(sample));
            } else if pending.moved && self.down {
                self.events.push(PenEvent::Move(sample));
            }
        }
        if (pending.up || pending.out) && self.down {
            self.down = false;
            self.events.push(PenEvent::Up);
        }
        if pending.out {
            self.near = false;
            self.sample = None;
        }
    }

    /// The gestures since the last call.
    pub fn take_events(&mut self) -> Vec<PenEvent> {
        std::mem::take(&mut self.events)
    }
}

/// The connection to the tablet service of the system, if any.
pub struct Tablet {
    manager: Option<octotablet::Manager>,
    tracker: PenTracker,
}

impl std::fmt::Debug for Tablet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tablet")
            .field("connected", &self.manager.is_some())
            .field("tracker", &self.tracker)
            .finish()
    }
}

impl Tablet {
    /// Connects to the tablets of `window`. On failure, logs a warning: the mouse
    /// paints alone.
    pub fn new(window: &Arc<Window>) -> Self {
        // The mouse already reaches egui: no emulated tool for it.
        let manager = octotablet::builder::Builder::new()
            .emulate_tool_from_mouse(false)
            .build_shared(window);
        let manager = match manager {
            Ok(manager) => {
                tracing::info!(backend = ?manager.backed(), "tablet service connected");
                for tool in manager.tools() {
                    log_tool(tool);
                }
                Some(manager)
            }
            Err(error) => {
                tracing::warn!(%error, "no tablet service: the mouse paints at full pressure");
                None
            }
        };
        Self {
            manager,
            tracker: PenTracker::default(),
        }
    }

    /// Whether a pen is in proximity of the window.
    pub fn near(&self) -> bool {
        self.tracker.near()
    }

    /// Reads the pending tablet events; `scale_factor` is the window's. Returns the pen
    /// gestures since the last call, positions in physical window pixels.
    pub fn pump(&mut self, scale_factor: f32) -> Vec<PenEvent> {
        let Some(manager) = &mut self.manager else {
            return Vec::new();
        };
        let events = match manager.pump() {
            Ok(events) => events,
            Err(error) => {
                tracing::warn!(%error, "tablet events lost");
                return self.tracker.take_events();
            }
        };
        for event in events {
            let Event::Tool { tool, event } = event else {
                continue;
            };
            let input = match event {
                ToolEvent::Added => {
                    log_tool(tool);
                    continue;
                }
                ToolEvent::In { .. } => PenInput::In,
                ToolEvent::Down => PenInput::Down,
                ToolEvent::Pose(pose) => PenInput::Pose {
                    position: pose.position,
                    // A tool without a pressure axis presses fully.
                    pressure: pose
                        .pressure
                        .get()
                        .or(tool.axes.pressure.is_none().then_some(1.0)),
                    tilt: pose.tilt,
                },
                ToolEvent::Frame(_) => PenInput::Frame,
                ToolEvent::Up => PenInput::Up,
                ToolEvent::Out => PenInput::Out,
                ToolEvent::Removed | ToolEvent::Button { .. } => continue,
            };
            self.tracker.feed(input, scale_factor);
        }
        self.tracker.flush();
        self.tracker.take_events()
    }
}

/// Logs a tool and the axes it reports.
fn log_tool(tool: &octotablet::tool::Tool) {
    tracing::info!(
        name = tool.name.as_deref().unwrap_or("?"),
        kind = ?tool.tool_type,
        pressure = tool.axes.pressure.is_some(),
        tilt = tool.axes.tilt.is_some(),
        "tablet tool"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(x: f32, pressure: Option<f32>) -> PenInput {
        PenInput::Pose {
            position: [x, 10.0],
            pressure,
            tilt: Some([0.1, -0.2]),
        }
    }

    fn feed(tracker: &mut PenTracker, inputs: &[PenInput]) -> Vec<PenEvent> {
        for input in inputs {
            tracker.feed(*input, 2.0);
        }
        tracker.take_events()
    }

    fn sample(x: f32, pressure: f32) -> PenSample {
        PenSample {
            position: [x * 2.0, 20.0],
            pressure,
            tilt: [0.1, -0.2],
        }
    }

    #[test]
    fn pressure_is_clamped_to_the_unit_range() {
        assert_eq!(normalize_pressure(0.4), 0.4);
        assert_eq!(normalize_pressure(1.3), 1.0);
        assert_eq!(normalize_pressure(-0.1), 0.0);
        assert_eq!(normalize_pressure(f32::INFINITY), 0.0);
    }

    #[test]
    fn logical_pixels_scale_to_physical_ones() {
        assert_eq!(logical_to_physical([10.0, 2.5], 1.5), [15.0, 3.75]);
        assert_eq!(logical_to_physical([7.0, 8.0], 1.0), [7.0, 8.0]);
    }

    #[test]
    fn the_pen_takes_over_while_near_but_not_during_a_mouse_stroke() {
        assert_eq!(paint_source(None, false), PaintSource::Mouse);
        assert_eq!(paint_source(None, true), PaintSource::Pen);
        assert_eq!(
            paint_source(Some(PaintSource::Mouse), true),
            PaintSource::Mouse
        );
        assert_eq!(
            paint_source(Some(PaintSource::Pen), false),
            PaintSource::Pen
        );
    }

    #[test]
    fn a_pen_stroke_goes_down_moves_and_goes_up() {
        let mut tracker = PenTracker::default();
        // Hovering: near, no gesture.
        let hover = feed(
            &mut tracker,
            &[PenInput::In, pose(1.0, Some(0.0)), PenInput::Frame],
        );
        assert!(hover.is_empty());
        assert!(tracker.near());
        // Down and its pose in any order within a frame: the down has the frame's pose.
        let down = feed(
            &mut tracker,
            &[PenInput::Down, pose(2.0, Some(0.3)), PenInput::Frame],
        );
        assert_eq!(down, [PenEvent::Down(sample(2.0, 0.3))]);
        let moved = feed(&mut tracker, &[pose(3.0, Some(0.5)), PenInput::Frame]);
        assert_eq!(moved, [PenEvent::Move(sample(3.0, 0.5))]);
        // A missing pressure keeps the previous one.
        let moved = feed(&mut tracker, &[pose(4.0, None), PenInput::Frame]);
        assert_eq!(moved, [PenEvent::Move(sample(4.0, 0.5))]);
        let up = feed(
            &mut tracker,
            &[pose(5.0, Some(0.0)), PenInput::Up, PenInput::Frame],
        );
        assert_eq!(up, [PenEvent::Move(sample(5.0, 0.0)), PenEvent::Up]);
        // Hovering again, then leaving.
        assert!(feed(&mut tracker, &[pose(6.0, Some(0.0)), PenInput::Frame]).is_empty());
        assert!(feed(&mut tracker, &[PenInput::Out, PenInput::Frame]).is_empty());
        assert!(!tracker.near());
    }

    #[test]
    fn a_tap_within_one_frame_paints_a_dab() {
        let mut tracker = PenTracker::default();
        let events = feed(
            &mut tracker,
            &[
                PenInput::Up,
                pose(1.0, Some(0.8)),
                PenInput::Down,
                PenInput::Frame,
            ],
        );
        assert_eq!(events, [PenEvent::Down(sample(1.0, 0.8)), PenEvent::Up]);
    }

    #[test]
    fn leaving_the_range_while_touching_ends_the_stroke() {
        let mut tracker = PenTracker::default();
        feed(
            &mut tracker,
            &[PenInput::Down, pose(1.0, Some(1.0)), PenInput::Frame],
        );
        let events = feed(&mut tracker, &[PenInput::Out]);
        // No frame yet: the pump's flush applies it.
        assert!(events.is_empty());
        tracker.flush();
        assert_eq!(tracker.take_events(), [PenEvent::Up]);
        assert!(!tracker.near());
    }

    #[test]
    fn a_tool_without_pressure_presses_fully() {
        let mut tracker = PenTracker::default();
        let events = feed(
            &mut tracker,
            &[PenInput::Down, pose(1.0, Some(1.0)), PenInput::Frame],
        );
        assert_eq!(events, [PenEvent::Down(sample(1.0, 1.0))]);
        // A first pose without pressure: full pressure.
        let mut tracker = PenTracker::default();
        let events = feed(
            &mut tracker,
            &[PenInput::Down, pose(1.0, None), PenInput::Frame],
        );
        assert_eq!(events, [PenEvent::Down(sample(1.0, 1.0))]);
    }
}
