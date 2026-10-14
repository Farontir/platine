//! The camera of a stroke, the dabs as the shaders read them, and the tiles of a texture
//! set that a batch of dabs touches.

use std::collections::BTreeSet;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3, Vec4};
use patine_core::mesh::MeshAsset;
use patine_core::stroke::{Stroke, StrokeCamera, StrokeSpace};
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
        Self::transformed(camera, Mat4::IDENTITY)
    }

    /// The projection of `camera` seeing the mesh moved by `transform`: the camera of a
    /// symmetry copy.
    pub fn transformed(camera: &StrokeCamera, transform: Mat4) -> Self {
        let view_proj = Mat4::from_cols_array_2d(&camera.view_proj) * transform;
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
    /// Most paint the stroke may hold where this dab lands (pen pressure through the
    /// opacity curve), 0 to 1.
    pub opacity: f32,
}

/// The pixels beyond which none of `dabs` covers anything (`[min x, min y, max x, max
/// y]`): a round dab reaches its radius, an alpha dab the corners of its square (radius ×
/// √2), plus a pixel of margin. Empty (min above max) without dabs.
pub fn dab_bounds(dabs: &[GpuDab]) -> [f32; 4] {
    dabs.iter().fold(
        [f32::MAX, f32::MAX, f32::MIN, f32::MIN],
        |[x0, y0, x1, y1], dab| {
            let reach = dab.radius * std::f32::consts::SQRT_2 + 1.0;
            let [x, y] = dab.center;
            [
                x0.min(x - reach),
                y0.min(y - reach),
                x1.max(x + reach),
                y1.max(y + reach),
            ]
        },
    )
}

/// The projections of `stroke`: in the 3D view, one per symmetry copy (the stroke's
/// camera first); in UV space, the camera alone.
pub fn stroke_projections(stroke: &Stroke) -> Vec<StrokeProjection> {
    match stroke.space {
        StrokeSpace::View => crate::symmetry::copies(&stroke.symmetry)
            .into_iter()
            .map(|copy| StrokeProjection::transformed(&stroke.camera, copy))
            .collect(),
        StrokeSpace::Uv => vec![StrokeProjection::new(&stroke.camera)],
    }
}

/// Screen rectangle a dab may paint, a pixel wider than its alpha turned by 45 degrees.
fn dab_rect(dab: &GpuDab) -> (Vec2, Vec2) {
    let center = Vec2::from(dab.center);
    let reach = Vec2::splat(dab.radius * std::f32::consts::SQRT_2 + 1.0);
    (center - reach, center + reach)
}

/// Tiles of a `size`² texture holding the texel rectangle `lo..hi` (texel coordinates,
/// widened by the painted margin).
fn tiles_of(lo: Vec2, hi: Vec2, size: u32, tiles: &mut BTreeSet<[u32; 2]>) {
    let columns = size.div_ceil(TILE).max(1);
    let margin = DILATION + 1.0;
    let (lo, hi) = (lo - margin, hi + margin);
    if hi.x < 0.0 || hi.y < 0.0 {
        return;
    }
    let tile = |v: f32| ((v.max(0.0) as u32) / TILE).min(columns - 1);
    for y in tile(lo.y)..=tile(hi.y) {
        for x in tile(lo.x)..=tile(hi.x) {
            tiles.insert([y, x]);
        }
    }
}

/// The tiles of a `size`² texture that `dabs` of a stroke painted in UV space may paint:
/// the UV rectangles under the dabs (the camera maps UVs to pixels).
pub fn touched_tiles_uv(
    projection: &StrokeProjection,
    dabs: &[GpuDab],
    size: u32,
) -> Vec<[u32; 2]> {
    let pixel_of = |u: f32, v: f32| {
        let clip = projection.clip(Vec3::new(u, v, 0.0));
        projection.pixel(clip)
    };
    let (Some(origin), Some(along_u), Some(along_v)) =
        (pixel_of(0.0, 0.0), pixel_of(1.0, 0.0), pixel_of(0.0, 1.0))
    else {
        return Vec::new();
    };
    // pixel = origin + A × uv; invert A.
    let (a, b) = (along_u - origin, along_v - origin);
    let det = a.x * b.y - a.y * b.x;
    if det.abs() < 1e-12 {
        return Vec::new();
    }
    let uv_of = |p: Vec2| {
        let d = p - origin;
        Vec2::new(d.x * b.y - d.y * b.x, a.x * d.y - a.y * d.x) / det
    };
    let mut tiles = BTreeSet::new();
    for dab in dabs {
        let (lo, hi) = dab_rect(dab);
        let corners = [lo, hi, Vec2::new(lo.x, hi.y), Vec2::new(hi.x, lo.y)].map(uv_of);
        let uv_lo = corners
            .iter()
            .fold(Vec2::splat(f32::INFINITY), |m, c| m.min(*c));
        let uv_hi = corners
            .iter()
            .fold(Vec2::splat(f32::NEG_INFINITY), |m, c| m.max(*c));
        tiles_of(
            uv_lo * size as f32 - 0.5,
            uv_hi * size as f32 - 0.5,
            size,
            &mut tiles,
        );
    }
    tiles.into_iter().map(|[y, x]| [x, y]).collect()
}

/// The triangles of one texture set, with a hierarchy to find those under the dabs.
#[derive(Clone, Debug)]
pub struct SetGeometry {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
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
        let normals = if mesh.normals.len() == mesh.positions.len() {
            mesh.normals.clone()
        } else {
            Vec::new()
        };
        Self {
            positions: mesh.positions.clone(),
            normals,
            uvs,
            triangles,
            bvh,
        }
    }

    /// The tiles of a `size`² texture of the set that `dabs` of `stroke` may paint,
    /// sorted (see [`SetGeometry::touched_tiles`] and [`touched_tiles_uv`]).
    pub fn stroke_tiles(&self, stroke: &Stroke, dabs: &[GpuDab], size: u32) -> Vec<[u32; 2]> {
        let projections = stroke_projections(stroke);
        match stroke.space {
            StrokeSpace::View => self.touched_tiles(&projections, dabs, size),
            StrokeSpace::Uv => touched_tiles_uv(&projections[0], dabs, size),
        }
    }

    /// The tiles of a `size`² texture of the set that `dabs` may paint through any of
    /// `projections` (the symmetry copies), sorted: the tiles around every triangle
    /// whose screen bounds meet a dab, widened by the painted margin around the islands
    /// ([`DILATION`]). Conservative: a texel outside gets nothing from these dabs.
    pub fn touched_tiles(
        &self,
        projections: &[StrokeProjection],
        dabs: &[GpuDab],
        size: u32,
    ) -> Vec<[u32; 2]> {
        let rects: Vec<(Vec2, Vec2)> = dabs.iter().map(dab_rect).collect();
        let mut tiles = BTreeSet::new();
        for projection in projections {
            self.touched_by(projection, &rects, size, &mut tiles);
        }
        tiles.into_iter().map(|[y, x]| [x, y]).collect()
    }

    /// Whether no texel of `triangle` takes paint from `projection` because it faces
    /// away. A texel's facing is its interpolated normal `Σ wᵢ nᵢ` against the direction
    /// toward the camera, a positive combination of the directions `t(pⱼ)` at the corners
    /// (perspective or orthographic); so `Σᵢⱼ wᵢ wⱼ nᵢ·t(pⱼ)` has the sign of the facing,
    /// and when every `nᵢ·t(pⱼ)` is at most 0 the facing stays under
    /// [`crate::GRAZING`]. Normals more than 90° apart could cancel (the shader then
    /// falls back to +Z): no culling for them, nor behind the camera.
    fn faces_away(&self, projection: &StrokeProjection, triangle: [u32; 3]) -> bool {
        if self.normals.is_empty() {
            return false;
        }
        let n = triangle.map(|v| Vec3::from(self.normals[v as usize]));
        if n[0].dot(n[1]) <= 0.0 || n[1].dot(n[2]) <= 0.0 || n[0].dot(n[2]) <= 0.0 {
            return false;
        }
        triangle.iter().all(|&v| {
            let clip = projection.clip(Vec3::from(self.positions[v as usize]));
            if clip.w <= W_MIN {
                return false;
            }
            let toward = projection.toward_camera(clip);
            n.iter().all(|normal| normal.dot(toward) <= 0.0)
        })
    }

    fn touched_by(
        &self,
        projection: &StrokeProjection,
        rects: &[(Vec2, Vec2)],
        size: u32,
        tiles: &mut BTreeSet<[u32; 2]>,
    ) {
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
        for triangle in found {
            if self.faces_away(projection, self.triangles[triangle as usize]) {
                continue;
            }
            let corners =
                self.triangles[triangle as usize].map(|v| Vec2::from(self.uvs[v as usize]));
            let lo = corners[0].min(corners[1]).min(corners[2]) * size as f32 - 0.5;
            let hi = corners[0].max(corners[1]).max(corners[2]) * size as f32 - 0.5;
            tiles_of(lo, hi, size, tiles);
        }
    }
}

#[cfg(test)]
mod tests {
    proptest::proptest! {
        /// Beyond the bounds, every dab covers nothing, round or with an alpha.
        #[test]
        fn nothing_is_covered_beyond_the_dab_bounds(
            dabs in proptest::collection::vec(
                (0.0f32..500.0, 0.0f32..500.0, 0.3f32..80.0, 0.0f32..6.3, 0.0f32..1.0),
                1..6,
            ),
            far in 0.0f32..300.0,
            side in 0usize..4,
            along in 0.0f32..1.0,
        ) {
            let dabs: Vec<GpuDab> = dabs
                .iter()
                .map(|(x, y, radius, angle, hardness)| GpuDab {
                    center: [*x, *y],
                    radius: *radius,
                    flow: 1.0,
                    rotation: [angle.cos(), angle.sin()],
                    hardness: *hardness,
                    opacity: 1.0,
                })
                .collect();
            let [x0, y0, x1, y1] = dab_bounds(&dabs);
            let t = |a: f32, b: f32| a + (b - a) * along;
            let pixel = match side {
                0 => Vec2::new(x0 - 0.01 - far, t(y0, y1)),
                1 => Vec2::new(x1 + 0.01 + far, t(y0, y1)),
                2 => Vec2::new(t(x0, x1), y0 - 0.01 - far),
                _ => Vec2::new(t(x0, x1), y1 + 0.01 + far),
            };
            let alpha = crate::alphas::ALPHAS[1].image();
            for dab in &dabs {
                proptest::prop_assert_eq!(crate::reference::dab_coverage(dab, pixel, None), 0.0);
                proptest::prop_assert_eq!(
                    crate::reference::dab_coverage(dab, pixel, Some(&alpha)),
                    0.0
                );
            }
        }
    }

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

    /// Whether the camera of `projection` paints the surface point at `clip` with normal
    /// `normal`: the angle the shaders and the reference give (`smoothstep` of the
    /// facing between the [`crate::GRAZING`] bounds) is above 0. Occlusion only paints
    /// less, so it is left out.
    fn faces(projection: &StrokeProjection, clip: Vec4, normal: Vec3) -> bool {
        let facing = normal.dot(projection.toward_camera(clip));
        crate::reference::smoothstep(crate::GRAZING[0], crate::GRAZING[1], facing) > 0.0
    }

    /// Every texel a dab can paint (its nearest surface point lands under the dab, within
    /// the painted margin, and faces the camera) lies in a touched tile, and far dabs
    /// touch nothing.
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
                opacity: 1.0,
            })
            .collect();
        let tiles = geometry.touched_tiles(&[projection], &dabs, size);
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
                let clip = projection.clip(surface.position);
                let Some(pixel) = projection.pixel(clip) else {
                    continue;
                };
                if !faces(&projection, clip, surface.normal) {
                    continue;
                }
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
        assert!(
            geometry
                .touched_tiles(&[projection], &away, size)
                .is_empty()
        );
    }

    /// Texels a stroke paints through `projections` (in UV space with `uv`) that lie
    /// outside `tiles`, and texels painted: brute force over the surface of every texel.
    fn outside(
        mesh: &MeshAsset,
        geometry: &SetGeometry,
        projections: &[StrokeProjection],
        uv: bool,
        dabs: &[GpuDab],
        tiles: &[[u32; 2]],
        size: u32,
    ) -> (usize, usize) {
        use patine_procedural::reference::surface::{closest_barycentric, surface_at};
        use patine_procedural::surface::gpu_vertices;

        use crate::reference::dab_coverage;

        let vertices = gpu_vertices(mesh);
        let triangles = &geometry.triangles;
        let (mut outside, mut painted) = (0, 0);
        for y in 0..size {
            for x in 0..size {
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
                let point = if uv {
                    surface.uv.extend(0.0)
                } else {
                    surface.position
                };
                let covered = projections.iter().any(|projection| {
                    let clip = projection.clip(point);
                    // In UV space every texel faces the camera.
                    (uv || faces(projection, clip, surface.normal))
                        && projection.pixel(clip).is_some_and(|pixel| {
                            dabs.iter().any(|dab| dab_coverage(dab, pixel, None) > 0.0)
                        })
                });
                if covered {
                    painted += 1;
                    outside += usize::from(!tiles.contains(&[x / TILE, y / TILE]));
                }
            }
        }
        (outside, painted)
    }

    proptest::proptest! {
        /// A triangle culled as facing away takes no paint anywhere: at any point, its
        /// interpolated normal against the direction toward the camera stays under the
        /// grazing bound, in perspective and orthographic views.
        #[test]
        fn culled_triangles_take_no_paint(
            corners in proptest::array::uniform3(proptest::array::uniform3(-1.0f32..1.0)),
            normals in proptest::array::uniform3(proptest::array::uniform3(-1.0f32..1.0)),
            eye in proptest::array::uniform3(-4.0f32..4.0),
            orthographic in proptest::bool::ANY,
            weights in proptest::array::uniform3(0.0f32..1.0),
        ) {
            let eye = Vec3::from(eye);
            proptest::prop_assume!(eye.length() > 2.5);
            let normals = normals.map(|n| Vec3::from(n).normalize_or(Vec3::Z));
            let mesh = MeshAsset {
                positions: corners.to_vec(),
                normals: normals.map(|n| n.to_array()).to_vec(),
                uv0: Some(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
                triangles: vec![[0, 1, 2]],
                triangle_polygon: vec![0],
                triangle_object: vec![0],
                objects: vec!["o".into()],
                materials: vec!["m".into()],
                submeshes: vec![patine_core::mesh::Submesh { material: 0, triangles: 0..1 }],
                ..MeshAsset::default()
            };
            let geometry = SetGeometry::new(&mesh, 0);
            let proj = if orthographic {
                Mat4::from_cols(
                    Vec4::new(1.0 / 3.0, 0.0, 0.0, 0.0),
                    Vec4::new(0.0, 1.0 / 3.0, 0.0, 0.0),
                    Vec4::new(0.0, 0.0, -0.05, 0.0),
                    Vec4::new(0.0, 0.0, 0.5, 1.0),
                )
            } else {
                Mat4::from_cols(
                    Vec4::new(1.2, 0.0, 0.0, 0.0),
                    Vec4::new(0.0, 1.2, 0.0, 0.0),
                    Vec4::new(0.0, 0.0, 0.0, -1.0),
                    Vec4::new(0.0, 0.0, 0.05, 0.0),
                )
            };
            let projection = StrokeProjection::new(&camera(proj * look(eye, Vec3::ZERO)));
            if geometry.faces_away(&projection, [0, 1, 2]) {
                let sum = weights[0] + weights[1] + weights[2];
                proptest::prop_assume!(sum > 1e-3);
                let w = Vec3::from(weights) / sum;
                let point = Vec3::from(corners[0]) * w.x
                    + Vec3::from(corners[1]) * w.y
                    + Vec3::from(corners[2]) * w.z;
                let normal = (normals[0] * w.x + normals[1] * w.y + normals[2] * w.z).normalize();
                let clip = projection.clip(point);
                proptest::prop_assume!(clip.w > W_MIN);
                proptest::prop_assert!(!faces(&projection, clip, normal));
            }
        }
    }

    /// A dab behind the cube's faces seen from the front touches only the front faces:
    /// the faces turned away from the camera, which project under it too, are left out.
    #[test]
    fn faces_turned_away_touch_no_tile() {
        let mesh = patine_mesh::primitives::cube_cross();
        let geometry = SetGeometry::new(&mesh, 0);
        let proj = Mat4::from_cols(
            Vec4::new(1.2, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 1.2, 0.0, 0.0),
            Vec4::new(0.0, 0.0, 0.0, -1.0),
            Vec4::new(0.0, 0.0, 0.05, 0.0),
        );
        let projection =
            StrokeProjection::new(&camera(proj * look(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO)));
        let culled = (0..geometry.triangles.len())
            .filter(|&i| geometry.faces_away(&projection, geometry.triangles[i]))
            .count();
        // Seen from +Z, the back face and the four side faces seen edge-on or from behind.
        assert!(culled >= 2, "{culled} triangles culled");
        assert!(culled < geometry.triangles.len());
        let front = (0..geometry.triangles.len())
            .filter(|&i| {
                let normal = Vec3::from(mesh.normals[geometry.triangles[i][0] as usize]);
                normal.z > 0.9
            })
            .all(|i| !geometry.faces_away(&projection, geometry.triangles[i]));
        assert!(front, "front faces are never culled");
        // No point of a culled triangle faces the camera.
        for triangle in geometry
            .triangles
            .iter()
            .filter(|t| geometry.faces_away(&projection, **t))
        {
            let [a, b, c] = triangle.map(|v| Vec3::from(mesh.positions[v as usize]));
            let [na, nb, nc] = triangle.map(|v| Vec3::from(mesh.normals[v as usize]));
            for i in 0..=8 {
                for j in 0..=(8 - i) {
                    let (u, v) = (i as f32 / 8.0, j as f32 / 8.0);
                    let w = 1.0 - u - v;
                    let point = a * u + b * v + c * w;
                    let normal = (na * u + nb * v + nc * w).normalize();
                    assert!(!faces(&projection, projection.clip(point), normal));
                }
            }
        }
    }

    /// With symmetry copies, and in UV space, every painted texel lies in a touched tile.
    #[test]
    fn touched_tiles_hold_symmetry_copies_and_uv_strokes() {
        use patine_core::stroke::Symmetry;

        let mesh = patine_mesh::primitives::cube_cross();
        let geometry = SetGeometry::new(&mesh, 0);
        let size = 512;
        let proj = Mat4::from_cols(
            Vec4::new(1.2, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 2.4, 0.0, 0.0),
            Vec4::new(0.0, 0.0, 0.0, -1.0),
            Vec4::new(0.0, 0.0, 0.05, 0.0),
        );
        let camera = camera(proj * look(Vec3::new(2.0, 1.5, 2.5), Vec3::ZERO));
        let dabs: Vec<GpuDab> = [[90.0, 40.0], [100.0, 55.0], [130.0, 70.0]]
            .iter()
            .map(|center| GpuDab {
                center: *center,
                radius: 12.0,
                flow: 1.0,
                rotation: [1.0, 0.0],
                hardness: 0.3,
                opacity: 1.0,
            })
            .collect();
        let projections: Vec<StrokeProjection> = crate::symmetry::copies(&Symmetry {
            mirror: [true, false, false],
            radial: 3,
        })
        .into_iter()
        .map(|copy| StrokeProjection::transformed(&camera, copy))
        .collect();
        let tiles = geometry.touched_tiles(&projections, &dabs, size);
        let (out, painted) = outside(&mesh, &geometry, &projections, false, &dabs, &tiles, size);
        assert_eq!(out, 0, "{painted} texels painted");
        // The copies paint more than the stroke alone.
        let (_, alone) = outside(
            &mesh,
            &geometry,
            &projections[..1],
            false,
            &dabs,
            &tiles,
            size,
        );
        assert!(painted > alone && alone > 100, "{painted} against {alone}");

        // UV space: a camera mapping the UV square to a 200 × 200 view.
        let uv_camera = StrokeCamera {
            view_proj: Mat4::from_cols(
                Vec4::new(2.0, 0.0, 0.0, 0.0),
                Vec4::new(0.0, -2.0, 0.0, 0.0),
                Vec4::new(0.0, 0.0, 1.0, 0.0),
                Vec4::new(-1.0, 1.0, 0.0, 1.0),
            )
            .to_cols_array_2d(),
            viewport: [200, 200],
        };
        let uv = StrokeProjection::new(&uv_camera);
        assert_eq!(
            uv.pixel(uv.clip(Vec3::new(0.25, 0.5, 0.0))),
            Some(Vec2::new(50.0, 100.0))
        );
        let tiles = touched_tiles_uv(&uv, &dabs, size);
        let (out, painted) = outside(&mesh, &geometry, &[uv], true, &dabs, &tiles, size);
        assert_eq!(out, 0);
        assert!(painted > 100, "{painted}");
    }
}
