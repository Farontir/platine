//! The procedural materials on the GPU against their CPU reference, texel by texel, for
//! every output channel, in UV and triplanar projection on a cube and on the flat
//! surface of the thumbnails.
//!
//! The pass writes the five outputs at once, which bounds the bytes per texel of its
//! targets (32 by default): colors and normals go to `Rgba16Float` as in the compositor,
//! scalars to `R32Float`; the reference is rounded to f16 where the target is.

use patine_core::layer::{Projection, ProjectionKind};
use patine_core::mask::ParamValue;
use patine_gpu::{GpuContext, gpu_test, readback, wgpu};
use patine_mesh::primitives;
use patine_procedural::def::{Category, Output};
use patine_procedural::evaluate::{EvaluationUniforms, Procedurals, SurfaceInput};
use patine_procedural::library;
use patine_procedural::reference::material::evaluate_material;
use patine_procedural::reference::surface::{flat_surface, surface_at};
use patine_procedural::surface::{
    MeshBuffers, SurfaceBuilder, gpu_vertices, material_triangles, surface_descriptor,
};

const SIZE: u32 = 48;

/// Format of output `index`: `Rgba16Float` for colors and normals, `R32Float` otherwise.
fn format(index: usize) -> wgpu::TextureFormat {
    match index {
        0 | 3 => wgpu::TextureFormat::Rgba16Float,
        _ => wgpu::TextureFormat::R32Float,
    }
}

fn target(ctx: &GpuContext, format: wgpu::TextureFormat) -> wgpu::Texture {
    ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("material"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

gpu_test!(
    fn materials_match_their_reference(ctx) {
        let procedurals = Procedurals::new(&ctx);
        let mesh = primitives::cube_cross();
        let buffers = MeshBuffers::new(&ctx, &mesh);
        let map = ctx.device.create_texture(&surface_descriptor(SIZE));
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        SurfaceBuilder::new(&ctx).expect("shaders compile").build(&ctx, &mut encoder, &buffers, 0, &map);
        ctx.queue.submit([encoder.finish()]);
        let entries: Vec<u32> =
            bytemuck::cast_slice(&readback::read_texture(&ctx, &map).expect("readback")).to_vec();
        let map_view = map.create_view(&Default::default());
        let vertices = gpu_vertices(&mesh);
        let indices = material_triangles(&mesh, 0);
        let triplanar = Projection {
            kind: ProjectionKind::Triplanar { blend: 0.6 },
            ..Projection::default()
        };
        let mut failures = Vec::new();
        let materials = library::all().iter().filter(|def| def.category == Category::Material);
        for def in materials {
            // Defaults, with some oxidation for the copper to cover its second layer.
            let mut params = def.defaults();
            if def.id == "material.copper" {
                params.insert("oxidation".into(), ParamValue::Float(0.4));
            }
            for (name, projection, flat) in [
                ("uv", Projection::default(), false),
                ("triplanar", triplanar, false),
                ("flat", Projection::default(), true),
            ] {
                let uniforms = EvaluationUniforms::new(&projection, SIZE, buffers.center, buffers.extent, flat);
                let surface = if flat {
                    SurfaceInput::Flat
                } else {
                    SurfaceInput::Mesh { mesh: &buffers, material: 0, map: &map_view }
                };
                let textures: Vec<wgpu::Texture> = (0..def.outputs.len()).map(|i| target(&ctx, format(i))).collect();
                let views: Vec<wgpu::TextureView> = textures.iter().map(|t| t.create_view(&Default::default())).collect();
                let targets: Vec<_> = views.iter().enumerate().map(|(i, v)| (v, format(i))).collect();
                let mut encoder = ctx.device.create_command_encoder(&Default::default());
                procedurals
                    .evaluate(&ctx, &mut encoder, def, &params, &uniforms, surface, &targets)
                    .expect("evaluates");
                ctx.queue.submit([encoder.finish()]);
                let gpu: Vec<Vec<[f32; 4]>> = textures
                    .iter()
                    .map(|t| match t.format() {
                        wgpu::TextureFormat::Rgba16Float => readback::read_rgba16f(&ctx, t).expect("readback"),
                        _ => bytemuck::cast_slice::<u8, f32>(&readback::read_texture(&ctx, t).expect("readback"))
                            .iter()
                            .map(|v| [*v, 0.0, 0.0, 0.0])
                            .collect(),
                    })
                    .collect();
                let mut worst = [0.0f32; 5];
                for y in 0..SIZE {
                    for x in 0..SIZE {
                        let i = (y * SIZE + x) as usize;
                        let s = if flat {
                            flat_surface([x, y], SIZE as f32)
                        } else {
                            surface_at(&vertices, &indices, entries[i], [x, y], SIZE as f32)
                        };
                        let cpu = evaluate_material(&def.id, &params, &uniforms, &s);
                        for (output, values) in gpu.iter().enumerate() {
                            let half = format(output) == wgpu::TextureFormat::Rgba16Float;
                            for c in 0..4 {
                                let expected = if half {
                                    half::f16::from_f32(cpu[output][c]).to_f32()
                                } else {
                                    cpu[output][c]
                                };
                                worst[output] = worst[output].max((values[i][c] - expected).abs());
                            }
                        }
                    }
                }
                for (output, worst) in def.outputs.iter().zip(worst) {
                    assert_eq!(output.index().map(|i| i as usize), def.outputs.iter().position(|o| o == output));
                    if worst > 1e-3 {
                        failures.push(format!("{} {name} {output:?}: {worst}", def.id));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
        assert_eq!(Output::Gray.index(), Some(0));
    }
);
