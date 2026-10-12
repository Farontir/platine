//! The procedural materials on the GPU against their CPU reference, texel by texel, for
//! every output channel, in UV and triplanar projection on a cube and on the flat
//! surface of the thumbnails.

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

fn target(ctx: &GpuContext) -> wgpu::Texture {
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
        format: wgpu::TextureFormat::Rgba32Float,
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
                for output in &def.outputs {
                    let index = output.index().expect("materials produce standard channels");
                    let uniforms = EvaluationUniforms::new(&projection, SIZE, buffers.center, buffers.extent, flat, index);
                    let surface = if flat {
                        SurfaceInput::Flat
                    } else {
                        SurfaceInput::Mesh { mesh: &buffers, material: 0, map: &map_view }
                    };
                    let texture = target(&ctx);
                    let mut encoder = ctx.device.create_command_encoder(&Default::default());
                    procedurals
                        .evaluate(&ctx, &mut encoder, def, &params, &uniforms, surface, &texture.create_view(&Default::default()), wgpu::TextureFormat::Rgba32Float)
                        .expect("evaluates");
                    ctx.queue.submit([encoder.finish()]);
                    let bytes = readback::read_texture(&ctx, &texture).expect("readback");
                    let gpu: &[[f32; 4]] = bytemuck::cast_slice(&bytes);
                    let mut worst = 0.0f32;
                    for y in 0..SIZE {
                        for x in 0..SIZE {
                            let i = (y * SIZE + x) as usize;
                            let s = if flat {
                                flat_surface([x, y], SIZE as f32)
                            } else {
                                surface_at(&vertices, &indices, entries[i], [x, y], SIZE as f32)
                            };
                            let cpu = evaluate_material(&def.id, &params, &uniforms, &s);
                            for c in 0..4 {
                                worst = worst.max((gpu[i][c] - cpu[c]).abs());
                            }
                        }
                    }
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
