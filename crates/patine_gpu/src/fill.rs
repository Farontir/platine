//! Fills a texture with a constant value on the GPU.

use wgpu::util::DeviceExt as _;

use crate::{GpuContext, GpuError};

const WORKGROUP_SIZE: u32 = 8;

/// Fills `texture` (`Rgba16Float`, with `STORAGE_BINDING` usage) with `color`.
///
/// The work is submitted to the queue; it is not waited for.
pub fn fill_rgba16f(
    ctx: &GpuContext,
    texture: &wgpu::Texture,
    color: [f32; 4],
) -> Result<(), GpuError> {
    debug_assert_eq!(texture.format(), wgpu::TextureFormat::Rgba16Float);
    let device = &ctx.device;
    let module = ctx.create_shader_module("gpu/fill")?;

    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("fill"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba16Float,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("fill"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("fill"),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });

    let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("fill params"),
        contents: bytemuck::cast_slice(&color),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("fill"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
        ],
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("fill"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("fill"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        // A constant fill is far below the driver timeout even at 8K, so a single
        // dispatch is fine here. Heavier passes must be split into tiles.
        pass.dispatch_workgroups(
            texture.width().div_ceil(WORKGROUP_SIZE),
            texture.height().div_ceil(WORKGROUP_SIZE),
            1,
        );
    }
    ctx.queue.submit([encoder.finish()]);
    Ok(())
}
