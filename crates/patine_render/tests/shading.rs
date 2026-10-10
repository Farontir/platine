//! Physically based shading checks on the test adapter.

use patine_gpu::{GpuContext, gpu_test, readback};
use patine_mesh::primitives;
use patine_render::camera::OrbitCamera;
use patine_render::environment::{EquirectImage, studio_sky};
use patine_render::mesh_gpu::GpuMesh;
use patine_render::reference;
use patine_render::settings::{Background, ConstantMaterial, ToneMapper, ViewSettings};
use patine_render::viewport::{Scene, Viewport, ViewportOptions};

const SIZE: u32 = 48;

/// Linear radiance of the sphere's pixels (background is black and skipped).
fn render_sphere(
    ctx: &GpuContext,
    viewport: &mut Viewport,
    material: ConstantMaterial,
    shadows: bool,
) -> Vec<[f32; 3]> {
    let mesh = GpuMesh::new(ctx, &primitives::uv_sphere(96, 48)).expect("the sphere has triangles");
    let mut camera = OrbitCamera::default();
    camera.frame(&mesh.bounds, 1.0);
    let settings = ViewSettings {
        background: Background::Solid([0.0; 3]),
        tone_mapper: ToneMapper::Linear,
        shadows,
        material,
        ..ViewSettings::default()
    };
    let scene = Scene {
        mesh: Some(&mesh),
        camera: &camera,
        settings: &settings,
    };
    viewport.render(ctx, &scene, SIZE, SIZE);
    readback::read_rgba16f(ctx, viewport.hdr_output().expect("rendered"))
        .expect("readback")
        .into_iter()
        .map(|p| [p[0], p[1], p[2]])
        .collect()
}

/// Pixels fully inside the sphere: all neighbors are non-background, so MSAA edge
/// blending with the black background does not interfere.
fn interior(pixels: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let lit = |x: u32, y: u32| pixels[(y * SIZE + x) as usize][1] > 0.0;
    let mut result = Vec::new();
    for y in 1..SIZE - 1 {
        for x in 1..SIZE - 1 {
            if (0..9).all(|i| lit(x + i % 3 - 1, y + i / 3 - 1)) {
                result.push(pixels[(y * SIZE + x) as usize]);
            }
        }
    }
    assert!(
        result.len() > 200,
        "sphere covers too few pixels: {}",
        result.len()
    );
    result
}

fn white(metallic: f32, roughness: f32) -> ConstantMaterial {
    ConstantMaterial {
        base_color: [1.0; 3],
        metallic,
        roughness,
    }
}

gpu_test!(
    /// White furnace: under uniform radiance 1, a white dielectric reflects 1 at every
    /// angle (diffuse takes the energy specular leaves), and nothing exceeds 1.
    fn white_furnace(ctx) {
        let mut viewport = Viewport::with_options(&ctx, ViewportOptions::small()).unwrap();
        viewport.set_environment(&ctx, &EquirectImage::uniform(1.0)).unwrap();
        for roughness in [0.3, 1.0] {
            let pixels = interior(&render_sphere(&ctx, &mut viewport, white(0.0, roughness), false));
            for p in &pixels {
                for c in p {
                    assert!((c - 1.0).abs() < 0.04, "dielectric r={roughness}: {p:?}");
                }
            }
        }
        // A white metal reflects A + B of the split-sum integral: at most 1, and less at
        // grazing angles of rough surfaces (single-scattering GGX loses energy). The
        // lower bound comes from the CPU reference over the visible angles.
        for roughness in [0.1, 1.0] {
            let minimum = (1..20)
                .map(|i| {
                    let e = reference::brdf_lut(i as f32 / 20.0, roughness, 512);
                    e.x + e.y
                })
                .fold(f32::INFINITY, f32::min);
            let pixels = interior(&render_sphere(&ctx, &mut viewport, white(1.0, roughness), false));
            for p in &pixels {
                for c in p {
                    assert!(
                        *c <= 1.02 && *c >= minimum - 0.04,
                        "metal r={roughness}: {p:?}, minimum {minimum}"
                    );
                }
            }
        }
    }
);

gpu_test!(
    fn light_from_above_brightens_the_top(ctx) {
        let mut viewport = Viewport::with_options(&ctx, ViewportOptions::small()).unwrap();
        let sky = EquirectImage::from_fn(64, 32, |d| [d.y.max(0.0) * 2.0; 3]);
        viewport.set_environment(&ctx, &sky).unwrap();
        let pixels = render_sphere(&ctx, &mut viewport, white(0.0, 0.8), false);
        let column = SIZE / 2;
        let (top, bottom) = (SIZE / 2 - SIZE / 5, SIZE / 2 + SIZE / 5);
        let at = |y: u32| pixels[(y * SIZE + column) as usize][0];
        assert!(at(top) > 2.0 * at(bottom), "top {} bottom {}", at(top), at(bottom));
    }
);

gpu_test!(
    /// Without occluders, shadows on (residual harmonics + directional light) matches
    /// shadows off (full harmonics) up to the order-2 approximation of the clamped cosine.
    fn shadows_without_occluders_preserve_the_lighting(ctx) {
        let mut viewport = Viewport::with_options(&ctx, ViewportOptions::small()).unwrap();
        viewport.set_environment(&ctx, &studio_sky(128)).unwrap();
        let material = white(0.0, 0.7);
        let off = interior(&render_sphere(&ctx, &mut viewport, material, false));
        let on = interior(&render_sphere(&ctx, &mut viewport, material, true));
        assert_eq!(off.len(), on.len());
        let luma = |p: &[f32; 3]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
        let mut total_off = 0.0;
        let mut total_on = 0.0;
        let mut worst: f32 = 0.0;
        for (a, b) in off.iter().zip(&on) {
            total_off += luma(a);
            total_on += luma(b);
            worst = worst.max((luma(a) - luma(b)).abs() / luma(a).max(0.05));
        }
        let mean = (total_on - total_off).abs() / total_off;
        eprintln!("shadow on/off: mean difference {mean:.4}, worst pixel {worst:.4}");
        assert!(mean < 0.05, "mean difference {mean}");
        assert!(worst < 0.35, "worst pixel difference {worst}");
    }
);
