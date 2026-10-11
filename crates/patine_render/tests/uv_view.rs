//! M4 acceptance criterion 2: the 2D view shows the right UV wireframe.
//!
//! The expected edges come from the mesh on the CPU: every polygon edge of the submesh is
//! drawn at its UV position, the inside of the polygons is not, and neither are the
//! triangulation diagonals.

use std::collections::HashMap;

use image::{Rgba, RgbaImage};
use patine_core::mesh::MeshAsset;
use patine_gpu::testing::golden::Tolerance;
use patine_gpu::{assert_golden, gpu_test, readback};
use patine_mesh::primitives;
use patine_mesh::topology::uv_polygon_edges;
use patine_render::mesh_gpu::GpuMesh;
use patine_render::uv_view::{UvScene, UvTransform, UvView, WIRE_COLOR};

const SIZE: u32 = 256;

struct Image {
    pixels: Vec<[u8; 4]>,
}

impl Image {
    fn at(&self, x: i64, y: i64) -> Option<[u8; 4]> {
        (0..SIZE as i64)
            .contains(&x)
            .then_some(())
            .filter(|_| (0..SIZE as i64).contains(&y))
            .map(|_| self.pixels[(y as u32 * SIZE + x as u32) as usize])
    }

    fn is_wire(&self, x: i64, y: i64) -> bool {
        self.at(x, y).is_some_and(|p| {
            WIRE_COLOR
                .iter()
                .zip(p)
                .all(|(c, v)| (f32::from(v) - c * 255.0).abs() <= 8.0)
        })
    }

    /// Whether a wire pixel is within one pixel of `pixel`.
    fn wire_near(&self, pixel: [f32; 2]) -> bool {
        let (x, y) = (pixel[0].floor() as i64, pixel[1].floor() as i64);
        (-1..=1).any(|dy| (-1..=1).any(|dx| self.is_wire(x + dx, y + dy)))
    }
}

fn render(mesh: &MeshAsset) -> (Image, UvTransform, RgbaImage) {
    let ctx = patine_gpu::testing::context();
    let gpu_mesh = GpuMesh::new(&ctx, mesh).expect("triangles");
    let mut view = UvView::new(&ctx).expect("shaders compile");
    let transform = UvTransform::framing(SIZE, SIZE);
    let scene = UvScene {
        mesh: &gpu_mesh,
        material: 0,
        texture: None,
        transform,
        wireframe: true,
    };
    view.render(&ctx, &scene, SIZE, SIZE);
    let (texture, _) = view.output().expect("rendered");
    let bytes = readback::read_texture(&ctx, texture).expect("readback");
    let pixels: Vec<[u8; 4]> = bytes.as_chunks::<4>().0.to_vec();
    let mut image = RgbaImage::new(SIZE, SIZE);
    for (i, p) in pixels.iter().enumerate() {
        image.put_pixel(i as u32 % SIZE, i as u32 / SIZE, Rgba(*p));
    }
    (Image { pixels }, transform, image)
}

fn lerp(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

gpu_test!(
    fn cube_uv_wireframe_is_exact(ctx) {
        let _ = ctx; // `render` opens its own context, as the application's 2D view does.
        // The cube unfolded as a cross: one island, faces sharing edges, quads to split.
        let mesh = primitives::cube_cross();
        let uvs = mesh.uv0.clone().expect("the cube has UVs");
        let triangles = 0..mesh.triangles.len() as u32;
        let (image, transform, rendered) = render(&mesh);
        let size = [SIZE, SIZE];

        // Every polygon edge is drawn along its whole length.
        let edges = uv_polygon_edges(&mesh, triangles);
        // 4 edges per face, no diagonal. The faces do not share vertices (flat normals),
        // so an edge between two faces comes once per face, both at the same UVs.
        assert_eq!(edges.len(), 24, "edges of the cross");
        for [a, b] in &edges {
            let (a, b) = (uvs[*a as usize], uvs[*b as usize]);
            for t in [0.2, 0.5, 0.8] {
                let pixel = transform.pixel_of(lerp(a, b, t), size);
                assert!(image.wire_near(pixel), "edge {a:?}–{b:?} missing at t = {t} ({pixel:?})");
            }
        }

        // Inside each polygon, and on its triangulation diagonal, nothing is drawn.
        let mut polygons: HashMap<u32, Vec<[u32; 3]>> = HashMap::new();
        for (t, triangle) in mesh.triangles.iter().enumerate() {
            polygons.entry(mesh.triangle_polygon[t]).or_default().push(*triangle);
        }
        for (polygon, triangles) in &polygons {
            let corners: Vec<[f32; 2]> = triangles.iter().flatten().map(|v| uvs[*v as usize]).collect();
            let n = corners.len() as f32;
            let centroid = [
                corners.iter().map(|c| c[0]).sum::<f32>() / n,
                corners.iter().map(|c| c[1]).sum::<f32>() / n,
            ];
            let pixel = transform.pixel_of(centroid, size);
            assert!(!image.wire_near(pixel), "polygon {polygon} has a line inside");
            // The diagonal shared by the two triangles of the quad.
            let shared: Vec<u32> = triangles[0]
                .iter()
                .copied()
                .filter(|v| triangles[1].contains(v))
                .collect();
            assert_eq!(shared.len(), 2, "polygon {polygon} is a quad of two triangles");
            let (a, b) = (uvs[shared[0] as usize], uvs[shared[1] as usize]);
            for t in [0.3, 0.7] {
                let pixel = transform.pixel_of(lerp(a, b, t), size);
                assert!(!image.wire_near(pixel), "diagonal of polygon {polygon} is drawn");
            }
        }

        assert_golden!("uv_view_cube_cross", &rendered, Tolerance::gpu_render());
    }
);
