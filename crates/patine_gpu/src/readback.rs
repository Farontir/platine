//! Copies textures back to CPU memory, for tests, export and thumbnails.

use crate::{GpuContext, GpuError};

/// Reads mip 0 / layer 0 of `texture` and returns its texels, rows tightly packed.
///
/// The texture needs `COPY_SRC` usage and an uncompressed, single-aspect format.
/// Blocks until the GPU has finished all submitted work.
pub fn read_texture(ctx: &GpuContext, texture: &wgpu::Texture) -> Result<Vec<u8>, GpuError> {
    read_texture_level(ctx, texture, 0, 0)
}

/// Reads mip `mip` of array layer `layer` (a cube face for cubemaps), rows tightly packed.
pub fn read_texture_level(
    ctx: &GpuContext,
    texture: &wgpu::Texture,
    mip: u32,
    layer: u32,
) -> Result<Vec<u8>, GpuError> {
    let format = texture.format();
    let texel_size = format
        .block_copy_size(None)
        .filter(|_| format.block_dimensions() == (1, 1))
        .ok_or(GpuError::UnsupportedReadbackFormat(format))?;
    let width = (texture.width() >> mip).max(1);
    let height = (texture.height() >> mip).max(1);
    let row_bytes = width * texel_size;
    let padded_row_bytes = row_bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);

    let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(padded_row_bytes) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("readback"),
        });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: mip,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row_bytes),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    ctx.queue.submit([encoder.finish()]);

    let (sender, receiver) = std::sync::mpsc::channel();
    staging.map_async(wgpu::MapMode::Read, .., move |result| {
        // The receiver outlives the poll below; a send failure cannot happen.
        let _ = sender.send(result);
    });
    ctx.device.poll(wgpu::PollType::wait_indefinitely())?;
    receiver
        .recv()
        .map_err(|error| GpuError::BufferMap(error.to_string()))?
        .map_err(|error| GpuError::BufferMap(error.to_string()))?;

    let mut texels = Vec::with_capacity(row_bytes as usize * height as usize);
    {
        let mapped = staging
            .get_mapped_range(..)
            .map_err(|error| GpuError::BufferMap(error.to_string()))?;
        for row in mapped.chunks_exact(padded_row_bytes as usize) {
            texels.extend_from_slice(&row[..row_bytes as usize]);
        }
    }
    staging.unmap();
    Ok(texels)
}

/// Reads the first `size` bytes of `buffer` (which needs `COPY_SRC` usage). Blocks until
/// the GPU has finished all submitted work.
pub fn read_buffer(
    ctx: &GpuContext,
    buffer: &wgpu::Buffer,
    size: u64,
) -> Result<Vec<u8>, GpuError> {
    let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("readback"),
        });
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, size);
    ctx.queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    staging.map_async(wgpu::MapMode::Read, .., move |result| {
        // The receiver outlives the poll below; a send failure cannot happen.
        let _ = sender.send(result);
    });
    ctx.device.poll(wgpu::PollType::wait_indefinitely())?;
    receiver
        .recv()
        .map_err(|error| GpuError::BufferMap(error.to_string()))?
        .map_err(|error| GpuError::BufferMap(error.to_string()))?;
    let bytes = staging
        .get_mapped_range(..)
        .map_err(|error| GpuError::BufferMap(error.to_string()))?
        .to_vec();
    staging.unmap();
    Ok(bytes)
}

/// Reads an `Rgba16Float` texture and converts its texels to `f32`.
pub fn read_rgba16f(ctx: &GpuContext, texture: &wgpu::Texture) -> Result<Vec<[f32; 4]>, GpuError> {
    read_rgba16f_level(ctx, texture, 0, 0)
}

/// Reads mip `mip` of layer `layer` of an `Rgba16Float` texture as `f32`.
pub fn read_rgba16f_level(
    ctx: &GpuContext,
    texture: &wgpu::Texture,
    mip: u32,
    layer: u32,
) -> Result<Vec<[f32; 4]>, GpuError> {
    if texture.format() != wgpu::TextureFormat::Rgba16Float {
        return Err(GpuError::UnsupportedReadbackFormat(texture.format()));
    }
    let bytes = read_texture_level(ctx, texture, mip, layer)?;
    let (texels, _) = bytes.as_chunks::<8>();
    Ok(texels
        .iter()
        .map(|texel| {
            let channel =
                |i: usize| half::f16::from_le_bytes([texel[2 * i], texel[2 * i + 1]]).to_f32();
            [channel(0), channel(1), channel(2), channel(3)]
        })
        .collect())
}

/// Reads mip `mip` of layer `layer` of an `R16Float` texture as `f32`.
pub fn read_r16f_level(
    ctx: &GpuContext,
    texture: &wgpu::Texture,
    mip: u32,
    layer: u32,
) -> Result<Vec<f32>, GpuError> {
    if texture.format() != wgpu::TextureFormat::R16Float {
        return Err(GpuError::UnsupportedReadbackFormat(texture.format()));
    }
    let bytes = read_texture_level(ctx, texture, mip, layer)?;
    let (texels, _) = bytes.as_chunks::<2>();
    Ok(texels
        .iter()
        .map(|texel| half::f16::from_le_bytes(*texel).to_f32())
        .collect())
}
