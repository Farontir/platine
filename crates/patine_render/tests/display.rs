//! Display modes: the inspected channel and the UV checker, read back as displayed.

mod common;

use common::texture;
use glam::Vec3;
use patine_gpu::{GpuContext, gpu_test, readback, wgpu};
use patine_mesh::primitives;
use patine_render::camera::OrbitCamera;
use patine_render::environment::{EquirectImage, studio_sky};
use patine_render::material::{GpuMaterial, InspectKind, MaterialTextures};
use patine_render::mesh_gpu::GpuMesh;
use patine_render::reference;
use patine_render::settings::{DisplayMode, ToneMapper, ViewSettings};
use patine_render::viewport::{Scene, Viewport, ViewportOptions};

const SIZE: u32 = 48;

/// Displayed 8-bit pixels of a sphere with `material`, in `display` mode.
fn displayed(
    ctx: &GpuContext,
    viewport: &mut Viewport,
    material: &GpuMaterial,
    display: DisplayMode,
) -> Vec<[u8; 4]> {
    let mesh = GpuMesh::new(ctx, &primitives::uv_sphere(48, 24)).expect("triangles");
    let mut camera = OrbitCamera::default();
    camera.frame(&mesh.bounds, 1.0);
    let settings = ViewSettings {
        display,
        // Ignored by the unlit modes.
        exposure_ev: 3.0,
        tone_mapper: ToneMapper::AgX,
        ..ViewSettings::default()
    };
    let scene = Scene {
        mesh: Some(&mesh),
        materials: Some(std::slice::from_ref(material)),
        camera: &camera,
        settings: &settings,
    };
    viewport.render(ctx, &scene, SIZE, SIZE);
    let (texture, _) = viewport.output().expect("rendered");
    let bytes = readback::read_texture(ctx, texture).expect("readback");
    bytes.as_chunks::<4>().0.to_vec()
}

fn center(pixels: &[[u8; 4]]) -> [u8; 4] {
    pixels[(SIZE / 2 * SIZE + SIZE / 2) as usize]
}

fn assert_near(actual: [u8; 4], expected: [f32; 3]) {
    for i in 0..3 {
        let expected = (expected[i] * 255.0).round();
        assert!(
            (f32::from(actual[i]) - expected).abs() <= 1.0,
            "displayed {actual:?}, expected {expected} in channel {i}"
        );
    }
}

gpu_test!(
    /// Each kind of channel shows its value, whatever the lighting, exposure and tone
    /// mapper: colors sRGB-encoded, data as is, normals remapped to 0–1.
    fn inspected_channels_show_their_values(ctx) {
        use wgpu::TextureFormat::{R16Float, Rgba16Float};
        let mut viewport = Viewport::with_options(&ctx, ViewportOptions::small()).unwrap();
        viewport.set_environment(&ctx, &studio_sky(128)).unwrap();

        let roughness = texture(&ctx, R16Float, [0.3; 4]);
        let scalar = viewport.material(&ctx, &MaterialTextures {
            inspect: Some((&roughness, InspectKind::Scalar)),
            ..Default::default()
        });
        assert_near(center(&displayed(&ctx, &mut viewport, &scalar, DisplayMode::Channel)), [0.3; 3]);
        // The mask mode shows a mask as an inspected scalar.
        let mask = texture(&ctx, R16Float, [0.6; 4]);
        let masked = viewport.material(&ctx, &MaterialTextures {
            inspect: Some((&mask, InspectKind::Scalar)),
            ..Default::default()
        });
        assert_near(center(&displayed(&ctx, &mut viewport, &masked, DisplayMode::Mask)), [0.6; 3]);

        let color = [0.75, 0.25, 0.125];
        let base = texture(&ctx, Rgba16Float, [color[0], color[1], color[2], 1.0]);
        let colored = viewport.material(&ctx, &MaterialTextures {
            inspect: Some((&base, InspectKind::Color)),
            ..Default::default()
        });
        let encoded = reference::srgb_encode(Vec3::from(color)).to_array();
        assert_near(center(&displayed(&ctx, &mut viewport, &colored, DisplayMode::Channel)), encoded);

        let normal = texture(&ctx, Rgba16Float, [0.6, 0.0, 0.8, 0.0]);
        let normals = viewport.material(&ctx, &MaterialTextures {
            inspect: Some((&normal, InspectKind::Normal)),
            ..Default::default()
        });
        assert_near(center(&displayed(&ctx, &mut viewport, &normals, DisplayMode::Channel)), [0.8, 0.5, 0.9]);

        // A texture set without the channel shows black.
        let absent = viewport.material(&ctx, &MaterialTextures::default());
        assert_near(center(&displayed(&ctx, &mut viewport, &absent, DisplayMode::Channel)), [0.0; 3]);
    }
);

gpu_test!(
    /// The UV checker alternates light and dark squares and does not depend on lighting.
    fn uv_checker_ignores_lighting(ctx) {
        let mut viewport = Viewport::with_options(&ctx, ViewportOptions::small()).unwrap();
        let material = viewport.material(&ctx, &MaterialTextures::default());
        viewport.set_environment(&ctx, &studio_sky(128)).unwrap();
        let studio = displayed(&ctx, &mut viewport, &material, DisplayMode::UvChecker);
        viewport.set_environment(&ctx, &EquirectImage::uniform(0.0)).unwrap();
        let dark = displayed(&ctx, &mut viewport, &material, DisplayMode::UvChecker);
        assert_eq!(studio, dark);
        let sphere: Vec<u8> = studio
            .iter()
            .map(|p| p[1])
            .filter(|g| *g != studio[0][1])
            .collect();
        let (min, max) = (sphere.iter().min().unwrap(), sphere.iter().max().unwrap());
        assert!(*max > 150 && *min < 120, "light {max} and dark {min} squares");
    }
);
