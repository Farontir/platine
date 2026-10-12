//! Surface maps, projections and gray procedurals on meshes: GPU against the CPU
//! reference, and M7 acceptance criterion 2 (no seam with the triplanar projection on
//! the test sphere).

use std::collections::HashMap;

use patine_core::layer::{Projection, ProjectionKind};
use patine_core::mesh::MeshAsset;
use patine_gpu::{GpuContext, gpu_test, readback, wgpu};
use patine_mesh::primitives;
use patine_procedural::evaluate::{EvaluationUniforms, Procedurals, SurfaceInput};
use patine_procedural::library;
use patine_procedural::reference::Vec2;
use patine_procedural::reference::gray::evaluate_gray;
use patine_procedural::reference::surface::{flat_surface, surface_at};
use patine_procedural::surface::{
    MeshBuffers, SurfaceBuilder, gpu_vertices, material_triangles, surface_descriptor,
};

/// The surface map of material 0 of `mesh` at `size`², read back.
fn surface_map(ctx: &GpuContext, mesh: &MeshBuffers, size: u32) -> (wgpu::Texture, Vec<u32>) {
    let builder = SurfaceBuilder::new(ctx).expect("shaders compile");
    let target = ctx.device.create_texture(&surface_descriptor(size));
    let mut encoder = ctx.device.create_command_encoder(&Default::default());
    builder.build(ctx, &mut encoder, mesh, 0, &target);
    ctx.queue.submit([encoder.finish()]);
    let bytes = readback::read_texture(ctx, &target).expect("readback");
    (target, bytemuck::cast_slice(&bytes).to_vec())
}

/// Evaluates the gray procedural `id` (default parameters) into an `R32Float` texture.
fn evaluate(
    ctx: &GpuContext,
    procedurals: &Procedurals,
    id: &str,
    uniforms: &EvaluationUniforms,
    surface: SurfaceInput<'_>,
) -> Vec<f32> {
    let def = library::find(id).expect("shipped");
    evaluate_with(ctx, procedurals, id, &def.defaults(), uniforms, surface)
}

/// Evaluates the gray procedural `id` with `params` into an `R32Float` texture.
fn evaluate_with(
    ctx: &GpuContext,
    procedurals: &Procedurals,
    id: &str,
    params: &patine_core::mask::Params,
    uniforms: &EvaluationUniforms,
    surface: SurfaceInput<'_>,
) -> Vec<f32> {
    let def = library::find(id).expect("shipped");
    let size = uniforms.size as u32;
    let target = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gray"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut encoder = ctx.device.create_command_encoder(&Default::default());
    procedurals
        .evaluate(
            ctx,
            &mut encoder,
            def,
            params,
            uniforms,
            surface,
            &[(
                &target.create_view(&Default::default()),
                wgpu::TextureFormat::R32Float,
            )],
        )
        .expect("evaluates");
    ctx.queue.submit([encoder.finish()]);
    let bytes = readback::read_texture(ctx, &target).expect("readback");
    bytemuck::cast_slice(&bytes).to_vec()
}

/// Barycentric coordinates of `p` in triangle (a, b, c), `None` if degenerate.
fn barycentric(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> Option<[f32; 3]> {
    let (v0, v1, v2) = (b - a, c - a, p - a);
    let d = v0.x * v1.y - v1.x * v0.y;
    if d.abs() < 1e-12 {
        return None;
    }
    let v = (v2.x * v1.y - v1.x * v2.y) / d;
    let w = (v0.x * v2.y - v2.x * v0.y) / d;
    Some([1.0 - v - w, v, w])
}

/// Distance from `p` to triangle (a, b, c), in UV units.
fn distance(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> f32 {
    let w = patine_procedural::reference::surface::closest_barycentric(p, a, b, c);
    (p - (a * w.x + b * w.y + c * w.z)).length()
}

fn uv_triangles(mesh: &MeshAsset) -> Vec<[Vec2; 3]> {
    let uv = mesh.uv0.as_ref().expect("the test meshes have UVs");
    material_triangles(mesh, 0)
        .iter()
        .map(|t| t.map(|i| Vec2::from(uv[i as usize])))
        .collect()
}

gpu_test!(
    /// Covered texels name a triangle containing their center; the others name a
    /// triangle within 1.5 texels of the nearest one.
    fn surface_maps_cover_and_extend_the_islands(ctx) {
        for (name, mesh) in [("sphere", primitives::uv_sphere(16, 8)), ("cube", primitives::cube_cross())] {
            let size = 64;
            let buffers = MeshBuffers::new(&ctx, &mesh);
            let (_, map) = surface_map(&ctx, &buffers, size);
            let triangles = uv_triangles(&mesh);
            let (mut covered, mut extended) = (0, 0);
            for y in 0..size {
                for x in 0..size {
                    let entry = map[(y * size + x) as usize];
                    assert_ne!(entry, 0, "{name}: texel ({x}, {y}) has no triangle");
                    let p = (Vec2::new(x as f32, y as f32) + 0.5) / size as f32;
                    let [a, b, c] = triangles[(entry - 1) as usize];
                    let inside = triangles.iter().any(|[a, b, c]| {
                        barycentric(p, *a, *b, *c).is_some_and(|w| w.iter().all(|v| *v > 1e-5))
                    });
                    if inside {
                        covered += 1;
                        let w = barycentric(p, a, b, c).expect("a covering triangle is not flat");
                        assert!(w.iter().all(|v| *v > -1e-4), "{name}: ({x}, {y}) is not in its triangle: {w:?}");
                    } else {
                        extended += 1;
                        let nearest = triangles
                            .iter()
                            .map(|[a, b, c]| distance(p, *a, *b, *c))
                            .fold(f32::MAX, f32::min);
                        let chosen = distance(p, a, b, c);
                        assert!(
                            chosen <= nearest + 1.5 / size as f32,
                            "{name}: ({x}, {y}) chose a triangle at {chosen}, the nearest is at {nearest}"
                        );
                    }
                }
            }
            assert!(covered > 0 && extended > 0, "{name}: {covered} covered, {extended} extended");
        }
    }
);

fn projections() -> Vec<(&'static str, Projection)> {
    let base = Projection {
        scale: [2.0, 2.0],
        ..Projection::default()
    };
    vec![
        ("uv", base),
        (
            "uv rotated",
            Projection {
                rotation: 0.6,
                offset: [0.3, -0.2],
                scale: [3.0, 1.5],
                ..base
            },
        ),
        (
            "triplanar",
            Projection {
                kind: ProjectionKind::Triplanar { blend: 0.4 },
                ..base
            },
        ),
        (
            "planar",
            Projection {
                kind: ProjectionKind::Planar,
                ..base
            },
        ),
        (
            "spherical",
            Projection {
                kind: ProjectionKind::Spherical,
                ..base
            },
        ),
        (
            "cylindrical",
            Projection {
                kind: ProjectionKind::Cylindrical,
                ..base
            },
        ),
    ]
}

gpu_test!(
    /// Every projection, on the cube and on the sphere, and the flat surface: the GPU
    /// matches the CPU reference texel by texel (from the GPU's surface map).
    fn projections_match_the_reference(ctx) {
        let procedurals = Procedurals::new(&ctx);
        let size = 64u32;
        let mut failures = Vec::new();
        for (mesh_name, mesh) in [("cube", primitives::cube_cross()), ("sphere", primitives::uv_sphere(24, 12))] {
            let buffers = MeshBuffers::new(&ctx, &mesh);
            let (map_texture, map) = surface_map(&ctx, &buffers, size);
            let map_view = map_texture.create_view(&Default::default());
            let vertices = gpu_vertices(&mesh);
            let indices = material_triangles(&mesh, 0);
            for (projection_name, projection) in projections() {
                for id in ["noise.perlin", "noise.fbm"] {
                    let def = library::find(id).expect("shipped");
                    let uniforms = EvaluationUniforms::new(&projection, size, buffers.center, buffers.extent, false);
                    let surface = SurfaceInput::Mesh { mesh: &buffers, material: 0, map: &map_view };
                    let gpu = evaluate(&ctx, &procedurals, id, &uniforms, surface);
                    let mut worst = 0.0f32;
                    for y in 0..size {
                        for x in 0..size {
                            let i = (y * size + x) as usize;
                            let s = surface_at(&vertices, &indices, map[i], [x, y], size as f32);
                            let cpu = evaluate_gray(def, &def.defaults(), &uniforms, &s);
                            worst = worst.max((gpu[i] - cpu).abs());
                        }
                    }
                    if worst > 1e-3 {
                        failures.push(format!("{mesh_name} {projection_name} {id}: {worst}"));
                    }
                }
            }
        }
        // The flat surface of the thumbnails.
        let def = library::find("pattern.bricks").expect("shipped");
        let uniforms = EvaluationUniforms::new(&Projection::default(), size, [0.0; 3], 1.0, true);
        let gpu = evaluate(&ctx, &procedurals, "pattern.bricks", &uniforms, SurfaceInput::Flat);
        for y in 0..size {
            for x in 0..size {
                let cpu = evaluate_gray(def, &def.defaults(), &uniforms, &flat_surface([x, y], size as f32));
                let g = gpu[(y * size + x) as usize];
                if (g - cpu).abs() > 1e-3 {
                    failures.push(format!("flat bricks ({x}, {y}): gpu {g}, cpu {cpu}"));
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }
);

/// Bilinear sample of a `size`² texture at `uv`, edges clamped.
fn bilinear(texels: &[f32], size: u32, uv: Vec2) -> f32 {
    let x = uv.x * size as f32 - 0.5;
    let y = uv.y * size as f32 - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |x: f32, y: f32| {
        let xi = (x as i32).clamp(0, size as i32 - 1);
        let yi = (y as i32).clamp(0, size as i32 - 1);
        texels[(yi as u32 * size + xi as u32) as usize]
    };
    let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1.0, y0) * fx;
    let bottom = at(x0, y0 + 1.0) * (1.0 - fx) + at(x0 + 1.0, y0 + 1.0) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// Edges of the UV seams: the same edge in space, with different UVs on its two sides.
fn seam_edges(mesh: &MeshAsset) -> Vec<([Vec2; 2], [Vec2; 2])> {
    let uv = mesh.uv0.as_ref().expect("UVs");
    let key = |i: u32| mesh.positions[i as usize].map(|v| (v * 1e4).round() as i64);
    type Key = ([i64; 3], [i64; 3]);
    let mut edges: HashMap<Key, Vec<[Vec2; 2]>> = HashMap::new();
    for t in material_triangles(mesh, 0) {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let (ka, kb) = (key(a), key(b));
            if ka == kb {
                continue;
            }
            let (ka, kb, a, b) = if ka < kb {
                (ka, kb, a, b)
            } else {
                (kb, ka, b, a)
            };
            edges
                .entry((ka, kb))
                .or_default()
                .push([Vec2::from(uv[a as usize]), Vec2::from(uv[b as usize])]);
        }
    }
    edges
        .into_values()
        .filter(|sides| {
            sides.len() == 2
                && (sides[0][0] - sides[1][0]).length() + (sides[0][1] - sides[1][1]).length()
                    > 1e-4
        })
        .map(|sides| (sides[0], sides[1]))
        .collect()
}

/// Largest difference between the two sides of the seams (with where it happens), and
/// largest difference between neighboring texels showing the same triangle (the image's
/// own variation per texel; texels of different triangles are skipped, since around the
/// islands neighbors may extend different triangles, which the mesh never shows).
fn seam_and_step(
    texels: &[f32],
    map: &[u32],
    size: u32,
    seams: &[([Vec2; 2], [Vec2; 2])],
) -> (f32, Vec2, f32) {
    let mut seam = 0.0f32;
    let mut at = Vec2::ZERO;
    for (a, b) in seams {
        for i in 1..10 {
            let t = i as f32 / 10.0;
            let pa = a[0] + (a[1] - a[0]) * t;
            let pb = b[0] + (b[1] - b[0]) * t;
            let d = (bilinear(texels, size, pa) - bilinear(texels, size, pb)).abs();
            if d > seam {
                seam = d;
                at = pa;
            }
        }
    }
    let mut step = 0.0f32;
    for y in 0..size - 1 {
        for x in 0..size - 1 {
            let i = (y * size + x) as usize;
            for j in [i + 1, i + size as usize] {
                if map[i] == map[j] {
                    step = step.max((texels[j] - texels[i]).abs());
                }
            }
        }
    }
    (seam, at, step)
}

gpu_test!(
    /// M7 acceptance criterion 2: Perlin noise in triplanar projection on the test sphere
    /// (512²) takes the same values on both sides of every UV seam, within the largest
    /// change between neighboring texels (bilinear sampling on each side). The same noise
    /// in UV projection with a scale that does not tile breaks at the seam, by many times
    /// that change: the test does detect seams.
    fn triplanar_shows_no_seam_on_the_sphere(ctx) {
        let procedurals = Procedurals::new(&ctx);
        let mesh = primitives::uv_sphere(32, 16);
        let seams = seam_edges(&mesh);
        assert!(!seams.is_empty(), "the test sphere has UV seams");
        let size = 512;
        let buffers = MeshBuffers::new(&ctx, &mesh);
        let (map_texture, map_texels) = surface_map(&ctx, &buffers, size);
        let map = map_texture.create_view(&Default::default());
        let surface = SurfaceInput::Mesh { mesh: &buffers, material: 0, map: &map };
        // Two noise cells per projection unit.
        let mut params = library::find("noise.perlin").expect("shipped").defaults();
        params.insert("scale".into(), patine_core::mask::ParamValue::Int(2));
        let run = |projection: Projection| {
            let uniforms = EvaluationUniforms::new(&projection, size, buffers.center, buffers.extent, false);
            let texels = evaluate_with(&ctx, &procedurals, "noise.perlin", &params, &uniforms, surface);
            seam_and_step(&texels, &map_texels, size, &seams)
        };
        let (seam, at, step) = run(Projection {
            kind: ProjectionKind::Triplanar { blend: 0.5 },
            ..Projection::default()
        });
        println!("triplanar: seam {seam} at {at}, neighbor step {step}");
        assert!(seam <= step, "triplanar seam {seam} for a neighbor step of {step}");
        let (control, control_at, control_step) = run(Projection {
            scale: [1.5, 1.5],
            ..Projection::default()
        });
        println!("uv x1.5: seam {control} at {control_at}, neighbor step {control_step}");
        assert!(control > 4.0 * control_step, "the control seam {control} is not detected (step {control_step})");
    }
);
