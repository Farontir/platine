//! The WGSL display operators against reference::tone_map, through a probe shader.

use glam::Vec3;
use patine_gpu::{gpu_test, wgpu};
use patine_render::reference::{srgb_encode, tone_map};
use patine_render::settings::ToneMapper;
use wgpu::util::DeviceExt as _;

/// HDR ramp over 20 EV plus saturated and colored values.
fn inputs() -> Vec<Vec3> {
    let mut colors: Vec<Vec3> = (0..64)
        .map(|i| Vec3::splat((i as f32 / 63.0 * 20.0 - 12.0).exp2()))
        .collect();
    for scale in [0.05, 0.5, 2.0, 16.0, 200.0] {
        colors.push(Vec3::new(1.0, 0.2, 0.05) * scale);
        colors.push(Vec3::new(0.1, 0.6, 1.0) * scale);
        colors.push(Vec3::new(0.0, 0.0, 1.0) * scale);
    }
    colors
}

gpu_test!(
    fn display_operators_match_the_reference(ctx) {
        let colors = inputs();
        let input: Vec<[f32; 4]> = colors.iter().map(|c| c.extend(1.0).to_array()).collect();
        let device = &ctx.device;
        let input_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("probe inputs"),
            contents: bytemuck::cast_slice(&input),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let output_size = (input.len() * 4 * 16) as u64;
        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("probe outputs"),
            size: output_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("probe staging"),
            size: output_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let module = ctx.create_shader_module("tests/tonemap_probe").unwrap();
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("probe"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output_buffer.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups((input.len() as u32).div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output_buffer, 0, &staging, 0, output_size);
        ctx.queue.submit([encoder.finish()]);
        staging.map_async(wgpu::MapMode::Read, .., |result| result.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mapped = staging.get_mapped_range(..).unwrap();
        let outputs: &[[f32; 4]] = bytemuck::cast_slice(&mapped);

        for (i, color) in colors.iter().enumerate() {
            for (op, operator) in ToneMapper::ALL.into_iter().enumerate() {
                let expected = srgb_encode(tone_map(operator, *color));
                let actual = Vec3::from_slice(&outputs[i * 4 + op][..3]);
                // Half an 8-bit display step.
                assert!(
                    (actual - expected).abs().max_element() < 0.5 / 255.0,
                    "{operator:?}({color}): GPU {actual} vs CPU {expected}"
                );
            }
        }
    }
);
