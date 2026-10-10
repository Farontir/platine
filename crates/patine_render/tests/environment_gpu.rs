//! GPU environment processing against the CPU references (crate::reference).

use glam::Vec3;
use patine_gpu::{GpuContext, gpu_test, readback, wgpu};
use patine_render::environment::{EquirectImage, studio_sky};
use patine_render::environment_gpu::{BrdfLut, EnvironmentSizes, GpuEnvironment};
use patine_render::reference::{
    self, CubeImage, brdf_lut, cube_texel_direction, downsample_cube, equirect_to_cube,
};

/// Reads mip `mip` of a cube texture into a [`CubeImage`].
fn read_cube(ctx: &GpuContext, texture: &wgpu::Texture, mip: u32) -> CubeImage {
    let size = (texture.width() >> mip).max(1);
    CubeImage {
        size,
        faces: std::array::from_fn(|face| {
            readback::read_rgba16f_level(ctx, texture, mip, face as u32)
                .expect("cube readback")
                .into_iter()
                .map(|t| Vec3::new(t[0], t[1], t[2]))
                .collect()
        }),
    }
}

/// Asserts `actual ≈ expected` within f16 storage precision plus `relative`.
fn assert_close(actual: Vec3, expected: Vec3, relative: f32, what: &str) {
    let tolerance = expected.abs().max_element() * relative + 2e-3;
    assert!(
        (actual - expected).abs().max_element() <= tolerance,
        "{what}: GPU {actual} vs CPU {expected}"
    );
}

/// Small sizes: WARP is a CPU rasterizer.
const SIZES: EnvironmentSizes = EnvironmentSizes {
    cube: 16,
    specular: 8,
    specular_mips: 4,
    samples: 1024,
};

gpu_test!(
    fn equirect_to_cube_and_mips_match_the_reference(ctx) {
        // A smooth environment: near a sharp light edge (like the studio sky's key light,
        // 0 to 120 within 0.01 of cosine), the precision of acos/atan2 allowed by the
        // shading languages (lower on WARP) dominates; the mapping and filtering are what
        // this test checks.
        let sky = EquirectImage::from_fn(64, 32, |d| {
            [1.0 + d.y, 0.6 + 0.4 * d.x, 0.3 + 0.2 * d.z]
        });
        let environment = GpuEnvironment::new(&ctx, &sky, SIZES).unwrap();
        let mut expected = equirect_to_cube(&sky, SIZES.cube);
        for mip in 0..5 {
            let actual = read_cube(&ctx, &environment.cube, mip);
            assert_eq!(actual.size, expected.size);
            for face in 0..6 {
                for y in 0..actual.size {
                    for x in 0..actual.size {
                        assert_close(
                            actual.texel(face, x, y),
                            expected.texel(face, x, y),
                            // f16 rounding accumulates through the box filters.
                            2e-3 * (mip + 1) as f32,
                            &format!("mip {mip} face {face} ({x}, {y})"),
                        );
                    }
                }
            }
            expected = downsample_cube(&expected);
        }
    }
);

gpu_test!(
    fn studio_sky_processes_to_finite_values_around_its_key_light(ctx) {
        let environment = GpuEnvironment::new(&ctx, &studio_sky(128), SIZES).unwrap();
        let cube = read_cube(&ctx, &environment.cube, 0);
        let mut brightest = (0.0, Vec3::ZERO);
        for face in 0..6 {
            for y in 0..cube.size {
                for x in 0..cube.size {
                    let texel = cube.texel(face, x, y);
                    assert!(texel.is_finite());
                    if texel.x > brightest.0 {
                        brightest = (texel.x, cube_texel_direction(face, x, y, cube.size));
                    }
                }
            }
        }
        let key = Vec3::new(0.55, 0.65, 0.52).normalize();
        assert!(brightest.1.dot(key) > 0.95, "brightest towards {}", brightest.1);
        for mip in 0..SIZES.specular_mips {
            let specular = read_cube(&ctx, &environment.specular, mip);
            assert!(specular.faces.iter().flatten().all(|t| t.is_finite()), "mip {mip}");
        }
    }
);

gpu_test!(
    fn cube_faces_follow_the_hardware_convention(ctx) {
        // Bright only towards +X: the +X face (0) must hold it, whatever the sampling API.
        let image = EquirectImage::from_fn(64, 32, |d| [d.x.max(0.0) * 10.0, 0.0, 0.0]);
        let environment = GpuEnvironment::new(&ctx, &image, SIZES).unwrap();
        let cube = read_cube(&ctx, &environment.cube, 0);
        let center = SIZES.cube / 2;
        assert!(cube.texel(0, center, center).x > 9.0);
        assert!(cube.texel(1, center, center).x < 0.1);
        // Top of the +Z face (small y) looks up: the +Y axis.
        let up = EquirectImage::from_fn(64, 32, |d| [d.y.max(0.0), 0.0, 0.0]);
        let environment = GpuEnvironment::new(&ctx, &up, SIZES).unwrap();
        let cube = read_cube(&ctx, &environment.cube, 0);
        assert!(cube.texel(4, center, 0).x > cube.texel(4, center, SIZES.cube - 1).x);
        assert!(cube_texel_direction(4, center, 0, SIZES.cube).y > 0.0);
    }
);

gpu_test!(
    fn brdf_lut_matches_the_reference(ctx) {
        let size = 16;
        let samples = 256;
        let lut = BrdfLut::new(&ctx, size, samples).unwrap();
        let texels = readback::read_rgba16f(&ctx, &lut.texture).unwrap();
        for y in 0..size {
            for x in 0..size {
                let n_dot_v = (x as f32 + 0.5) / size as f32;
                let roughness = (y as f32 + 0.5) / size as f32;
                let expected = brdf_lut(n_dot_v, roughness, samples);
                let actual = texels[(y * size + x) as usize];
                assert!(
                    (actual[0] - expected.x).abs() < 2e-3 && (actual[1] - expected.y).abs() < 2e-3,
                    "({x}, {y}): GPU {actual:?} vs CPU {expected}"
                );
            }
        }
    }
);

gpu_test!(
    fn prefiltered_specular_converges_to_the_exact_integral(ctx) {
        // Smooth environment: importance sampling with mip filtering is accurate there.
        let image = EquirectImage::from_fn(64, 32, |d| {
            [1.0 + d.y, 0.6 + 0.4 * d.x, 0.3 + 0.2 * d.z]
        });
        let environment = GpuEnvironment::new(&ctx, &image, SIZES).unwrap();
        let source = equirect_to_cube(&image, SIZES.cube);
        for mip in 0..SIZES.specular_mips {
            let roughness = mip as f32 / (SIZES.specular_mips - 1) as f32;
            let actual = read_cube(&ctx, &environment.specular, mip);
            for face in 0..6 {
                for (x, y) in [(0, 0), (actual.size / 2, actual.size / 2), (actual.size - 1, 0)] {
                    let n = cube_texel_direction(face, x, y, actual.size);
                    let expected = if mip == 0 {
                        // Mirror level: the source at this resolution (one 2 × 2 box step).
                        downsample_cube(&source).texel(face, x, y)
                    } else {
                        reference::prefilter_brute_force(&source, n, roughness)
                    };
                    assert_close(
                        actual.texel(face, x, y),
                        expected,
                        0.03,
                        &format!("mip {mip} face {face} ({x}, {y})"),
                    );
                }
            }
        }
    }
);
