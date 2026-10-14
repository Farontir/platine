//! Editor of a response curve (pressure curves): control points dragged in a square.
//! The first and last points stay at inputs 0 and 1; a double click adds a point, a
//! right click removes one.

use egui::{Color32, Pos2, Rect, Sense, Stroke as Line, Ui, Vec2};
use patine_core::stroke::Curve;

/// Distance in points within which the pointer grabs a control point.
const GRAB: f32 = 8.0;

/// Moves point `index` of `curve` to `to` (input, output), kept in the unit square and
/// between its neighbors; the ends keep their input.
pub fn move_point(curve: &mut Curve, index: usize, to: [f32; 2]) {
    let count = curve.0.len();
    if index >= count {
        return;
    }
    let y = to[1].clamp(0.0, 1.0);
    let x = if index == 0 {
        0.0
    } else if index == count - 1 {
        1.0
    } else {
        let low = curve.0[index - 1][0];
        let high = curve.0[index + 1][0];
        to[0].clamp(low, high)
    };
    curve.0[index] = [x, y];
}

/// Adds a point at `at`, in input order; returns its index. Ends are not duplicated:
/// a point at input 0 or 1 or beyond replaces nothing and is not added.
pub fn insert_point(curve: &mut Curve, at: [f32; 2]) -> Option<usize> {
    let x = at[0];
    if !(x > 0.0 && x < 1.0) {
        return None;
    }
    let index = curve
        .0
        .iter()
        .position(|p| p[0] > x)
        .unwrap_or(curve.0.len());
    curve.0.insert(index, [x, at[1].clamp(0.0, 1.0)]);
    Some(index)
}

/// Removes point `index` unless it is an end.
pub fn remove_point(curve: &mut Curve, index: usize) {
    if index > 0 && index + 1 < curve.0.len() {
        curve.0.remove(index);
    }
}

/// Draws the editor of `curve`, `size` points wide. Returns whether the curve changed
/// and whether a gesture ended.
pub fn curve_editor(ui: &mut Ui, id: egui::Id, curve: &mut Curve, size: f32) -> (bool, bool) {
    // A curve always has its two ends.
    if curve.0.len() < 2 {
        *curve = Curve::identity();
    }
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());
    let to_screen = |p: [f32; 2]| {
        Pos2::new(
            rect.left() + p[0] * rect.width(),
            rect.bottom() - p[1] * rect.height(),
        )
    };
    let from_screen = |p: Pos2| {
        [
            (p.x - rect.left()) / rect.width(),
            (rect.bottom() - p.y) / rect.height(),
        ]
    };
    let closest = |curve: &Curve, pointer: Pos2| {
        curve
            .0
            .iter()
            .enumerate()
            .map(|(i, p)| (i, to_screen(*p).distance(pointer)))
            .filter(|(_, d)| *d <= GRAB)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };

    let mut changed = false;
    let mut ended = false;
    let dragged_key = id.with("dragged");
    let mut dragged: Option<usize> = ui.data(|d| d.get_temp(dragged_key));
    if let Some(pointer) = response.interact_pointer_pos() {
        if response.drag_started() {
            dragged = closest(curve, pointer);
        }
        if response.dragged()
            && let Some(index) = dragged
        {
            let before = curve.0.clone();
            move_point(curve, index, from_screen(pointer));
            changed |= curve.0 != before;
        }
        if response.double_clicked() && closest(curve, pointer).is_none() {
            changed |= insert_point(curve, from_screen(pointer)).is_some();
            ended = true;
        }
        if response.secondary_clicked()
            && let Some(index) = closest(curve, pointer)
        {
            let count = curve.0.len();
            remove_point(curve, index);
            changed |= curve.0.len() != count;
            ended = true;
        }
    }
    if response.drag_stopped() {
        dragged = None;
        ended = true;
    }
    ui.data_mut(|d| d.insert_temp(dragged_key, dragged));

    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);
    let grid = Line::new(1.0, visuals.widgets.noninteractive.bg_stroke.color);
    for i in 1..4 {
        let t = i as f32 / 4.0;
        painter.line_segment([to_screen([t, 0.0]), to_screen([t, 1.0])], grid);
        painter.line_segment([to_screen([0.0, t]), to_screen([1.0, t])], grid);
    }
    let points: Vec<Pos2> = curve.0.iter().map(|p| to_screen(*p)).collect();
    painter.add(egui::Shape::line(
        points.clone(),
        Line::new(2.0, visuals.selection.bg_fill),
    ));
    let hovered = response.hover_pos().and_then(|p| closest(curve, p));
    for (i, point) in points.iter().enumerate() {
        let radius = if Some(i) == hovered || Some(i) == dragged {
            5.0
        } else {
            3.5
        };
        painter.circle_filled(*point, radius, Color32::WHITE);
    }
    painter.rect_stroke(
        Rect::from_min_max(rect.min, rect.max),
        2.0,
        visuals.widgets.noninteractive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    (changed, ended)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ends_keep_their_input() {
        let mut curve = Curve::identity();
        move_point(&mut curve, 0, [0.4, 0.3]);
        move_point(&mut curve, 1, [0.2, 1.5]);
        assert_eq!(curve.0, [[0.0, 0.3], [1.0, 1.0]]);
    }

    #[test]
    fn inner_points_stay_between_their_neighbors() {
        let mut curve = Curve(vec![[0.0, 0.0], [0.3, 0.5], [0.6, 0.5], [1.0, 1.0]]);
        move_point(&mut curve, 1, [0.9, -1.0]);
        assert_eq!(curve.0[1], [0.6, 0.0]);
        move_point(&mut curve, 2, [0.1, 0.7]);
        assert_eq!(curve.0[2], [0.6, 0.7]);
        // Out of range: nothing.
        move_point(&mut curve, 9, [0.5, 0.5]);
        assert_eq!(curve.0.len(), 4);
    }

    #[test]
    fn points_are_inserted_in_order_and_ends_are_never_removed() {
        let mut curve = Curve::identity();
        assert_eq!(insert_point(&mut curve, [0.5, 0.2]), Some(1));
        assert_eq!(insert_point(&mut curve, [0.25, 0.9]), Some(1));
        assert_eq!(insert_point(&mut curve, [1.0, 0.9]), None);
        assert_eq!(insert_point(&mut curve, [0.0, 0.9]), None);
        assert_eq!(curve.0, [[0.0, 0.0], [0.25, 0.9], [0.5, 0.2], [1.0, 1.0]]);
        assert!((curve.evaluate(0.25) - 0.9).abs() < 1e-6);
        remove_point(&mut curve, 0);
        remove_point(&mut curve, 3);
        assert_eq!(curve.0.len(), 4);
        remove_point(&mut curve, 1);
        assert_eq!(curve.0, [[0.0, 0.0], [0.5, 0.2], [1.0, 1.0]]);
    }
}
