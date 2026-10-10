//! Orbit camera with reverse-Z projections.
//!
//! Reverse-Z maps the near plane to depth 1 and infinity (or the far plane) to 0, which
//! spreads f32 depth precision evenly. Depth tests use `GreaterEqual` and clear to 0.

use std::f32::consts::FRAC_PI_2;

use glam::{Mat4, Vec2, Vec3, Vec4};
use patine_core::mesh::BoundingBox;
use patine_mesh::bvh::Ray;

/// Right-handed view matrix looking from `eye` towards `center`.
pub fn look_at_rh(eye: Vec3, center: Vec3, up: Vec3) -> Mat4 {
    let forward = (center - eye).normalize();
    let side = forward.cross(up).normalize();
    let up = side.cross(forward);
    Mat4::from_cols(
        Vec4::new(side.x, up.x, -forward.x, 0.0),
        Vec4::new(side.y, up.y, -forward.y, 0.0),
        Vec4::new(side.z, up.z, -forward.z, 0.0),
        Vec4::new(-side.dot(eye), -up.dot(eye), forward.dot(eye), 1.0),
    )
}

/// Right-handed perspective with an infinite far plane and reverse-Z depth: the plane
/// `z = -near` maps to depth 1, infinity to 0.
pub fn perspective_infinite_reverse_rh(fov_y: f32, aspect: f32, near: f32) -> Mat4 {
    let f = 1.0 / (fov_y * 0.5).tan();
    Mat4::from_cols(
        Vec4::new(f / aspect, 0.0, 0.0, 0.0),
        Vec4::new(0.0, f, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 0.0, -1.0),
        Vec4::new(0.0, 0.0, near, 0.0),
    )
}

/// Right-handed orthographic projection to depth [0, 1]: `z = -near` maps to 0 and
/// `z = -far` to 1. Pass the planes swapped for reverse-Z.
pub fn orthographic_rh(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Mat4 {
    let width = 1.0 / (right - left);
    let height = 1.0 / (top - bottom);
    let depth = 1.0 / (near - far);
    Mat4::from_cols(
        Vec4::new(2.0 * width, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 2.0 * height, 0.0, 0.0),
        Vec4::new(0.0, 0.0, depth, 0.0),
        Vec4::new(
            -(left + right) * width,
            -(top + bottom) * height,
            depth * near,
            1.0,
        ),
    )
}

/// Kind of projection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Projection {
    /// Perspective with an infinite far plane.
    #[default]
    Perspective,
    /// Orthographic; its half-height follows the distance and FOV, so zooming works the
    /// same way in both modes.
    Orthographic,
}

/// Camera orbiting around a target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitCamera {
    /// Point looked at and orbited around.
    pub target: Vec3,
    /// Distance from the target to the eye.
    pub distance: f32,
    /// Rotation around the vertical axis, radians. 0 looks towards -Z.
    pub yaw: f32,
    /// Elevation, radians, within ±[`MAX_PITCH`].
    pub pitch: f32,
    /// Vertical field of view, radians.
    pub fov_y: f32,
    /// Projection kind.
    pub projection: Projection,
    /// Radius of the scene around the target, set by [`OrbitCamera::frame`]. Bounds the
    /// orthographic depth range, which must not depend on the zoom.
    pub scene_radius: f32,
}

/// Largest elevation, just short of the pole to keep the up vector defined.
pub const MAX_PITCH: f32 = FRAC_PI_2 - 0.01;
const MIN_DISTANCE: f32 = 1e-4;
const MAX_DISTANCE: f32 = 1e6;

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            distance: 4.0,
            yaw: 0.6,
            pitch: 0.35,
            fov_y: 40f32.to_radians(),
            projection: Projection::Perspective,
            scene_radius: 1.0,
        }
    }
}

impl OrbitCamera {
    /// Unit vector from the target to the eye.
    pub fn back(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        Vec3::new(cos_pitch * sin_yaw, sin_pitch, cos_pitch * cos_yaw)
    }

    /// Eye position.
    pub fn eye(&self) -> Vec3 {
        self.target + self.back() * self.distance
    }

    /// World-to-view matrix (right-handed, looking down -Z).
    pub fn view(&self) -> Mat4 {
        look_at_rh(self.eye(), self.target, Vec3::Y)
    }

    /// Distance of the near plane, relative to the orbit distance.
    fn near(&self) -> f32 {
        (self.distance * 1e-3).max(1e-5)
    }

    /// Half-height of the orthographic view volume.
    fn ortho_half_height(&self) -> f32 {
        self.distance * (self.fov_y * 0.5).tan()
    }

    /// View-to-clip matrix for the aspect ratio `width / height`, reverse-Z.
    pub fn projection_matrix(&self, aspect: f32) -> Mat4 {
        match self.projection {
            Projection::Perspective => {
                perspective_infinite_reverse_rh(self.fov_y, aspect, self.near())
            }
            Projection::Orthographic => {
                let half_height = self.ortho_half_height();
                let half_width = half_height * aspect;
                // Depth range covering the scene around the target, even behind the eye;
                // arguments swapped for reverse-Z.
                let depth = 2.0 * self.scene_radius + self.distance;
                orthographic_rh(
                    -half_width,
                    half_width,
                    -half_height,
                    half_height,
                    self.distance + depth,
                    self.distance - depth,
                )
            }
        }
    }

    /// World-to-clip matrix.
    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        self.projection_matrix(aspect) * self.view()
    }

    /// Ray through `ndc` (x right, y up, both in [-1, 1]), in world space.
    pub fn ray(&self, ndc: Vec2, aspect: f32) -> Ray {
        let inverse = self.view_projection(aspect).inverse();
        // Reverse-Z: depth 1 is the near plane; 0.5 lies inside the volume for both
        // projections (infinite perspective reaches depth 0 only at infinity).
        let near = inverse.project_point3(ndc.extend(1.0));
        let inside = inverse.project_point3(ndc.extend(0.5));
        let direction = (inside - near).normalize();
        Ray {
            origin: match self.projection {
                // Start at the eye so geometry between the eye and the near plane counts.
                Projection::Perspective => self.eye(),
                Projection::Orthographic => near,
            },
            direction,
        }
    }

    /// Rotates around the target. Positive `dx` turns right, positive `dy` looks from
    /// higher up.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx;
        self.pitch = (self.pitch + dy).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Moves the target in the view plane by a mouse drag of `(dx, dy)` pixels, for a
    /// viewport `height` pixels tall: the point under the cursor follows it at the
    /// target's depth.
    pub fn pan(&mut self, dx: f32, dy: f32, height: f32) {
        let world_per_pixel = 2.0 * self.ortho_half_height() / height.max(1.0);
        let view = self.view();
        let right = view.row(0).truncate();
        let up = view.row(1).truncate();
        self.target += (-dx * right + dy * up) * world_per_pixel;
    }

    /// Multiplies the distance by `factor` (below 1 zooms in).
    pub fn zoom(&mut self, factor: f32) {
        self.distance = (self.distance * factor).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    /// Centers the camera on `bounds` so its bounding sphere fits the view for the aspect
    /// ratio `aspect`, keeping the orientation.
    pub fn frame(&mut self, bounds: &BoundingBox, aspect: f32) {
        let min = Vec3::from(bounds.min);
        let max = Vec3::from(bounds.max);
        self.target = (min + max) * 0.5;
        let radius = ((max - min).length() * 0.5).max(1e-3);
        self.scene_radius = radius;
        // Half-angle of the narrowest direction of the view.
        let half_y = self.fov_y * 0.5;
        let half_x = (half_y.tan() * aspect).atan();
        let half = half_y.min(half_x);
        // 5% margin around the sphere.
        self.distance = (radius * 1.05 / half.sin()).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depth(camera: &OrbitCamera, point: Vec3) -> f32 {
        camera.view_projection(1.0).project_point3(point).z
    }

    #[test]
    fn eye_is_at_distance_and_target_is_centered() {
        let camera = OrbitCamera {
            target: Vec3::new(1.0, 2.0, 3.0),
            ..OrbitCamera::default()
        };
        assert!((camera.eye().distance(camera.target) - camera.distance).abs() < 1e-5);
        let ndc = camera.view_projection(1.5).project_point3(camera.target);
        assert!(ndc.x.abs() < 1e-5 && ndc.y.abs() < 1e-5);
    }

    #[test]
    fn reverse_z_puts_near_points_at_higher_depth() {
        for projection in [Projection::Perspective, Projection::Orthographic] {
            let camera = OrbitCamera {
                projection,
                ..OrbitCamera::default()
            };
            let back = camera.back();
            let near = depth(&camera, camera.target + back);
            let middle = depth(&camera, camera.target);
            // Behind the target but inside the scene (the orthographic range follows
            // `scene_radius`).
            let far = depth(&camera, camera.target - back * camera.scene_radius);
            assert!(
                near > middle && middle > far,
                "{projection:?}: {near} {middle} {far}"
            );
            assert!((0.0..=1.0).contains(&far) && (0.0..=1.0).contains(&near));
        }
    }

    #[test]
    fn orbit_is_reversible_and_pitch_is_clamped() {
        let mut camera = OrbitCamera::default();
        let start = camera;
        camera.orbit(0.7, -0.2);
        camera.orbit(-0.7, 0.2);
        assert!((camera.yaw - start.yaw).abs() < 1e-6 && (camera.pitch - start.pitch).abs() < 1e-6);
        camera.orbit(0.0, 10.0);
        assert_eq!(camera.pitch, MAX_PITCH);
        assert!(camera.view().is_finite());
    }

    #[test]
    fn pan_keeps_orientation_and_distance() {
        let mut camera = OrbitCamera::default();
        let back = camera.back();
        let start = camera.target;
        camera.pan(100.0, -50.0, 800.0);
        assert!((camera.back() - back).length() < 1e-6);
        // The target moved in the view plane only.
        assert!((camera.target - start).dot(back).abs() < 1e-5);
        assert!(camera.target.distance(start) > 0.0);
    }

    #[test]
    fn pan_follows_the_cursor_at_the_target_depth() {
        let mut camera = OrbitCamera::default();
        let height = 600.0;
        // A point of the target plane at the screen center moves 100 px to the right.
        let point = camera.target;
        camera.pan(100.0, 0.0, height);
        let ndc = camera.view_projection(1.0).project_point3(point);
        let pixels = ndc.x * height * 0.5;
        assert!((pixels - 100.0).abs() < 0.5, "{pixels}");
    }

    #[test]
    fn framing_keeps_every_corner_in_view() {
        let bounds = BoundingBox {
            min: [-3.0, 0.0, -1.0],
            max: [5.0, 2.0, 1.0],
        };
        for projection in [Projection::Perspective, Projection::Orthographic] {
            for aspect in [0.5, 1.0, 16.0 / 9.0] {
                let mut camera = OrbitCamera {
                    projection,
                    ..OrbitCamera::default()
                };
                camera.frame(&bounds, aspect);
                let vp = camera.view_projection(aspect);
                for i in 0..8 {
                    let corner = Vec3::new(
                        if i & 1 == 0 {
                            bounds.min[0]
                        } else {
                            bounds.max[0]
                        },
                        if i & 2 == 0 {
                            bounds.min[1]
                        } else {
                            bounds.max[1]
                        },
                        if i & 4 == 0 {
                            bounds.min[2]
                        } else {
                            bounds.max[2]
                        },
                    );
                    let ndc = vp.project_point3(corner);
                    assert!(
                        ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0 && (0.0..=1.0).contains(&ndc.z),
                        "{projection:?} aspect {aspect}: {ndc}"
                    );
                }
            }
        }
    }

    #[test]
    fn rays_go_through_their_pixel() {
        for projection in [Projection::Perspective, Projection::Orthographic] {
            let camera = OrbitCamera {
                projection,
                ..OrbitCamera::default()
            };
            let center = camera.ray(Vec2::ZERO, 1.3);
            let to_target = (camera.target - center.origin).normalize();
            assert!(center.direction.dot(to_target) > 0.9999, "{projection:?}");
            // A point along an off-center ray projects back onto its NDC.
            let ndc = Vec2::new(0.4, -0.7);
            let ray = camera.ray(ndc, 1.3);
            let point = ray.origin + ray.direction * camera.distance;
            let projected = camera.view_projection(1.3).project_point3(point);
            assert!(
                (projected.truncate() - ndc).length() < 1e-4,
                "{projection:?}"
            );
        }
    }

    #[test]
    fn zoom_is_clamped() {
        let mut camera = OrbitCamera::default();
        camera.zoom(0.5);
        assert!((camera.distance - 2.0).abs() < 1e-6);
        camera.zoom(1e-12);
        assert_eq!(camera.distance, MIN_DISTANCE);
    }
}
