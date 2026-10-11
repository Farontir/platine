//! Helpers shared by the render tests.

#![allow(dead_code)] // Each test file uses a subset.

use patine_gpu::{GpuContext, wgpu};

/// A `size`² texture filled with `value` (f16), with the usages of channel textures.
pub fn texture(
    ctx: &GpuContext,
    format: wgpu::TextureFormat,
    value: [f32; 4],
) -> wgpu::TextureView {
    let size = 4;
    let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test channel"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let components = if format == wgpu::TextureFormat::R16Float {
        1
    } else {
        4
    };
    let texel: Vec<u8> = value[..components]
        .iter()
        .flat_map(|v| half::f16::from_f32(*v).to_le_bytes())
        .collect();
    let bytes = texel.repeat((size * size) as usize);
    ctx.queue.write_texture(
        texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size * texel.len() as u32),
            rows_per_image: None,
        },
        texture.size(),
    );
    texture.create_view(&Default::default())
}
