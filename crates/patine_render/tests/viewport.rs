//! Viewport rendering smoke tests on the test adapter.

use patine_gpu::{GpuContext, gpu_test, readback};
use patine_mesh::primitives;
use patine_render::camera::OrbitCamera;
use patine_render::mesh_gpu::GpuMesh;
use patine_render::settings::{Background, ToneMapper, ViewSettings};
use patine_render::viewport::{Scene, Viewport, ViewportOptions};

const SIZE: u32 = 64;

/// Renders `mesh` framed by the default camera and returns the RGBA8 pixels.
fn render(ctx: &GpuContext, mesh: Option<&GpuMesh>, settings: &ViewSettings) -> Vec<[u8; 4]> {
    let mut viewport =
        Viewport::with_options(ctx, ViewportOptions::small()).expect("viewport shaders compile");
    let mut camera = OrbitCamera::default();
    if let Some(mesh) = mesh {
        camera.frame(&mesh.bounds, 1.0);
    }
    let scene = Scene {
        mesh,
        materials: None,
        camera: &camera,
        settings,
    };
    viewport.render(ctx, &scene, SIZE, SIZE);
    let (texture, _) = viewport.output().expect("rendered");
    let bytes = readback::read_texture(ctx, texture).expect("readback");
    bytes.as_chunks::<4>().0.to_vec()
}

fn pixel(pixels: &[[u8; 4]], x: u32, y: u32) -> [u8; 4] {
    pixels[(y * SIZE + x) as usize]
}

gpu_test!(
    fn empty_scene_shows_the_background(ctx) {
        // Linear display: the default AgX look slightly desaturates pure colors.
        let settings = ViewSettings {
            background: Background::Solid([0.5, 0.0, 0.0]),
            tone_mapper: ToneMapper::Linear,
            ..ViewSettings::default()
        };
        let pixels = render(&ctx, None, &settings);
        // sRGB encoding of linear 0.5 is 188.
        for p in [pixel(&pixels, 0, 0), pixel(&pixels, 32, 32)] {
            assert!((i32::from(p[0]) - 188).abs() <= 1 && p[1] == 0 && p[3] == 255, "{p:?}");
        }
    }
);

gpu_test!(
    fn framed_cube_covers_the_center_but_not_the_corners(ctx) {
        let settings = ViewSettings {
            background: Background::Solid([0.0, 0.0, 0.0]),
            ..ViewSettings::default()
        };
        let mesh = GpuMesh::new(&ctx, &primitives::cube()).unwrap();
        let pixels = render(&ctx, Some(&mesh), &settings);
        assert_ne!(pixel(&pixels, 32, 32)[..3], [0, 0, 0]);
        assert_eq!(pixel(&pixels, 0, 0)[..3], [0, 0, 0]);
        assert_eq!(pixel(&pixels, 63, 63)[..3], [0, 0, 0]);
    }
);

gpu_test!(
    fn wireframe_draws_dark_edges(ctx) {
        let base = ViewSettings {
            background: Background::Solid([0.0, 0.0, 0.0]),
            ..ViewSettings::default()
        };
        let mesh = GpuMesh::new(&ctx, &primitives::cube()).unwrap();
        let shaded = render(&ctx, Some(&mesh), &base);
        let wired = render(&ctx, Some(&mesh), &ViewSettings { wireframe: true, ..base });
        let changed = shaded.iter().zip(&wired).filter(|(a, b)| a != b).count();
        assert!(changed > 20, "{changed} pixels changed");
    }
);
