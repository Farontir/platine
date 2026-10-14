//! The camera of a stroke, the dabs as the shaders read them, and the tiles of a texture
//! set that a batch of dabs touches.

use std::collections::BTreeSet;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3, Vec4};
use patine_core::mesh::MeshAsset;
use patine_core::stroke::{Dab, Stroke, StrokeCamera};
use patine_mesh::bvh::Bvh;
use patine_procedural::surface::material_triangles;

use crate::DILATION;

/// Side of the tiles painting works on, in texels (the compositor's tiles).
pub const TILE: u32 = 128;

/// Clip `w` under which a point counts as behind the camera.
pub const W_MIN: f32 = 1e-6;

/// The camera of a stroke: where a world point lands on its viewport, how far it is,
/// and which way the camera is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeProjection {
    /// World to clip.
    pub view_proj: Mat4,
    /// Viewport size in pixels.
    pub viewport: Vec2,
    /// `dot(depth_axis, clip)` is the distance of a point from the camera along the view
    /// direction, in world units, up to a constant: `w` for a perspective, the depth
    /// for an orthographic projection (where `w` is 1).
    pub depth_axis: Vec4,
    /// World gradient of that distance: the view direction, away from the camera.
    pub forward: Vec3,
}

impl StrokeProjection {
    /// The projection of `camera`.
    pub fn new(camera: &StrokeCamera) -> Self {
        let view_proj = Mat4::from_cols_array_2d(&camera.view_proj);
        let row = |i: usize| view_proj.row(i);
        let w_gradient = row(3).truncate();
        let depth_axis = if w_gradient.length() > 1e-6 {
            Vec4::W
        } else {
            // Reverse-Z orthographic: z grows toward the camera, in units of the
            // projection's depth scale.
            Vec4::new(0.0, 0.0, -1.0 / row(2).truncate().length().max(1e-12), 0.0)
        };
        let forward = (0..4)
            .map(|i| row(i).truncate() * depth_axis[i])
            .fold(Vec3::ZERO, |a, b| a + b);
        Self {
            view_proj,
            viewport: Vec2::new(camera.viewport[0] as f32, camera.viewport[1] as f32),
            depth_axis,
            forward,
        }
    }

    /// Clip coordinates of `p`.
    pub fn clip(&self, p: Vec3) -> Vec4 {
        self.view_proj * p.extend(1.0)
    }

    /// Viewport pixel of the clip position `clip` (y down), or `None` behind the camera.
    pub fn pixel(&self, clip: Vec4) -> Option<Vec2> {
        if clip.w <= W_MIN {
            return None;
        }
        let ndc = Vec2::new(clip.x, clip.y) / clip.w;
        Some(Vec2::new(
            (ndc.x * 0.5 + 0.5) * self.viewport.x,
            (0.5 - ndc.y * 0.5) * self.viewport.y,
        ))
    }

    /// Distance of the clip position `clip` from the camera, in world units, up to a
    /// constant.
    pub fn depth(&self, clip: Vec4) -> f32 {
        self.depth_axis.dot(clip)
    }

    /// Unit direction toward the camera from a point at clip position `clip`: the
    /// direction along which its pixel does not change, oriented toward smaller depths.
    pub fn toward_camera(&self, clip: Vec4) -> Vec3 {
        let ndc = Vec2::new(clip.x, clip.y) / clip.w;
        let row = |i: usize| self.view_proj.row(i);
        let a = (row(0) - row(3) * ndc.x).truncate();
        let b = (row(1) - row(3) * ndc.y).truncate();
        let d = a.cross(b);
        let length = d.length();
        if length <= 1e-20 {
            return -self.forward.normalize_or_zero();
        }
        let d = d / length;
        if d.dot(self.forward) > 0.0 { -d } else { d }
    }
}

/// One dab as the shaders read it. Must match `Dab` in `shaders/paint/dabs.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuDab {
    /// Center in viewport pixels.
    pub center: [f32; 2],
    /// Radius in pixels.
    pub radius: f32,
    /// Paint deposited at the center, 0 to 1.
    pub flow: f32,
    /// Cosine and sine of the dab's rotation.
    pub rotation: [f32; 2],
    /// Edge hardness, 0 to 1.
    pub hardness: f32,
    /// Unused, for alignment.
    pub padding: f32,
}

/// The dab `dab` of `stroke` as the shaders read it. Pressure, jitters and the
/// direction of the path shape dabs from M9 on.
pub fn gpu_dab(stroke: &Stroke, dab: &Dab) -> GpuDab {
    let brush = &stroke.brush;
    GpuDab {
        center: dab.position,
        radius: (brush.size * 0.5).max(0.5),
        flow: brush.flow.clamp(0.0, 1.0),
        rotation: [brush.angle.cos(), brush.angle.sin()],
        hardness: brush.hardness.clamp(0.0, 1.0),
        padding: 0.0,
    }
}

/// The triangles of one texture set, with a hierarchy to find those under the dabs.
#[derive(Clone, Debug)]
pub struct SetGeometry {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    triangles: Vec<[u32; 3]>,
    bvh: Bvh,
}

impl SetGeometry {
    /// The triangles of material `material` of `mesh`.
    pub fn new(mesh: &MeshAsset, material: u32) -> Self {
        let triangles = material_triangles(mesh, material);
        let uvs = mesh
            .uv0
            .clone()
            .unwrap_or_else(|| vec![[0.0, 0.0]; mesh.positions.len()]);
        let bvh = Bvh::build(&mesh.positions, &triangles);
        Self {
            positions: mesh.positions.clone(),
            uvs,
            triangles,
            bvh,
        }
    }

    /// The tiles of a `size`² texture of the set that `dabs` may paint, sorted: the
    /// tiles around every triangle whose screen bounds meet a dab, widened by the
    /// painted margin around the islands ([`DILATION`]). Conservative: a texel outside
    /// gets nothing from these dabs.
    pub fn touched_tiles(
        &self,
        projection: &StrokeProjection,
        dabs: &[GpuDab],
        size: u32,
    ) -> Vec<[u32; 2]> {
        let rects: Vec<(Vec2, Vec2)> = dabs
            .iter()
            .map(|dab| {
                let center = Vec2::from(dab.center);
                let reach = Vec2::splat(dab.radius + 1.0);
                (center - reach, center + reach)
            })
            .collect();
        // Screen bounds of a world box, or `None` when part of it is behind the camera.
        let screen = |min: Vec3, max: Vec3| -> Option<(Vec2, Vec2)> {
            let mut lo = Vec2::splat(f32::INFINITY);
            let mut hi = Vec2::splat(f32::NEG_INFINITY);
            for i in 0..8 {
                let corner = Vec3::new(
                    if i & 1 == 0 { min.x } else { max.x },
                    if i & 2 == 0 { min.y } else { max.y },
                    if i & 4 == 0 { min.z } else { max.z },
                );
                let pixel = projection.pixel(projection.clip(corner))?;
                lo = lo.min(pixel);
                hi = hi.max(pixel);
            }
            Some((lo, hi))
        };
        let meets = |min: Vec3, max: Vec3| match screen(min, max) {
            None => true,
            Some((lo, hi)) => rects
                .iter()
                .any(|(a, b)| lo.cmple(*b).all() && hi.cmpge(*a).all()),
        };
        let mut found = Vec::new();
        self.bvh
            .query_bounds(&self.positions, &self.triangles, meets, &mut found);

        let columns = size.div_ceil(TILE).max(1);
        let margin = DILATION + 1.0;
        let mut tiles = BTreeSet::new();
        for triangle in found {
            let corners =
                self.triangles[triangle as usize].map(|v| Vec2::from(self.uvs[v as usize]));
            let lo = corners[0].min(corners[1]).min(corners[2]) * size as f32 - 0.5 - margin;
            let hi = corners[0].max(corners[1]).max(corners[2]) * size as f32 - 0.5 + margin;
            let tile = |v: f32| ((v.max(0.0) as u32) / TILE).min(columns - 1);
            if hi.x < 0.0 || hi.y < 0.0 {
                continue;
            }
            for y in tile(lo.y)..=tile(hi.y) {
                for x in tile(lo.x)..=tile(hi.x) {
                    tiles.insert([y, x]);
                }
            }
        }
        tiles.into_iter().map(|[y, x]| [x, y]).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera(view_proj: Mat4) -> StrokeCamera {
        StrokeCamera {
            view_proj: view_proj.to_cols_array_2d(),
            viewport: [200, 100],
        }
    }

    /// Right-handed view matrix (the viewport camera's convention).
    fn look(eye: Vec3, center: Vec3) -> Mat4 {
        let forward = (center - eye).normalize();
        let side = forward.cross(Vec3::Y).normalize();
        let up = side.cross(forward);
        Mat4::from_cols(
            Vec4::new(side.x, up.x, -forward.x, 0.0),
            Vec4::new(side.y, up.y, -forward.y, 0.0),
            Vec4::new(side.z, up.z, -forward.z, 0.0),
            Vec4::new(-side.dot(eye), -up.dot(eye), forward.dot(eye), 1.0),
        )
    }

    /// Reverse-Z orthographic projection: `z = -near` maps to depth 1, `z = -far` to 0.
    fn orthographic(half_width: f32, half_height: f32, near: f32, far: f32) -> Mat4 {
        let depth = 1.0 / (far - near);
        Mat4::from_cols(
            Vec4::new(1.0 / half_width, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 1.0 / half_height, 0.0, 0.0),
            Vec4::new(0.0, 0.0, depth, 0.0),
            Vec4::new(0.0, 0.0, depth * far, 1.0),
        )
    }

    #[test]
    fn perspective_depth_is_the_view_distance() {
        let proj = Mat4::from_cols(
            Vec4::new(1.0, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 2.0, 0.0, 0.0),
            Vec4::new(0.0, 0.0, 0.0, -1.0),
            Vec4::new(0.0, 0.0, 0.1, 0.0),
        );
        let eye = Vec3::new(0.0, 0.0, 5.0);
        let p = StrokeProjection::new(&camera(proj * look(eye, Vec3::ZERO)));
        let clip = p.clip(Vec3::new(0.0, 0.0, 1.0));
        assert!((p.depth(clip) - 4.0).abs() < 1e-5);
        assert_eq!(p.pixel(clip), Some(Vec2::new(100.0, 50.0)));
        // Toward the camera, from a point off axis: along the line to the eye.
        let point = Vec3::new(1.0, 0.5, 0.0);
        let toward = p.toward_camera(p.clip(point));
        assert!(toward.dot((eye - point).normalize()) > 0.9999, "{toward}");
        // Behind the camera.
        assert_eq!(p.pixel(p.clip(Vec3::new(0.0, 0.0, 9.0))), None);
    }

    #[test]
    fn orthographic_depth_grows_away_from_the_camera() {
        let proj = orthographic(2.0, 1.0, 0.1, 20.0);
        let p = StrokeProjection::new(&camera(proj * look(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO)));
        let near = p.depth(p.clip(Vec3::new(0.3, 0.0, 1.0)));
        let far = p.depth(p.clip(Vec3::new(0.3, 0.0, -1.0)));
        assert!((far - near - 2.0).abs() < 1e-4, "{near} {far}");
        let toward = p.toward_camera(p.clip(Vec3::new(0.5, 0.2, 0.0)));
        assert!((toward - Vec3::Z).length() < 1e-5, "{toward}");
    }

    /// Every texel a dab can paint (its nearest surface point lands under the dab, within
    /// the painted margin) lies in a touched tile, and far dabs touch nothing.
    #[test]
    fn touched_tiles_hold_every_painted_texel() {
        use patine_procedural::reference::surface::{closest_barycentric, surface_at};
        use patine_procedural::surface::gpu_vertices;

        use crate::alphas::find;
        use crate::reference::dab_coverage;

        let mesh = patine_mesh::primitives::cube_cross();
        let geometry = SetGeometry::new(&mesh, 0);
        let vertices = gpu_vertices(&mesh);
        let size = 512;
        let proj = Mat4::from_cols(
            Vec4::new(1.2, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 2.4, 0.0, 0.0),
            Vec4::new(0.0, 0.0, 0.0, -1.0),
            Vec4::new(0.0, 0.0, 0.05, 0.0),
        );
        let projection =
            StrokeProjection::new(&camera(proj * look(Vec3::new(2.0, 1.5, 2.5), Vec3::ZERO)));
        let dabs: Vec<GpuDab> = [[90.0, 40.0], [100.0, 55.0], [130.0, 70.0]]
            .iter()
            .map(|center| GpuDab {
                center: *center,
                radius: 12.0,
                flow: 1.0,
                rotation: [0.8, 0.6],
                hardness: 0.3,
                padding: 0.0,
            })
            .collect();
        let tiles = geometry.touched_tiles(&projection, &dabs, size);
        assert!(!tiles.is_empty());
        assert!(
            tiles
                .windows(2)
                .all(|w| (w[0][1], w[0][0]) < (w[1][1], w[1][0]))
        );
        let alpha = find("alpha.splatter").map(|a| a.image());

        let triangles = &geometry.triangles;
        let mut painted = 0;
        for y in 0..size {
            for x in 0..size {
                // The nearest triangle in UV space, as the surface map gives it.
                let p = (Vec2::new(x as f32, y as f32) + 0.5) / size as f32;
                let nearest = (0..triangles.len())
                    .map(|i| {
                        let [a, b, c] = triangles[i].map(|v| Vec2::from(geometry.uvs[v as usize]));
                        let w = closest_barycentric(p, a, b, c);
                        ((a * w.x + b * w.y + c * w.z - p).length(), i)
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .expect("the set has triangles");
                if nearest.0 * size as f32 > DILATION {
                    continue;
                }
                let surface = surface_at(
                    &vertices,
                    triangles,
                    nearest.1 as u32 + 1,
                    [x, y],
                    size as f32,
                );
                let Some(pixel) = projection.pixel(projection.clip(surface.position)) else {
                    continue;
                };
                let covered = dabs.iter().any(|dab| {
                    dab_coverage(dab, pixel, None) > 0.0
                        || dab_coverage(dab, pixel, alpha.as_ref()) > 0.0
                });
                if covered {
                    painted += 1;
                    assert!(
                        tiles.contains(&[x / TILE, y / TILE]),
                        "texel {x},{y} painted outside {tiles:?}"
                    );
                }
            }
        }
        assert!(painted > 100, "the dabs paint the cube ({painted} texels)");

        // Dabs off the mesh touch nothing.
        let away = [GpuDab {
            center: [5.0, 95.0],
            radius: 3.0,
            ..dabs[0]
        }];
        assert!(geometry.touched_tiles(&projection, &away, size).is_empty());
    }
}
