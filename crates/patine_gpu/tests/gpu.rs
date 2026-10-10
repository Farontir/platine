//! GPU smoke tests: the device works, a compute pass writes a texture, readback is exact.

use image::{Rgba, RgbaImage};
use patine_gpu::testing::golden::Tolerance;
use patine_gpu::{GpuContext, assert_golden, fill, gpu_test, readback, wgpu};

const COLOR: [f32; 4] = [0.25, 0.5, 0.75, 1.0];

fn storage_texture(ctx: &GpuContext, width: u32, height: u32) -> wgpu::Texture {
    ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

gpu_test!(
    /// M0 acceptance test: fill a texture on the GPU and read it back.
    fn fill_then_read_back(ctx) {
        // Odd size: exercises the workgroup bounds check and the row padding of readback.
        let texture = storage_texture(&ctx, 37, 19);
        fill::fill_rgba16f(&ctx, &texture, COLOR).unwrap();
        let texels = readback::read_rgba16f(&ctx, &texture).unwrap();
        assert_eq!(texels.len(), 37 * 19);
        // These values are exactly representable in f16.
        assert!(texels.iter().all(|texel| *texel == COLOR), "first texel: {:?}", texels[0]);
    }
);

gpu_test!(
    fn fill_matches_golden_image(ctx) {
        let texture = storage_texture(&ctx, 32, 32);
        fill::fill_rgba16f(&ctx, &texture, COLOR).unwrap();
        let texels = readback::read_rgba16f(&ctx, &texture).unwrap();
        let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let image = RgbaImage::from_fn(32, 32, |x, y| {
            let t = texels[(y * 32 + x) as usize];
            Rgba([to_u8(t[0]), to_u8(t[1]), to_u8(t[2]), to_u8(t[3])])
        });
        assert_golden!("fill_solid", &image, Tolerance::gpu_render());
    }
);

gpu_test!(
    /// Reports the optional capabilities later milestones depend on, and checks that a
    /// reported R16Float storage capability really works.
    fn capabilities_are_consistent(ctx) {
        let info = ctx.adapter_info();
        let caps = ctx.capabilities();
        eprintln!("adapter: {} ({:?}, {:?})", info.name, info.backend, info.device_type);
        eprintln!("capabilities: {caps:?}");
        if caps.r16float_storage {
            let scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let _texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("r16float storage"),
                size: wgpu::Extent3d { width: 4, height: 4, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R16Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING,
                view_formats: &[],
            });
            assert!(pollster::block_on(scope.pop()).is_none());
        }
    }
);

gpu_test!(
    fn software_adapter_is_used_by_default(ctx) {
        if std::env::var(patine_gpu::testing::ADAPTER_ENV).is_err() {
            assert_eq!(ctx.adapter_info().device_type, wgpu::DeviceType::Cpu);
        }
    }
);
