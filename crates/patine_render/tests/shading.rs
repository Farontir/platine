//! Physically based shading checks on the test adapter.

use glam::Vec3;
use patine_core::mesh::MeshAsset;
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
        materials: None,
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

/// A unit cube resting on a 10 × 10 floor at y = -1.
fn cube_on_floor() -> MeshAsset {
    let cube = primitives::cube();
    let floor = primitives::plane(1);
    let mut mesh = cube.clone();
    let offset = mesh.positions.len() as u32;
    mesh.positions.extend(
        floor
            .positions
            .iter()
            .map(|p| [p[0] * 5.0, -1.0, p[2] * 5.0]),
    );
    mesh.normals.extend(&floor.normals);
    mesh.uv0
        .as_mut()
        .expect("the cube has UVs")
        .extend(floor.uv0.as_ref().expect("the plane has UVs"));
    mesh.tangents
        .as_mut()
        .expect("the cube has tangents")
        .extend(floor.tangents.as_ref().expect("the plane has tangents"));
    mesh.triangles
        .extend(floor.triangles.iter().map(|t| t.map(|v| v + offset)));
    mesh.triangle_polygon
        .extend(floor.triangle_polygon.iter().map(|p| p + 100));
    mesh.triangle_object.extend(&floor.triangle_object);
    mesh.submeshes[0].triangles.end = mesh.triangles.len() as u32;
    mesh.validate().expect("merged mesh is valid");
    mesh
}

gpu_test!(
    /// A cube on a floor, lit from the +X side: with shadows, the floor darkens on the -X
    /// side of the cube and nowhere gets brighter.
    fn cube_casts_a_shadow_on_the_floor(ctx) {
        let mut viewport = Viewport::with_options(&ctx, ViewportOptions::small()).unwrap();
        let light = Vec3::new(0.7, 0.7, 0.0).normalize();
        let sky = EquirectImage::from_fn(128, 64, |d| {
            let sun = if d.dot(light) > 0.995 { 300.0 } else { 0.0 };
            [0.05 + sun; 3]
        });
        viewport.set_environment(&ctx, &sky).unwrap();
        let mesh = GpuMesh::new(&ctx, &cube_on_floor()).unwrap();
        let mut camera = OrbitCamera {
            yaw: 0.0,
            // Looking almost straight down: only the floor and the cube's top face show,
            // both well lit, so only the cast shadow can darken them (faces near the
            // terminator differ between harmonics and the exact cosine).
            pitch: 1.35,
            ..OrbitCamera::default()
        };
        camera.frame(&mesh.bounds, 1.0);
        let render = |viewport: &mut Viewport, shadows: bool| {
            let settings = ViewSettings {
                background: Background::Solid([0.0; 3]),
                tone_mapper: ToneMapper::Linear,
                shadows,
                material: white(0.0, 0.8),
                ..ViewSettings::default()
            };
            let scene = Scene {
                mesh: Some(&mesh),
                materials: None,
                camera: &camera,
                settings: &settings,
            };
            viewport.render(&ctx, &scene, 96, 96);
            readback::read_rgba16f(&ctx, viewport.hdr_output().unwrap()).unwrap()
        };
        let off = render(&mut viewport, false);
        let on = render(&mut viewport, true);
        let luma = |p: &[f32; 4]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
        // Pixels losing at least 40% of their light are in the cast shadow; it falls on the
        // -X side of the cube (left half of the image), away from the light.
        let (mut left, mut right) = (0, 0);
        for (i, (a, b)) in off.iter().zip(&on).enumerate() {
            if luma(a) > 0.01 && luma(b) < 0.6 * luma(a) {
                if i % 96 < 48 {
                    left += 1;
                } else {
                    right += 1;
                }
            }
        }
        eprintln!("shadowed pixels: left {left}, right {right}");
        assert!(left >= 50, "no cast shadow: {left} darkened pixels");
        assert!(left >= 9 * right, "shadow on the wrong side: left {left}, right {right}");
    }
);

gpu_test!(
    /// Shadows off must not change the lighting model: with an intense sun on the right,
    /// the left half of a sphere (facing away from it) still receives the sky. Evaluating
    /// the sun through the order-2 harmonics rang to large negative values there, clamped
    /// to black.
    fn intense_sun_keeps_the_sky_light_on_the_far_side_without_shadows(ctx) {
        let mut viewport = Viewport::with_options(&ctx, ViewportOptions::small()).unwrap();
        let camera = OrbitCamera::default();
        // The camera's right vector (first row of the view matrix).
        let sun = camera.view().row(0).truncate().normalize();
        let sky = [0.1, 0.12, 0.3];
        let environment = EquirectImage::from_fn(256, 128, |d| {
            if d.dot(sun) > 0.9995 { [5e4; 3] } else { sky }
        });
        viewport.set_environment(&ctx, &environment).unwrap();
        let pixels = render_sphere(&ctx, &mut viewport, white(0.0, 1.0), false);
        // Sphere pixels of the left third of the image, away from the silhouette rows.
        let mut far_side = Vec::new();
        for y in SIZE / 4..3 * SIZE / 4 {
            for x in 0..SIZE / 3 {
                let p = pixels[(y * SIZE + x) as usize];
                if p[1] > 0.0 {
                    far_side.push(p);
                }
            }
        }
        assert!(far_side.len() > 30, "too few far-side pixels: {}", far_side.len());
        // Pixels may be brighter than the sky alone (the split-sum specular of a rough
        // surface lets a little of this extreme sun through), never darker.
        let lit = far_side
            .iter()
            .filter(|p| p.iter().zip(sky).all(|(c, s)| *c > 0.5 * s))
            .count();
        assert!(
            lit * 100 >= far_side.len() * 95,
            "{lit} of {} far-side pixels keep the sky light",
            far_side.len()
        );
    }
);

gpu_test!(
    /// Without occluders, the shadow map changes nothing: a convex sphere renders the same
    /// with shadows on and off (no acne, no light leaks).
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
