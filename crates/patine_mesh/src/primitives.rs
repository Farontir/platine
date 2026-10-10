//! Procedural test meshes with known topology.
//!
//! Every mesh is centered on the origin, has analytic normals and, unless stated
//! otherwise, UVs. Quads are split along their shortest diagonal, as mikktspace
//! recommends, so the split is independent of vertex order.
//!
//! | Mesh | UV islands | Parts |
//! | --- | --- | --- |
//! | [`plane`] | 1 | 1 |
//! | [`cube`] | 6 | 1 |
//! | [`cube_cross`] | 1 | 1 |
//! | [`uv_sphere`] | 1 | 1 |
//! | [`cylinder`] | 3 | 1 |
//! | [`two_cubes`] | 12 | 2 |

use std::f32::consts::{PI, TAU};

use glam::{Vec2, Vec3};
use patine_core::mesh::MeshAsset;

use crate::assemble::{BuildOptions, assemble};
use crate::raw::{NormalSource, RawMesh, RawObject};

const WHITE: [[f32; 4]; 3] = [[1.0; 4]; 3];

/// Accumulates triangles and quads into a [`RawMesh`].
struct Builder {
    raw: RawMesh,
    polygon: u32,
    object: u32,
}

impl Builder {
    fn new(object: &str, with_uvs: bool) -> Self {
        Self {
            raw: RawMesh {
                uvs: with_uvs.then(Vec::new),
                objects: vec![RawObject {
                    name: object.to_owned(),
                    normals: NormalSource::Provided,
                    has_uvs: with_uvs,
                }],
                materials: vec!["default".to_owned()],
                ..RawMesh::default()
            },
            polygon: 0,
            object: 0,
        }
    }

    /// Starts a new source object; following polygons belong to it.
    fn begin_object(&mut self, name: &str) {
        self.object = self.raw.objects.len() as u32;
        self.raw.objects.push(RawObject {
            name: name.to_owned(),
            normals: NormalSource::Provided,
            has_uvs: self.raw.uvs.is_some(),
        });
    }

    fn push(&mut self, p: [Vec3; 3], n: [Vec3; 3], uv: [Vec2; 3], polygon: u32) {
        self.raw.push_triangle(
            p.map(<[f32; 3]>::from),
            n.map(<[f32; 3]>::from),
            uv.map(<[f32; 2]>::from),
            WHITE,
            polygon,
            self.object,
            0,
        );
    }

    fn triangle(&mut self, p: [Vec3; 3], n: [Vec3; 3], uv: [Vec2; 3]) {
        let polygon = self.next_polygon();
        self.push(p, n, uv, polygon);
    }

    /// Counter-clockwise quad, split along its shortest diagonal (ties broken by UVs).
    fn quad(&mut self, p: [Vec3; 4], n: [Vec3; 4], uv: [Vec2; 4]) {
        let polygon = self.next_polygon();
        let d02 = p[0].distance_squared(p[2]);
        let d13 = p[1].distance_squared(p[3]);
        let split_13 = if (d02 - d13).abs() > 1e-6 * d02.max(d13) {
            d13 < d02
        } else {
            uv[1].distance_squared(uv[3]) < uv[0].distance_squared(uv[2])
        };
        let [a, b] = if split_13 {
            [[0, 1, 3], [1, 2, 3]]
        } else {
            [[0, 1, 2], [0, 2, 3]]
        };
        for corners in [a, b] {
            self.push(
                corners.map(|i| p[i]),
                corners.map(|i| n[i]),
                corners.map(|i| uv[i]),
                polygon,
            );
        }
    }

    fn next_polygon(&mut self) -> u32 {
        self.polygon += 1;
        self.polygon - 1
    }

    fn finish(self, name: &str) -> MeshAsset {
        let options = BuildOptions {
            require_uvs: self.raw.uvs.is_some(),
        };
        assemble(name, self.raw, options)
            .expect("generated meshes are valid by construction")
            .0
    }
}

/// Square plane of side 2 in the XZ plane, facing +Y, split into `n` × `n` quads.
pub fn plane(n: u32) -> MeshAsset {
    let n = n.max(1);
    let mut builder = Builder::new("plane", true);
    let point = |i: u32, j: u32| {
        let (s, t) = (i as f32 / n as f32, j as f32 / n as f32);
        // +X along s, -Z along t: s × t gives +Y, counter-clockwise from above.
        (
            Vec3::new(2.0 * s - 1.0, 0.0, 1.0 - 2.0 * t),
            Vec2::new(s, t),
        )
    };
    for j in 0..n {
        for i in 0..n {
            let corners = [
                point(i, j),
                point(i + 1, j),
                point(i + 1, j + 1),
                point(i, j + 1),
            ];
            builder.quad(corners.map(|c| c.0), [Vec3::Y; 4], corners.map(|c| c.1));
        }
    }
    builder.finish("plane")
}

/// A face of the cube [-1, 1]³: outward normal and the 3D directions of U and V
/// (`u × v = normal`).
struct CubeFace {
    normal: Vec3,
    u: Vec3,
    v: Vec3,
    /// Cell of the face in the 4 × 3 cross layout (column, row from the bottom).
    cell: (f32, f32),
}

const CUBE_FACES: [CubeFace; 6] = [
    // Middle row of the cross, V along +Y.
    CubeFace {
        normal: Vec3::NEG_X,
        u: Vec3::Z,
        v: Vec3::Y,
        cell: (0.0, 1.0),
    },
    CubeFace {
        normal: Vec3::Z,
        u: Vec3::X,
        v: Vec3::Y,
        cell: (1.0, 1.0),
    },
    CubeFace {
        normal: Vec3::X,
        u: Vec3::NEG_Z,
        v: Vec3::Y,
        cell: (2.0, 1.0),
    },
    CubeFace {
        normal: Vec3::NEG_Z,
        u: Vec3::NEG_X,
        v: Vec3::Y,
        cell: (3.0, 1.0),
    },
    // Top above the front face, bottom below it.
    CubeFace {
        normal: Vec3::Y,
        u: Vec3::X,
        v: Vec3::NEG_Z,
        cell: (1.0, 2.0),
    },
    CubeFace {
        normal: Vec3::NEG_Y,
        u: Vec3::X,
        v: Vec3::Z,
        cell: (1.0, 0.0),
    },
];

fn add_cube(builder: &mut Builder, offset: Vec3, cross_layout: bool) {
    for face in &CUBE_FACES {
        let corner = |s: f32, t: f32| {
            let position =
                offset + face.normal + (2.0 * s - 1.0) * face.u + (2.0 * t - 1.0) * face.v;
            let uv = if cross_layout {
                Vec2::new((face.cell.0 + s) / 4.0, (face.cell.1 + t) / 3.0)
            } else {
                Vec2::new(s, t)
            };
            (position, uv)
        };
        let corners = [
            corner(0.0, 0.0),
            corner(1.0, 0.0),
            corner(1.0, 1.0),
            corner(0.0, 1.0),
        ];
        builder.quad(corners.map(|c| c.0), [face.normal; 4], corners.map(|c| c.1));
    }
}

/// Cube [-1, 1]³ whose six faces each fill the whole UV square: 6 UV islands.
pub fn cube() -> MeshAsset {
    let mut builder = Builder::new("cube", true);
    add_cube(&mut builder, Vec3::ZERO, false);
    builder.finish("cube")
}

/// Cube [-1, 1]³ unwrapped as a cross (4 × 3 cells): a single UV island.
pub fn cube_cross() -> MeshAsset {
    let mut builder = Builder::new("cube_cross", true);
    add_cube(&mut builder, Vec3::ZERO, true);
    builder.finish("cube_cross")
}

/// Two separate cubes, each in its own object: 12 UV islands, 2 parts.
pub fn two_cubes() -> MeshAsset {
    let mut builder = Builder::new("left", true);
    add_cube(&mut builder, Vec3::new(-2.0, 0.0, 0.0), false);
    builder.begin_object("right");
    add_cube(&mut builder, Vec3::new(2.0, 0.0, 0.0), false);
    builder.finish("two_cubes")
}

/// Direction on the unit sphere for longitude `u` and latitude `v` in [0, 1].
fn sphere_direction(u: f32, v: f32) -> Vec3 {
    let (theta, phi) = (u * TAU, v * PI);
    // v = 0 at the south pole, v = 1 at the north pole; U grows counter-clockwise
    // seen from outside (towards -Z from +X).
    Vec3::new(
        phi.sin() * theta.cos(),
        -phi.cos(),
        -phi.sin() * theta.sin(),
    )
}

fn add_sphere(builder: &mut Builder, segments: u32, rings: u32, radius: impl Fn(Vec3) -> f32) {
    let segments = segments.max(3);
    let rings = rings.max(2);
    let vertex = |i: u32, j: u32| {
        let (u, v) = (i as f32 / segments as f32, j as f32 / rings as f32);
        let direction = sphere_direction(u, v);
        (direction * radius(direction), direction, Vec2::new(u, v))
    };
    for j in 0..rings {
        for i in 0..segments {
            let [a, b, c, d] = [
                vertex(i, j),
                vertex(i + 1, j),
                vertex(i + 1, j + 1),
                vertex(i, j + 1),
            ];
            if j == 0 {
                // South pole: one triangle, the pole UV centered on the segment.
                let pole = Vec2::new((i as f32 + 0.5) / segments as f32, 0.0);
                builder.triangle([a.0, c.0, d.0], [a.1, c.1, d.1], [pole, c.2, d.2]);
            } else if j == rings - 1 {
                let pole = Vec2::new((i as f32 + 0.5) / segments as f32, 1.0);
                builder.triangle([a.0, b.0, c.0], [a.1, b.1, c.1], [a.2, b.2, pole]);
            } else {
                builder.quad(
                    [a.0, b.0, c.0, d.0],
                    [a.1, b.1, c.1, d.1],
                    [a.2, b.2, c.2, d.2],
                );
            }
        }
    }
}

/// Unit UV sphere with a seam along one meridian: a single UV island.
pub fn uv_sphere(segments: u32, rings: u32) -> MeshAsset {
    let mut builder = Builder::new("uv_sphere", true);
    add_sphere(&mut builder, segments, rings, |_| 1.0);
    builder.finish("uv_sphere")
}

/// Cylinder of radius 1 and height 2 with hard-edged caps: 3 UV islands (side, top,
/// bottom).
pub fn cylinder(segments: u32) -> MeshAsset {
    let segments = segments.max(3);
    let mut builder = Builder::new("cylinder", true);
    let ring = |i: u32| {
        let angle = i as f32 / segments as f32 * TAU;
        Vec3::new(angle.cos(), 0.0, -angle.sin())
    };
    // Side in the lower half of the UV square.
    for i in 0..segments {
        let (r0, r1) = (ring(i), ring(i + 1));
        let (u0, u1) = (i as f32 / segments as f32, (i + 1) as f32 / segments as f32);
        builder.quad(
            [r0 - Vec3::Y, r1 - Vec3::Y, r1 + Vec3::Y, r0 + Vec3::Y],
            [r0, r1, r1, r0],
            [
                Vec2::new(u0, 0.0),
                Vec2::new(u1, 0.0),
                Vec2::new(u1, 0.5),
                Vec2::new(u0, 0.5),
            ],
        );
    }
    // Caps as triangle fans, each in a disk of the upper half.
    for (y, center_uv) in [
        (1.0f32, Vec2::new(0.25, 0.75)),
        (-1.0, Vec2::new(0.75, 0.75)),
    ] {
        let normal = Vec3::Y * y;
        let center = normal;
        let uv = |p: Vec3| center_uv + 0.2 * Vec2::new(p.x, -p.z * y);
        for i in 0..segments {
            let (r0, r1) = (ring(i) + center, ring(i + 1) + center);
            let corners = if y > 0.0 {
                [center, r0, r1]
            } else {
                [center, r1, r0]
            };
            builder.triangle(corners, [normal; 3], corners.map(|p| uv(p - center)));
        }
    }
    builder.finish("cylinder")
}

/// Radius of the high poly of [`low_high_pair`] in a unit `direction`.
///
/// A smooth bump pattern of amplitude 0.03 around the unit sphere; the baker tests
/// (M11) derive the expected normal map from it.
pub fn high_poly_radius(direction: Vec3) -> f32 {
    let k = 6.0;
    1.0 + 0.03 * (k * direction.x).sin() * (k * direction.y).sin() * (k * direction.z).sin()
}

/// A low poly / high poly pair for baking tests.
pub struct LowHighPair {
    /// Unit UV sphere, 32 × 16.
    pub low: MeshAsset,
    /// Dense sphere (256 × 128) displaced by [`high_poly_radius`], without UVs.
    pub high: MeshAsset,
}

/// Builds the [`LowHighPair`].
pub fn low_high_pair() -> LowHighPair {
    let low = uv_sphere(32, 16);
    let mut builder = Builder::new("sphere_high", false);
    // Normals are recomputed from the displaced surface.
    builder.raw.objects[0].normals = NormalSource::Smooth;
    add_sphere(&mut builder, 256, 128, high_poly_radius);
    LowHighPair {
        low,
        high: builder.finish("sphere_high"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_mesh(mesh: &MeshAsset) {
        mesh.validate().unwrap();
        for normal in &mesh.normals {
            assert!(
                (Vec3::from(*normal).length() - 1.0).abs() < 1e-5,
                "{}",
                mesh.name
            );
        }
        if let Some(tangents) = &mesh.tangents {
            for (tangent, normal) in tangents.iter().zip(&mesh.normals) {
                let t = Vec3::new(tangent[0], tangent[1], tangent[2]);
                assert!(t.dot(Vec3::from(*normal)).abs() < 1e-4, "{}", mesh.name);
                assert!(tangent[3] == 1.0 || tangent[3] == -1.0);
            }
        }
        // Winding is counter-clockwise seen from outside: face normals agree with
        // vertex normals.
        for triangle in &mesh.triangles {
            let p = triangle.map(|v| Vec3::from(mesh.positions[v as usize]));
            let face = (p[1] - p[0]).cross(p[2] - p[0]);
            let normal: Vec3 = triangle
                .iter()
                .map(|&v| Vec3::from(mesh.normals[v as usize]))
                .sum();
            assert!(face.dot(normal) > 0.0, "{}: inverted triangle", mesh.name);
        }
    }

    #[test]
    fn generated_meshes_are_valid_and_outward() {
        for mesh in [
            plane(4),
            cube(),
            cube_cross(),
            uv_sphere(16, 8),
            cylinder(12),
            two_cubes(),
        ] {
            check_mesh(&mesh);
        }
    }

    #[test]
    fn triangle_and_polygon_counts() {
        assert_eq!(plane(4).triangle_count(), 32);
        let cube = cube();
        assert_eq!(cube.triangle_count(), 12);
        assert_eq!(cube.vertex_count(), 24);
        assert_eq!(cube.triangle_polygon.iter().max(), Some(&5));
        // 16 × 8 sphere: 2 pole rows of triangles, 6 rows of quads.
        assert_eq!(uv_sphere(16, 8).triangle_count(), 16 * 2 + 16 * 6 * 2);
        assert_eq!(cylinder(12).triangle_count(), 12 * 2 + 12 * 2);
        let two = two_cubes();
        assert_eq!(two.objects, ["left", "right"]);
        assert_eq!(two.triangle_object.iter().filter(|&&o| o == 1).count(), 12);
    }

    #[test]
    fn cube_cross_uvs_fit_the_unit_square() {
        // Hard edges keep 24 vertices (welding needs equal normals); the single UV island
        // of the cross layout is checked by the topology tests.
        let mesh = cube_cross();
        assert_eq!(mesh.vertex_count(), 24);
        for uv in mesh.uv0.unwrap() {
            assert!((0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1]));
        }
    }

    #[test]
    fn low_high_pair_matches_its_radius_function() {
        let pair = low_high_pair();
        check_mesh(&pair.low);
        assert!(pair.high.uv0.is_none());
        assert!(pair.high.triangle_count() > 60_000);
        for position in pair.high.positions.iter().step_by(97) {
            let p = Vec3::from(*position);
            assert!((p.length() - high_poly_radius(p.normalize())).abs() < 1e-5);
        }
    }

    #[test]
    fn generators_are_deterministic() {
        assert_eq!(uv_sphere(24, 12), uv_sphere(24, 12));
        assert_eq!(cylinder(9), cylinder(9));
    }
}
