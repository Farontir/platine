//! M2 acceptance criterion 1: golden image of a roughness × metallic sphere grid.
//!
//! Every sphere is rendered by the real viewport (procedural studio sky, AgX, MSAA 4x,
//! shadows on) in its own cell, then the cells are assembled. Regenerate the reference
//! with `PATINE_UPDATE_GOLDEN=1 cargo test -p patine_render --test golden` on the software
//! adapter.

use image::{Rgba, RgbaImage};
use patine_gpu::testing::golden::Tolerance;
use patine_gpu::{assert_golden, gpu_test, readback};
use patine_mesh::primitives;
use patine_render::camera::OrbitCamera;
use patine_render::mesh_gpu::GpuMesh;
use patine_render::settings::{Background, ConstantMaterial, ToneMapper, ViewSettings};
use patine_render::viewport::{Scene, Viewport, ViewportOptions};

const CELL: u32 = 64;
const STEPS: u32 = 5;

gpu_test!(
    fn sphere_grid_matches_the_golden_image(ctx) {
        let mut viewport = Viewport::with_options(&ctx, ViewportOptions::small()).unwrap();
        let mesh = GpuMesh::new(&ctx, &primitives::uv_sphere(64, 32)).unwrap();
        let mut camera = OrbitCamera::default();
        camera.frame(&mesh.bounds, 1.0);
        // Framing fits the bounding box's sphere (√3 times the unit sphere): come closer.
        camera.zoom(0.62);
        let mut grid = RgbaImage::new(CELL * STEPS, CELL * STEPS);
        for row in 0..STEPS {
            for column in 0..STEPS {
                // Roughness grows downwards, metallic to the right.
                let settings = ViewSettings {
                    background: Background::Solid([0.18; 3]),
                    tone_mapper: ToneMapper::AgX,
                    shadows: true,
                    material: ConstantMaterial {
                        base_color: [0.9, 0.55, 0.3],
                        metallic: column as f32 / (STEPS - 1) as f32,
                        roughness: row as f32 / (STEPS - 1) as f32,
                    },
                    ..ViewSettings::default()
                };
                let scene = Scene {
                    mesh: Some(&mesh),
                    camera: &camera,
                    settings: &settings,
                };
                viewport.render(&ctx, &scene, CELL, CELL);
                let (texture, _) = viewport.output().unwrap();
                let pixels = readback::read_texture(&ctx, texture).unwrap();
                for (i, p) in pixels.as_chunks::<4>().0.iter().enumerate() {
                    let (x, y) = (i as u32 % CELL, i as u32 / CELL);
                    grid.put_pixel(column * CELL + x, row * CELL + y, Rgba(*p));
                }
            }
        }
        assert_golden!("sphere_grid", &grid, Tolerance::gpu_render());
    }
);
