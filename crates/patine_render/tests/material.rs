//! Textured materials: channel textures, normal mapping, occlusion and emission.

mod common;

use common::texture;
use patine_core::mesh::MeshAsset;
use patine_gpu::{GpuContext, gpu_test, readback, wgpu};
use patine_mesh::primitives;
use patine_render::camera::OrbitCamera;
use patine_render::environment::{EquirectImage, studio_sky};
use patine_render::material::{GpuMaterial, MaterialTextures, Slot};
use patine_render::mesh_gpu::GpuMesh;
use patine_render::settings::{Background, ConstantMaterial, ToneMapper, ViewSettings};
use patine_render::viewport::{Scene, Viewport, ViewportOptions};

const SIZE: u32 = 48;

/// Linear radiance of every pixel of `mesh`, seen from the default camera, with
/// `material` (or the constant material of `settings`).
fn render(
    ctx: &GpuContext,
    viewport: &mut Viewport,
    mesh: &MeshAsset,
    material: Option<&GpuMaterial>,
    settings: &ViewSettings,
) -> Vec<[f32; 4]> {
    let mesh = GpuMesh::new(ctx, mesh).expect("the mesh has triangles");
    let mut camera = OrbitCamera::default();
    camera.frame(&mesh.bounds, 1.0);
    let materials = material.map(std::slice::from_ref);
    let scene = Scene {
        mesh: Some(&mesh),
        materials,
        camera: &camera,
        settings,
    };
    viewport.render(ctx, &scene, SIZE, SIZE);
    readback::read_rgba16f(ctx, viewport.hdr_output().expect("rendered")).expect("readback")
}

fn settings() -> ViewSettings {
    ViewSettings {
        background: Background::Solid([0.0; 3]),
        tone_mapper: ToneMapper::Linear,
        shadows: false,
        ..ViewSettings::default()
    }
}

/// The center pixel of a `SIZE`² image.
fn center(pixels: &[[f32; 4]]) -> [f32; 4] {
    pixels[(SIZE / 2 * SIZE + SIZE / 2) as usize]
}

gpu_test!(
    /// A document material whose textures hold constant values renders exactly like
    /// the constant material with the same values in its factors.
    fn constant_textures_equal_factors(ctx) {
        use wgpu::TextureFormat::{R16Float, Rgba16Float};
        let mut viewport = Viewport::with_options(&ctx, ViewportOptions::small()).unwrap();
        viewport.set_environment(&ctx, &studio_sky(128)).unwrap();
        // Values exactly representable in f16.
        let constant = ConstantMaterial {
            base_color: [0.75, 0.25, 0.125],
            metallic: 0.25,
            roughness: 0.5,
        };
        let base = texture(&ctx, Rgba16Float, [0.75, 0.25, 0.125, 1.0]);
        let metallic = texture(&ctx, R16Float, [0.25; 4]);
        let roughness = texture(&ctx, R16Float, [0.5; 4]);
        let textures = MaterialTextures::default()
            .with(Slot::BaseColor, &base)
            .with(Slot::Metallic, &metallic)
            .with(Slot::Roughness, &roughness);
        let material = viewport.material(&ctx, &textures);
        let sphere = primitives::uv_sphere(48, 24);
        let textured = render(&ctx, &mut viewport, &sphere, Some(&material), &settings());
        let factors = ViewSettings {
            material: constant,
            ..settings()
        };
        let reference = render(&ctx, &mut viewport, &sphere, None, &factors);
        assert_eq!(textured, reference);
        assert!(center(&textured)[0] > center(&textured)[2], "the base color shows");
    }
);

/// A plane of `divisions`² quads facing the camera (+Z) with every normal replaced by
/// `normal`.
fn plane_with_normal(divisions: u32, normal: [f32; 3]) -> MeshAsset {
    let mut mesh = primitives::plane(divisions);
    // The generated plane lies in XZ facing +Y; turn it to face +Z (the camera).
    for p in &mut mesh.positions {
        *p = [p[0], -p[2], p[1]];
    }
    mesh.normals.fill(normal);
    if let Some(tangents) = &mut mesh.tangents {
        tangents.fill([1.0, 0.0, 0.0, 1.0]);
    }
    mesh
}

gpu_test!(
    /// A normal map tilting the normal towards +X shades like geometry whose normals are
    /// tilted the same way: the CPU-side reference is the mesh itself.
    fn normal_map_matches_tilted_geometry(ctx) {
        use wgpu::TextureFormat::Rgba16Float;
        let mut viewport = Viewport::with_options(&ctx, ViewportOptions::small()).unwrap();
        // Light from +X only: tilting the normal towards +X brightens the plane.
        let sky = EquirectImage::from_fn(64, 32, |d| [d.x.max(0.0) * 2.0 + 0.05; 3]);
        viewport.set_environment(&ctx, &sky).unwrap();
        let tilt = [0.6, 0.0, 0.8];
        let normal = texture(&ctx, Rgba16Float, [tilt[0], tilt[1], tilt[2], 0.0]);
        let mapped = viewport.material(&ctx, &MaterialTextures::default().with(Slot::Normal, &normal));
        let flat = viewport.material(&ctx, &MaterialTextures::default());
        let with_map = render(&ctx, &mut viewport, &plane_with_normal(1, [0.0, 0.0, 1.0]), Some(&mapped), &settings());
        let tilted = render(&ctx, &mut viewport, &plane_with_normal(1, tilt), Some(&flat), &settings());
        let untilted = render(&ctx, &mut viewport, &plane_with_normal(1, [0.0, 0.0, 1.0]), Some(&flat), &settings());
        let (a, b, c) = (center(&with_map), center(&tilted), center(&untilted));
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() <= 0.01 * b[i].max(0.05), "mapped {a:?}, tilted {b:?}");
        }
        assert!(a[0] > 1.2 * c[0], "the normal map changes the shading: {a:?} vs {c:?}");
    }
);

gpu_test!(
    /// Emission adds radiance; occlusion darkens the indirect light only.
    fn emission_and_occlusion(ctx) {
        use wgpu::TextureFormat::{R16Float, Rgba16Float};
        let mut viewport = Viewport::with_options(&ctx, ViewportOptions::small()).unwrap();
        let sphere = primitives::uv_sphere(48, 24);

        // In the dark, only emission is seen.
        viewport.set_environment(&ctx, &EquirectImage::uniform(0.0)).unwrap();
        let emissive = texture(&ctx, Rgba16Float, [0.5, 0.25, 0.0, 1.0]);
        let glowing = viewport.material(&ctx, &MaterialTextures::default().with(Slot::Emissive, &emissive));
        let p = center(&render(&ctx, &mut viewport, &sphere, Some(&glowing), &settings()));
        assert!((p[0] - 0.5).abs() < 1e-3 && (p[1] - 0.25).abs() < 1e-3 && p[2].abs() < 1e-3, "{p:?}");

        // Occlusion scales the indirect light linearly and leaves the direct light (the
        // dominant light extracted from the environment) alone: half occlusion lands
        // halfway between none and full, and full occlusion still shows the direct part.
        viewport.set_environment(&ctx, &studio_sky(128)).unwrap();
        let white = texture(&ctx, Rgba16Float, [1.0; 4]);
        let rough = texture(&ctx, R16Float, [1.0; 4]);
        let open = MaterialTextures::default()
            .with(Slot::BaseColor, &white)
            .with(Slot::Roughness, &rough);
        let shade = |viewport: &mut Viewport, occlusion: f32| {
            let ao = texture(&ctx, R16Float, [occlusion; 4]);
            let material = viewport.material(&ctx, &open.with(Slot::AmbientOcclusion, &ao));
            center(&render(&ctx, viewport, &sphere, Some(&material), &settings()))
        };
        let (none, half, full) = (shade(&mut viewport, 1.0), shade(&mut viewport, 0.5), shade(&mut viewport, 0.0));
        for i in 0..3 {
            let midpoint = 0.5 * (none[i] + full[i]);
            assert!((half[i] - midpoint).abs() < 0.01 * none[i], "none {none:?}, half {half:?}, full {full:?}");
            assert!(full[i] > 0.0 && full[i] < 0.9 * none[i], "none {none:?}, full {full:?}");
        }
    }
);

gpu_test!(
    /// Each submesh is drawn with the material of its texture set.
    fn submeshes_use_their_own_material(ctx) {
        use wgpu::TextureFormat::Rgba16Float;
        let mut viewport = Viewport::with_options(&ctx, ViewportOptions::small()).unwrap();
        viewport.set_environment(&ctx, &EquirectImage::uniform(0.0)).unwrap();
        // A plane facing the camera, left half material 0, right half material 1: the
        // generated plane's triangles go row by row, so they are sorted by side.
        let mut mesh = plane_with_normal(4, [0.0, 0.0, 1.0]);
        let center_x = |t: &[u32; 3]| t.iter().map(|v| mesh.positions[*v as usize][0]).sum::<f32>();
        let (mut left, mut right): (Vec<_>, Vec<_>) = (Vec::new(), Vec::new());
        for (i, t) in mesh.triangles.iter().enumerate() {
            if center_x(t) < 0.0 { left.push(i) } else { right.push(i) }
        }
        let order: Vec<usize> = left.iter().chain(&right).copied().collect();
        mesh.triangles = order.iter().map(|i| mesh.triangles[*i]).collect();
        mesh.triangle_polygon = order.iter().map(|i| mesh.triangle_polygon[*i]).collect();
        mesh.triangle_object = order.iter().map(|i| mesh.triangle_object[*i]).collect();
        let split = left.len() as u32;
        mesh.materials = vec!["red".into(), "blue".into()];
        mesh.submeshes = vec![
            patine_core::mesh::Submesh { material: 0, triangles: 0..split },
            patine_core::mesh::Submesh { material: 1, triangles: split..mesh.triangles.len() as u32 },
        ];
        mesh.validate().unwrap();

        let red = texture(&ctx, Rgba16Float, [1.0, 0.0, 0.0, 1.0]);
        let blue = texture(&ctx, Rgba16Float, [0.0, 0.0, 1.0, 1.0]);
        let materials = [
            viewport.material(&ctx, &MaterialTextures::default().with(Slot::Emissive, &red)),
            viewport.material(&ctx, &MaterialTextures::default().with(Slot::Emissive, &blue)),
        ];
        let gpu_mesh = GpuMesh::new(&ctx, &mesh).unwrap();
        assert_eq!(gpu_mesh.submeshes.len(), 2);
        let mut camera = OrbitCamera::default();
        camera.frame(&gpu_mesh.bounds, 1.0);
        let settings = settings();
        let scene = Scene {
            mesh: Some(&gpu_mesh),
            materials: Some(&materials),
            camera: &camera,
            settings: &settings,
        };
        viewport.render(&ctx, &scene, SIZE, SIZE);
        let pixels = readback::read_rgba16f(&ctx, viewport.hdr_output().unwrap()).unwrap();
        let at = |x: u32| pixels[(SIZE / 2 * SIZE + x) as usize];
        let (l, r) = (at(SIZE / 2 - 6), at(SIZE / 2 + 6));
        assert!(l[0] > 0.9 && l[2] < 0.01, "left {l:?}");
        assert!(r[2] > 0.9 && r[0] < 0.01, "right {r:?}");
    }
);
