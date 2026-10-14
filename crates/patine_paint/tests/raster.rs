//! The paint passes on the test adapter: depth against ray casting, dabs and composition
//! against the CPU reference, live painting against replay (M8 criterion 2), and a stroke
//! across a UV seam of the cube without holes (M8 criterion 1). From M9: dab dynamics,
//! symmetry copies and UV strokes against the reference, exact replay of jittered strokes
//! (M9 criterion 3), exact symmetry on the sphere (M9 criterion 2), values read from a
//! texture.

use glam::{Mat4, Vec2, Vec3, Vec4};
use patine_core::channel::Channel;
use patine_core::layer::FillSource;
use patine_core::mesh::MeshAsset;
use patine_core::stroke::{
    BrushSnapshot, Curve, Dab, PressureCurves, SizeSpace, Stroke, StrokeCamera, StrokeSpace,
    Symmetry, Tool,
};
use patine_gpu::pool::TexturePool;
use patine_gpu::{GpuContext, gpu_test, readback, wgpu};
use patine_mesh::bvh::{Bvh, Ray};
use patine_mesh::primitives;
use patine_paint::alphas;
use patine_paint::dynamics::gpu_dabs;
use patine_paint::projection::{GpuDab, SetGeometry, StrokeProjection};
use patine_paint::raster::{Content, LiveStroke, Painter, SetSurface, content_descriptor};
use patine_paint::reference::{
    ContentLayout, DepthImage, deposit, erase, stroke_texel, texel_view, texel_view_uv,
};
use patine_procedural::reference::surface::{Surface, surface_at};
use patine_procedural::surface::{
    GpuVertex, MeshBuffers, SurfaceBuilder, gpu_vertices, material_triangles, surface_descriptor,
};

const VIEWPORT: [u32; 2] = [256, 256];

/// Right-handed view matrix (the viewport camera's convention).
fn look(eye: Vec3, center: Vec3) -> Mat4 {
    let forward = (center - eye).normalize();
    let side = forward.cross(Vec3::Y).normalize();
    let up = side.cross(forward);
    Mat4::from_cols(
        Vec4::new(side.x, up.x, -forward.x, 0.0),
        Vec4::new(side.y, up.y, -forward.y, 0.0),
        Vec4::new(side.z, up.z, -forward.z, 0.0),
        Vec4::new(-side.dot(eye), -up.dot(eye), forward.dot(eye), 1.0),
    )
}

/// Infinite reverse-Z perspective (the viewport camera's projection).
fn perspective(fov_y: f32, aspect: f32, near: f32) -> Mat4 {
    let f = 1.0 / (fov_y * 0.5).tan();
    Mat4::from_cols(
        Vec4::new(f / aspect, 0.0, 0.0, 0.0),
        Vec4::new(0.0, f, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 0.0, -1.0),
        Vec4::new(0.0, 0.0, near, 0.0),
    )
}

fn camera(eye: Vec3) -> StrokeCamera {
    let aspect = VIEWPORT[0] as f32 / VIEWPORT[1] as f32;
    StrokeCamera {
        view_proj: (perspective(0.8, aspect, 0.05) * look(eye, Vec3::ZERO)).to_cols_array_2d(),
        viewport: VIEWPORT,
    }
}

fn brush(
    size: f32,
    hardness: f32,
    alpha: Option<patine_core::resource::ResourceHash>,
) -> BrushSnapshot {
    BrushSnapshot {
        size,
        size_space: SizeSpace::Screen,
        flow: 0.6,
        opacity: 0.9,
        hardness,
        spacing: 0.1,
        angle: 0.4,
        size_jitter: 0.0,
        angle_jitter: 0.0,
        flow_jitter: 0.0,
        scatter: 0.0,
        follow_path: false,
        alpha,
        material: FillSource::Uniform(
            [
                (Channel::BaseColor, [0.9, 0.2, 0.1, 1.0]),
                (Channel::Roughness, [0.3; 4]),
            ]
            .into(),
        ),
        pressure: patine_core::stroke::PressureCurves::none(),
        tilt_angle: false,
        projection: BrushSnapshot::default_projection(),
    }
}

/// A stroke along `points` (viewport pixels), one dab every 3 pixels.
fn stroke(camera: StrokeCamera, brush: BrushSnapshot, tool: Tool, points: &[[f32; 2]]) -> Stroke {
    let mut dabs = Vec::new();
    for pair in points.windows(2) {
        let (a, b) = (Vec2::from(pair[0]), Vec2::from(pair[1]));
        let steps = (a.distance(b) / 3.0).ceil().max(1.0) as usize;
        for i in 0..steps {
            let p = a.lerp(b, i as f32 / steps as f32);
            dabs.push(Dab {
                position: p.to_array(),
                pressure: 1.0,
                tilt: [0.0; 2],
                time: dabs.len() as f32 / 60.0,
            });
        }
    }
    Stroke {
        brush,
        tool,
        camera,
        space: patine_core::stroke::StrokeSpace::View,
        symmetry: Symmetry::default(),
        seed: 1,
        dabs,
    }
}

/// A mesh on the GPU with the surface map of its first material.
struct Fixture {
    mesh: MeshAsset,
    buffers: MeshBuffers,
    map: wgpu::Texture,
    entries: Vec<u32>,
    geometry: SetGeometry,
    vertices: Vec<GpuVertex>,
    triangles: Vec<[u32; 3]>,
    size: u32,
}

impl Fixture {
    fn new(ctx: &GpuContext, mesh: MeshAsset, size: u32) -> Self {
        let buffers = MeshBuffers::new(ctx, &mesh);
        let map = ctx.device.create_texture(&surface_descriptor(size));
        let builder = SurfaceBuilder::new(ctx).expect("shaders compile");
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        builder.build(ctx, &mut encoder, &buffers, 0, &map);
        ctx.queue.submit([encoder.finish()]);
        let entries =
            bytemuck::cast_slice(&readback::read_texture(ctx, &map).expect("readback")).to_vec();
        Self {
            geometry: SetGeometry::new(&mesh, 0),
            vertices: gpu_vertices(&mesh),
            triangles: material_triangles(&mesh, 0),
            mesh,
            buffers,
            map,
            entries,
            size,
        }
    }

    fn surface<'a>(&'a self, view: &'a wgpu::TextureView) -> SetSurface<'a> {
        SetSurface {
            mesh: &self.buffers,
            material: 0,
            surface_map: view,
            geometry: &self.geometry,
            size: self.size,
        }
    }

    fn surface_at(&self, texel: [u32; 2]) -> Surface {
        let entry = self.entries[(texel[1] * self.size + texel[0]) as usize];
        surface_at(
            &self.vertices,
            &self.triangles,
            entry,
            texel,
            self.size as f32,
        )
    }
}

fn read_f32(ctx: &GpuContext, texture: &wgpu::Texture) -> Vec<f32> {
    bytemuck::cast_slice(&readback::read_texture(ctx, texture).expect("readback")).to_vec()
}

/// A content texture as `[f32; 4]` texels (scalar contents in the first two).
fn read_content(ctx: &GpuContext, texture: &wgpu::Texture, layout: ContentLayout) -> Vec<[f32; 4]> {
    match layout {
        ContentLayout::Vector => readback::read_rgba16f(ctx, texture).expect("readback"),
        ContentLayout::Scalar => readback::read_texture(ctx, texture)
            .expect("readback")
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| {
                let v = |i: usize| half::f16::from_le_bytes([b[i], b[i + 1]]).to_f32();
                [v(0), v(2), 0.0, 0.0]
            })
            .collect(),
    }
}

fn content(
    ctx: &GpuContext,
    layout: ContentLayout,
    size: u32,
    texels: &[[f32; 4]],
) -> wgpu::Texture {
    let texture = ctx.device.create_texture(&content_descriptor(layout, size));
    let components = match layout {
        ContentLayout::Vector => 4,
        ContentLayout::Scalar => 2,
    };
    let bytes: Vec<u8> = texels
        .iter()
        .flat_map(|t| {
            t[..components]
                .iter()
                .flat_map(|v| half::f16::from_f32(*v).to_le_bytes())
                .collect::<Vec<_>>()
        })
        .collect();
    ctx.queue.write_texture(
        texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size * components as u32 * 2),
            rows_per_image: None,
        },
        texture.size(),
    );
    texture
}

fn f16(v: f32) -> f32 {
    half::f16::from_f32(v).to_f32()
}

gpu_test!(
    /// The depth image holds, at every pixel the mesh covers, the distance of the first
    /// surface the pixel's ray meets (away from silhouettes).
    fn depth_matches_ray_casting(ctx) {
        let fixture = Fixture::new(&ctx, primitives::uv_sphere(48, 24), 256);
        let painter = Painter::new(&ctx).expect("shaders compile");
        let mut pool = TexturePool::new(u64::MAX);
        let view = fixture.map.create_view(&Default::default());
        let camera = camera(Vec3::new(0.5, 1.2, 3.0));
        let stroke = stroke(camera, brush(10.0, 0.5, None), Tool::Brush, &[[0.0, 0.0]]);
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        let target = painter.begin_stroke(&ctx, &mut pool, &mut encoder, &stroke, &fixture.surface(&view), None);
        ctx.queue.submit([encoder.finish()]);
        let depth = read_f32(&ctx, target.depth_texture());
        let projection = StrokeProjection::new(&camera);
        let positions = &fixture.mesh.positions;
        let bvh = Bvh::build(positions, &fixture.mesh.triangles);
        let inverse = projection.view_proj.inverse();
        let (mut compared, mut worst) = (0, 0.0f32);
        for y in (0..VIEWPORT[1]).step_by(3) {
            for x in (0..VIEWPORT[0]).step_by(3) {
                let ndc = Vec2::new(
                    (x as f32 + 0.5) / VIEWPORT[0] as f32 * 2.0 - 1.0,
                    1.0 - (y as f32 + 0.5) / VIEWPORT[1] as f32 * 2.0,
                );
                let near = inverse.project_point3(ndc.extend(1.0));
                let far = inverse.project_point3(ndc.extend(0.5));
                let ray = Ray { origin: near, direction: far - near };
                let gpu = depth[(y * VIEWPORT[0] + x) as usize];
                let Some(hit) = bvh.closest_hit(positions, &fixture.mesh.triangles, &ray, f32::INFINITY) else {
                    continue;
                };
                // Away from the silhouette: the neighbors see the mesh too.
                let around = [(-2i32, 0i32), (2, 0), (0, -2), (0, 2)].iter().all(|(dx, dy)| {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    nx >= 0 && ny >= 0 && (nx as u32) < VIEWPORT[0] && (ny as u32) < VIEWPORT[1]
                        && depth[(ny as u32 * VIEWPORT[0] + nx as u32) as usize] < f32::MAX
                });
                if !around {
                    continue;
                }
                let point = ray.origin + ray.direction * hit.t;
                let expected = projection.depth(projection.clip(point));
                worst = worst.max((gpu - expected).abs() / expected);
                compared += 1;
            }
        }
        assert!(compared > 1000, "{compared} pixels compared");
        // The sphere is faceted the same way on both sides; only rounding differs.
        assert!(worst < 1e-3, "worst relative error {worst}");
        // The background stays at f32::MAX.
        assert_eq!(depth[0], f32::MAX);
        painter.end_stroke(&mut pool, target);
    }
);

/// Runs `stroke` live in one batch on a fresh stroke buffer; returns the buffer, the
/// depth image and the tiles.
fn rasterize(
    ctx: &GpuContext,
    painter: &Painter,
    fixture: &Fixture,
    view: &wgpu::TextureView,
    stroke: &Stroke,
    alpha: Option<&alphas::AlphaImage>,
) -> (Vec<f32>, DepthImage, Vec<[u32; 2]>) {
    let mut pool = TexturePool::new(u64::MAX);
    let set = fixture.surface(view);
    let mut encoder = ctx.device.create_command_encoder(&Default::default());
    let target = painter.begin_stroke(ctx, &mut pool, &mut encoder, stroke, &set, alpha);
    let dabs: Vec<GpuDab> = gpu_dabs(stroke, 0..stroke.dabs.len());
    let tiles = fixture
        .geometry
        .touched_tiles(&target.projections, &dabs, fixture.size);
    painter.rasterize_dabs(ctx, &pool, &mut encoder, &target, &set, &dabs, &tiles);
    ctx.queue.submit([encoder.finish()]);
    let buffer = read_f32(ctx, pool.peek(target.buffer).expect("pinned"));
    let depth = DepthImage {
        width: VIEWPORT[0],
        height: VIEWPORT[1],
        values: read_f32(ctx, target.depth_texture()),
    };
    painter.end_stroke(&mut pool, target);
    (buffer, depth, tiles)
}

gpu_test!(
    /// The stroke buffer matches the CPU reference texel by texel, for round dabs of
    /// several hardnesses and for a rotated alpha, on a sphere with occlusion.
    fn dabs_match_the_reference(ctx) {
        let fixture = Fixture::new(&ctx, primitives::uv_sphere(48, 24), 256);
        let painter = Painter::new(&ctx).expect("shaders compile");
        let view = fixture.map.create_view(&Default::default());
        let camera = camera(Vec3::new(0.5, 1.2, 3.0));
        let splatter = alphas::find("alpha.splatter").expect("shipped").image();
        let cases = [
            (brush(40.0, 0.0, None), None),
            (brush(24.0, 1.0, None), None),
            (brush(60.0, 0.5, None), Some(&splatter)),
        ];
        let path = [[40.0, 60.0], [128.0, 140.0], [220.0, 120.0]];
        for (brush, alpha) in cases {
            let stroke = stroke(camera, brush, Tool::Brush, &path);
            let (gpu, depth, tiles) = rasterize(&ctx, &painter, &fixture, &view, &stroke, alpha);
            let projection = StrokeProjection::new(&camera);
            let dabs: Vec<GpuDab> = gpu_dabs(&stroke, 0..stroke.dabs.len());
            let (mut worst, mut painted, mut flipped) = (0.0f32, 0, 0);
            for tile in &tiles {
                for y in tile[1] * 128..((tile[1] + 1) * 128).min(fixture.size) {
                    for x in tile[0] * 128..((tile[0] + 1) * 128).min(fixture.size) {
                        let surface = fixture.surface_at([x, y]);
                        let view = texel_view(&projection, &depth, fixture.buffers.extent, &surface, [x, y], fixture.size);
                        let expected = stroke_texel(0.0, &[view], &dabs, alpha);
                        let got = gpu[(y * fixture.size + x) as usize];
                        let error = (got - expected).abs();
                        if error > 1e-3 {
                            // A texel exactly on a step (occlusion, grazing cut) may
                            // fall on either side.
                            flipped += 1;
                        } else {
                            worst = worst.max(error);
                        }
                        painted += usize::from(expected > 0.01);
                    }
                }
            }
            assert!(painted > 500, "the stroke paints ({painted} texels)");
            assert!(worst < 1e-4, "worst error {worst}");
            assert!(flipped * 1000 < painted, "{flipped} texels differ out of {painted}");
            // Nothing outside the touched tiles.
            for y in 0..fixture.size {
                for x in 0..fixture.size {
                    if !tiles.contains(&[x / 128, y / 128]) {
                        assert_eq!(gpu[(y * fixture.size + x) as usize], 0.0);
                    }
                }
            }
        }
    }
);

gpu_test!(
    /// Deposit and erase on both layouts match the reference, from a non-empty content.
    fn composition_matches_the_reference(ctx) {
        let fixture = Fixture::new(&ctx, primitives::uv_sphere(32, 16), 256);
        let painter = Painter::new(&ctx).expect("shaders compile");
        let view = fixture.map.create_view(&Default::default());
        let camera = camera(Vec3::new(0.3, 0.8, 3.0));
        let path = [[60.0, 128.0], [200.0, 128.0]];
        let size = fixture.size;
        let initial: Vec<[f32; 4]> = (0..size * size)
            .map(|i| {
                let c = (i % 7) as f32 / 7.0;
                [0.2 * c, 0.5 * c, 0.1 * c, c]
            })
            .collect();
        for tool in [Tool::Brush, Tool::Eraser] {
            for layout in [ContentLayout::Vector, ContentLayout::Scalar] {
                let initial: Vec<[f32; 4]> = initial
                    .iter()
                    .map(|t| match layout {
                        ContentLayout::Vector => t.map(f16),
                        ContentLayout::Scalar => [f16(t[0]), f16(t[3]), 0.0, 0.0],
                    })
                    .collect();
                let texture = content(&ctx, layout, size, &initial);
                let stroke = stroke(camera, brush(50.0, 0.4, None), tool, &path);
                let value = [0.9, 0.2, 0.1, 1.0];
                let mut pool = TexturePool::new(u64::MAX);
                let mut encoder = ctx.device.create_command_encoder(&Default::default());
                let set = fixture.surface(&view);
                painter.replay_stroke(
                    &ctx,
                    &mut pool,
                    &mut encoder,
                    &stroke,
                    &set,
                    None,
                    &[Content { texture: &texture, layout, value, values: None }],
                );
                ctx.queue.submit([encoder.finish()]);
                let got = read_content(&ctx, &texture, layout);
                let (buffer, _, _) = rasterize(&ctx, &painter, &fixture, &view, &stroke, None);
                let mut changed = 0;
                for i in 0..(size * size) as usize {
                    let a = buffer[i] * stroke.brush.opacity;
                    let expected = match tool {
                        Tool::Eraser => erase(layout, initial[i], a),
                        _ => deposit(layout, initial[i], value, a),
                    };
                    for c in 0..4 {
                        assert!(
                            (got[i][c] - expected[c]).abs() <= 1.0 / 512.0,
                            "{tool:?} {layout:?} texel {i}: {:?} against {expected:?}",
                            got[i]
                        );
                    }
                    changed += usize::from(got[i] != initial[i]);
                }
                assert!(changed > 500, "{tool:?} {layout:?} changes the content");
            }
        }
    }
);

gpu_test!(
    /// M8 criterion 2, paint side: a stroke painted live in batches of random sizes gives
    /// exactly the content its replay gives, and undo restores the content exactly.
    fn live_painting_equals_replay(ctx) {
        let fixture = Fixture::new(&ctx, primitives::uv_sphere(48, 24), 512);
        let painter = Painter::new(&ctx).expect("shaders compile");
        let view = fixture.map.create_view(&Default::default());
        let set = fixture.surface(&view);
        let camera = camera(Vec3::new(-0.4, 1.0, 3.0));
        let grain = alphas::find("alpha.grain").expect("shipped").image();
        let size = fixture.size;
        let path = [[30.0, 200.0], [120.0, 60.0], [230.0, 180.0], [90.0, 230.0]];
        let base: Vec<[f32; 4]> = (0..size * size).map(|i| [f16((i % 5) as f32 * 0.1), 0.0, 0.0, f16(0.5)]).collect();
        for (alpha, tool) in [(None, Tool::Brush), (Some(&grain), Tool::Brush), (None, Tool::Eraser)] {
            let stroke = stroke(camera, brush(36.0, 0.6, None), tool, &path);
            let value = [0.1, 0.7, 0.3, 1.0];
            let live_texture = content(&ctx, ContentLayout::Vector, size, &base);
            let replay_texture = content(&ctx, ContentLayout::Vector, size, &base);
            let live_content = Content { texture: &live_texture, layout: ContentLayout::Vector, value, values: None };
            let replay_content = Content { texture: &replay_texture, layout: ContentLayout::Vector, value, values: None };
            let mut pool = TexturePool::new(u64::MAX);

            // Live, in batches of 1 to 17 dabs, one submit per batch like frames.
            let mut encoder = ctx.device.create_command_encoder(&Default::default());
            let mut live = LiveStroke::begin(&painter, &ctx, &mut pool, &mut encoder, stroke.clone(), &set, alpha, &[ContentLayout::Vector]);
            let mut start = 0;
            let mut batch = 1;
            while start < stroke.dabs.len() {
                let end = (start + batch).min(stroke.dabs.len());
                live.add(&painter, &ctx, &mut pool, &mut encoder, &set, &[live_content], &stroke.dabs[start..end]);
                ctx.queue.submit([std::mem::replace(&mut encoder, ctx.device.create_command_encoder(&Default::default())).finish()]);
                pool.flush();
                start = end;
                batch = batch * 7 % 17 + 1;
            }
            let finished = live.finish(&painter, &mut pool);
            assert_eq!(finished.stroke.dabs, stroke.dabs);

            painter.replay_stroke(&ctx, &mut pool, &mut encoder, &stroke, &set, alpha, &[replay_content]);
            ctx.queue.submit([encoder.finish()]);
            let live_texels = readback::read_texture(&ctx, &live_texture).expect("readback");
            let replay_texels = readback::read_texture(&ctx, &replay_texture).expect("readback");
            assert!(live_texels == replay_texels, "{tool:?}: live and replay differ");
            let base_bytes = {
                let t = content(&ctx, ContentLayout::Vector, size, &base);
                readback::read_texture(&ctx, &t).expect("readback")
            };
            assert!(live_texels != base_bytes, "the stroke changes the content");

            // Undo: the saved tiles bring back the content from before the stroke.
            let mut encoder = ctx.device.create_command_encoder(&Default::default());
            assert!(finished.before[0].restore(&mut pool, &mut encoder, &live_texture, None));
            ctx.queue.submit([encoder.finish()]);
            assert!(readback::read_texture(&ctx, &live_texture).expect("readback") == base_bytes, "undo");
            let tiles: Vec<[u32; 2]> = finished.before[0].tiles().collect();
            assert_eq!(tiles, finished.tiles);
        }
    }
);

/// Bilinear coverage of `texels` (a vector content) at `uv`, as the viewport samples.
fn coverage_at(texels: &[[f32; 4]], size: u32, uv: Vec2) -> f32 {
    let p = uv * size as f32 - 0.5;
    let base = p.floor();
    let f = p - base;
    let at = |dx: i32, dy: i32| {
        let x = (base.x as i32 + dx).clamp(0, size as i32 - 1) as u32;
        let y = (base.y as i32 + dy).clamp(0, size as i32 - 1) as u32;
        texels[(y * size + x) as usize][3]
    };
    let top = at(0, 0) * (1.0 - f.x) + at(1, 0) * f.x;
    let bottom = at(0, 1) * (1.0 - f.x) + at(1, 1) * f.x;
    top * (1.0 - f.y) + bottom * f.y
}

/// Paints a stroke from the top face to the +X face of the cross-unwrapped cube (their
/// shared edge is a UV seam) with the surface map `map`; returns the smallest coverage
/// the viewport would show along the core of the stroke, and the number of pixels seen.
fn seam_coverage(
    ctx: &GpuContext,
    painter: &Painter,
    fixture: &Fixture,
    map: &wgpu::Texture,
) -> (f32, usize) {
    let view = map.create_view(&Default::default());
    let set = fixture.surface(&view);
    let eye = Vec3::new(2.6, 2.2, 1.6);
    let camera = camera(eye);
    let projection = StrokeProjection::new(&camera);
    let pixel = |p: Vec3| projection.pixel(projection.clip(p)).expect("in front");
    let from = pixel(Vec3::new(0.2, 1.0, 0.1));
    let to = pixel(Vec3::new(1.0, 0.2, 0.0));
    let mut brush = brush(16.0, 1.0, None);
    brush.flow = 1.0;
    brush.opacity = 1.0;
    let stroke = stroke(
        camera,
        brush,
        Tool::Brush,
        &[from.to_array(), to.to_array()],
    );
    let size = fixture.size;
    let texture = content(
        ctx,
        ContentLayout::Vector,
        size,
        &vec![[0.0; 4]; (size * size) as usize],
    );
    let mut pool = TexturePool::new(u64::MAX);
    let mut encoder = ctx.device.create_command_encoder(&Default::default());
    painter.replay_stroke(
        ctx,
        &mut pool,
        &mut encoder,
        &stroke,
        &set,
        None,
        &[Content {
            texture: &texture,
            layout: ContentLayout::Vector,
            value: [1.0, 0.0, 0.0, 1.0],
            values: None,
        }],
    );
    ctx.queue.submit([encoder.finish()]);
    let texels = read_content(ctx, &texture, ContentLayout::Vector);

    // Every pixel of the stroke's core that sees the cube, through its UV.
    let positions = &fixture.mesh.positions;
    let triangles = &fixture.mesh.triangles;
    let uvs = fixture.mesh.uv0.as_ref().expect("the cube has UVs");
    let bvh = Bvh::build(positions, triangles);
    let inverse = projection.view_proj.inverse();
    let (mut lowest, mut seen) = (1.0f32, 0);
    let direction = (to - from).normalize();
    let length = from.distance(to);
    for y in 0..VIEWPORT[1] {
        for x in 0..VIEWPORT[0] {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let along = (p - from).dot(direction);
            let across = (p - from - direction * along).length();
            if along < 0.1 * length || along > 0.9 * length || across > 0.4 * 8.0 {
                continue;
            }
            let ndc = Vec2::new(
                p.x / VIEWPORT[0] as f32 * 2.0 - 1.0,
                1.0 - p.y / VIEWPORT[1] as f32 * 2.0,
            );
            let near = inverse.project_point3(ndc.extend(1.0));
            let far = inverse.project_point3(ndc.extend(0.5));
            let ray = Ray {
                origin: near,
                direction: far - near,
            };
            let Some(hit) = bvh.closest_hit(positions, triangles, &ray, f32::INFINITY) else {
                continue;
            };
            let [a, b, c] = triangles[hit.triangle as usize].map(|v| Vec2::from(uvs[v as usize]));
            let [u, v] = hit.barycentric;
            let uv = a * (1.0 - u - v) + b * u + c * v;
            lowest = lowest.min(coverage_at(&texels, size, uv));
            seen += 1;
        }
    }
    (lowest, seen)
}

gpu_test!(
    /// M8 criterion 1: a stroke across a UV seam of the test cube leaves no hole: every
    /// pixel of its core shows full coverage, read through the UVs with bilinear
    /// filtering like the viewport. The same stroke painted with a surface map that is
    /// not dilated (texels outside the islands get nothing) shows holes along the seam,
    /// which proves the test sees them.
    fn a_stroke_across_a_seam_leaves_no_hole(ctx) {
        let fixture = Fixture::new(&ctx, primitives::cube_cross(), 512);
        let painter = Painter::new(&ctx).expect("shaders compile");
        let (lowest, seen) = seam_coverage(&ctx, &painter, &fixture, &fixture.map);
        assert!(seen > 200, "{seen} pixels on the stroke");
        assert!(lowest >= 0.99, "coverage drops to {lowest} along the stroke");

        // The control: only texels whose center lies on their triangle keep it.
        let size = fixture.size;
        let undilated: Vec<u32> = (0..size * size)
            .map(|i| {
                let texel = [i % size, i / size];
                let surface = fixture.surface_at(texel);
                let p = (Vec2::new(texel[0] as f32, texel[1] as f32) + 0.5) / size as f32;
                let inside = (p - surface.uv).length() * size as f32 <= 1e-3;
                if inside { fixture.entries[i as usize] } else { 0 }
            })
            .collect();
        let map = ctx.device.create_texture(&wgpu::TextureDescriptor {
            usage: surface_descriptor(size).usage | wgpu::TextureUsages::COPY_DST,
            ..surface_descriptor(size)
        });
        ctx.queue.write_texture(
            map.as_image_copy(),
            bytemuck::cast_slice(&undilated),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 4),
                rows_per_image: None,
            },
            map.size(),
        );
        let (control, _) = seam_coverage(&ctx, &painter, &fixture, &map);
        assert!(control < 0.9, "without dilation the seam shows ({control})");
    }
);

/// A stroke with every dynamic: pressure through curves (size, flow, opacity), jitters,
/// scatter, path direction, tilt, with `symmetry`.
fn dynamic_stroke(camera: StrokeCamera, symmetry: Symmetry, seed: u64) -> Stroke {
    let mut brush = brush(34.0, 0.5, None);
    brush.size_jitter = 0.4;
    brush.angle_jitter = 0.8;
    brush.flow_jitter = 0.3;
    brush.scatter = 0.6;
    brush.follow_path = true;
    brush.tilt_angle = true;
    brush.pressure = PressureCurves {
        size: Curve(vec![[0.0, 0.3], [1.0, 1.0]]),
        flow: Curve::identity(),
        opacity: Curve(vec![[0.0, 0.4], [1.0, 1.0]]),
    };
    let mut stroke = stroke(
        camera,
        brush,
        Tool::Brush,
        &[[60.0, 150.0], [128.0, 90.0], [200.0, 140.0]],
    );
    for (i, dab) in stroke.dabs.iter_mut().enumerate() {
        dab.pressure = 0.2 + 0.8 * ((i as f32 * 0.37).sin() * 0.5 + 0.5);
        dab.tilt = [0.3, (i as f32 * 0.2).cos() * 0.4];
    }
    stroke.symmetry = symmetry;
    stroke.seed = seed;
    stroke
}

/// Runs `stroke` in one batch; returns the stroke buffer, the depth image of each copy,
/// the copies' cameras and the tiles.
fn rasterize_copies(
    ctx: &GpuContext,
    painter: &Painter,
    fixture: &Fixture,
    view: &wgpu::TextureView,
    stroke: &Stroke,
) -> (
    Vec<f32>,
    Vec<DepthImage>,
    Vec<StrokeProjection>,
    Vec<[u32; 2]>,
) {
    let mut pool = TexturePool::new(u64::MAX);
    let set = fixture.surface(view);
    let mut encoder = ctx.device.create_command_encoder(&Default::default());
    let target = painter.begin_stroke(ctx, &mut pool, &mut encoder, stroke, &set, None);
    let dabs = gpu_dabs(stroke, 0..stroke.dabs.len());
    let tiles = fixture.geometry.stroke_tiles(stroke, &dabs, fixture.size);
    painter.rasterize_dabs(ctx, &pool, &mut encoder, &target, &set, &dabs, &tiles);
    ctx.queue.submit([encoder.finish()]);
    let buffer = read_f32(ctx, pool.peek(target.buffer).expect("pinned"));
    let texture = target.depth_texture();
    let depths = (0..texture.depth_or_array_layers())
        .map(|layer| DepthImage {
            width: texture.width(),
            height: texture.height(),
            values: bytemuck::cast_slice(
                &readback::read_texture_level(ctx, texture, 0, layer).expect("readback"),
            )
            .to_vec(),
        })
        .collect();
    let projections = target.projections.clone();
    painter.end_stroke(&mut pool, target);
    (buffer, depths, projections, tiles)
}

/// The stroke buffer against the CPU reference over `tiles`: worst error away from steps,
/// texels on a step (occlusion edge), texels painted.
#[allow(clippy::too_many_arguments)]
fn compare(
    fixture: &Fixture,
    stroke: &Stroke,
    gpu: &[f32],
    depths: &[DepthImage],
    projections: &[StrokeProjection],
    tiles: &[[u32; 2]],
) -> (f32, usize, usize) {
    let dabs = gpu_dabs(stroke, 0..stroke.dabs.len());
    let (mut worst, mut flipped, mut painted) = (0.0f32, 0, 0);
    for tile in tiles {
        for y in tile[1] * 128..((tile[1] + 1) * 128).min(fixture.size) {
            for x in tile[0] * 128..((tile[0] + 1) * 128).min(fixture.size) {
                let surface = fixture.surface_at([x, y]);
                let views: Vec<_> = match stroke.space {
                    StrokeSpace::View => projections
                        .iter()
                        .zip(depths)
                        .map(|(projection, depth)| {
                            texel_view(
                                projection,
                                depth,
                                fixture.buffers.extent,
                                &surface,
                                [x, y],
                                fixture.size,
                            )
                        })
                        .collect(),
                    StrokeSpace::Uv => vec![texel_view_uv(
                        &projections[0],
                        &surface,
                        [x, y],
                        fixture.size,
                    )],
                };
                let expected = stroke_texel(0.0, &views, &dabs, None);
                let error = (gpu[(y * fixture.size + x) as usize] - expected).abs();
                if error > 1e-3 {
                    flipped += 1;
                } else {
                    worst = worst.max(error);
                }
                painted += usize::from(expected > 0.01);
            }
        }
    }
    (worst, flipped, painted)
}

gpu_test!(
    /// Strokes with pressure curves, jitters, scatter, path direction and tilt, through
    /// mirror and radial copies, match the CPU reference texel by texel; so does a
    /// stroke painted in UV space.
    fn dynamic_and_symmetric_strokes_match_the_reference(ctx) {
        let fixture = Fixture::new(&ctx, primitives::uv_sphere(48, 24), 256);
        let painter = Painter::new(&ctx).expect("shaders compile");
        let view = fixture.map.create_view(&Default::default());
        let camera = camera(Vec3::new(0.8, 1.0, 3.0));
        for symmetry in [
            Symmetry::default(),
            Symmetry { mirror: [true, false, false], radial: 0 },
            Symmetry { mirror: [true, false, false], radial: 3 },
        ] {
            let stroke = dynamic_stroke(camera, symmetry, 99);
            let (gpu, depths, projections, tiles) = rasterize_copies(&ctx, &painter, &fixture, &view, &stroke);
            assert_eq!(projections.len(), depths.len());
            let (worst, flipped, painted) = compare(&fixture, &stroke, &gpu, &depths, &projections, &tiles);
            assert!(painted > 300, "{symmetry:?}: the stroke paints ({painted} texels)");
            assert!(worst < 1e-4, "{symmetry:?}: worst error {worst}");
            assert!(flipped * 1000 < painted, "{symmetry:?}: {flipped} texels differ out of {painted}");
        }

        // UV space: the camera maps the UV square to the 256 × 256 view.
        let uv_camera = StrokeCamera {
            view_proj: Mat4::from_cols(
                Vec4::new(2.0, 0.0, 0.0, 0.0),
                Vec4::new(0.0, -2.0, 0.0, 0.0),
                Vec4::new(0.0, 0.0, 1.0, 0.0),
                Vec4::new(-1.0, 1.0, 0.0, 1.0),
            )
            .to_cols_array_2d(),
            viewport: VIEWPORT,
        };
        let mut stroke = dynamic_stroke(uv_camera, Symmetry::default(), 5);
        stroke.space = StrokeSpace::Uv;
        let (gpu, depths, projections, tiles) = rasterize_copies(&ctx, &painter, &fixture, &view, &stroke);
        let (worst, flipped, painted) = compare(&fixture, &stroke, &gpu, &depths, &projections, &tiles);
        assert!(painted > 300, "UV: {painted} texels");
        assert!(worst < 1e-4 && flipped * 1000 < painted, "UV: {worst}, {flipped} differ");
    }
);

gpu_test!(
    /// M9 criterion 3: a stroke with jitters, scatter, path direction, pressure and
    /// symmetry, painted live in batches of random sizes, gives exactly the content its
    /// replay gives, and two replays give the same content.
    fn jittered_strokes_replay_exactly(ctx) {
        let fixture = Fixture::new(&ctx, primitives::uv_sphere(48, 24), 512);
        let painter = Painter::new(&ctx).expect("shaders compile");
        let view = fixture.map.create_view(&Default::default());
        let set = fixture.surface(&view);
        let size = fixture.size;
        let camera = camera(Vec3::new(-0.4, 1.0, 3.0));
        let stroke = dynamic_stroke(camera, Symmetry { mirror: [true, false, false], radial: 2 }, 4242);
        let empty = vec![[0.0f32; 4]; (size * size) as usize];
        let value = [0.2, 0.6, 0.9, 1.0];
        let replay = || {
            let texture = content(&ctx, ContentLayout::Vector, size, &empty);
            let mut pool = TexturePool::new(u64::MAX);
            let mut encoder = ctx.device.create_command_encoder(&Default::default());
            painter.replay_stroke(&ctx, &mut pool, &mut encoder, &stroke, &set, None, &[Content {
                texture: &texture,
                layout: ContentLayout::Vector,
                value,
                values: None,
            }]);
            ctx.queue.submit([encoder.finish()]);
            readback::read_texture(&ctx, &texture).expect("readback")
        };
        let first = replay();
        assert!(first == replay(), "two replays differ");

        let texture = content(&ctx, ContentLayout::Vector, size, &empty);
        let live_content = Content { texture: &texture, layout: ContentLayout::Vector, value, values: None };
        let mut pool = TexturePool::new(u64::MAX);
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        let mut live = LiveStroke::begin(&painter, &ctx, &mut pool, &mut encoder, stroke.clone(), &set, None, &[ContentLayout::Vector]);
        let (mut start, mut batch) = (0, 3);
        while start < stroke.dabs.len() {
            let end = (start + batch).min(stroke.dabs.len());
            live.add(&painter, &ctx, &mut pool, &mut encoder, &set, &[live_content], &stroke.dabs[start..end]);
            ctx.queue.submit([std::mem::replace(&mut encoder, ctx.device.create_command_encoder(&Default::default())).finish()]);
            pool.flush();
            start = end;
            batch = batch * 5 % 13 + 1;
        }
        let finished = live.finish(&painter, &mut pool);
        assert_eq!(finished.stroke, stroke);
        ctx.queue.submit([encoder.finish()]);
        let live_texels = readback::read_texture(&ctx, &texture).expect("readback");
        assert!(live_texels == first, "live and replay differ");
        assert!(first != readback::read_texture(&ctx, &content(&ctx, ContentLayout::Vector, size, &empty)).expect("readback"));

        // Another seed paints something else.
        let mut other = stroke.clone();
        other.seed = 4243;
        let (a, _, _, _) = rasterize_copies(&ctx, &painter, &fixture, &view, &stroke);
        let (b, _, _, _) = rasterize_copies(&ctx, &painter, &fixture, &view, &other);
        assert_ne!(a, b);
    }
);

/// The differences between the paint at every painted texel of `rows` and the paint at
/// the texel of the transformed point, given by `column` (the UV layouts of the sphere
/// and the cylinder map mirrors and turns of their segments to whole columns), sorted.
fn asymmetry(
    buffer: &[f32],
    size: u32,
    rows: std::ops::Range<u32>,
    column: impl Fn(u32) -> u32,
) -> Vec<f32> {
    let mut differences = Vec::new();
    for y in rows {
        for x in 0..size {
            let here = buffer[(y * size + x) as usize];
            let there = buffer[(y * size + column(x)) as usize];
            if here > 0.01 || there > 0.01 {
                differences.push((here - there).abs());
            }
        }
    }
    differences.sort_by(f32::total_cmp);
    differences
}

/// The difference below which `fraction` of `differences` lie.
fn quantile(differences: &[f32], fraction: f64) -> f32 {
    differences[((differences.len() as f64 * fraction) as usize).min(differences.len() - 1)]
}

/// The stroke buffer of a soft stroke through viewport points, with `symmetry`.
fn symmetric_buffer(
    ctx: &GpuContext,
    painter: &Painter,
    fixture: &Fixture,
    eye: Vec3,
    points: &[[f32; 2]],
    symmetry: Symmetry,
) -> Vec<f32> {
    let view = fixture.map.create_view(&Default::default());
    let mut brush = brush(40.0, 0.2, None);
    brush.flow = 1.0;
    let mut stroke = stroke(camera(eye), brush, Tool::Brush, points);
    stroke.symmetry = symmetry;
    rasterize_copies(ctx, painter, fixture, &view, &stroke).0
}

gpu_test!(
    /// M9 criterion 2, texel for texel (the UV layouts map these transforms to whole
    /// columns):
    /// - on the sphere, a radial stroke paints the same at every third of a turn, and on
    ///   the cylinder, whose sides are flat rectangles, an X mirror paints at the mirror
    ///   image of every point what it paints at the point. Exact to the precision of the
    ///   surface points: 99.5 % of the texels agree within 1e-3 (a quarter of an 8-bit
    ///   level) and the median difference is below 1e-5. The rest lies on occlusion
    ///   edges (each copy's depth is rasterized on its own), and the f32 barycentrics of
    ///   the cylinder's thin triangles move surface points by up to about 2e-5;
    /// - on the sphere, the X mirror agrees within the asymmetry of the test sphere
    ///   itself: its quads are not flat, and a mirror maps the diagonal splitting a quad
    ///   onto the other diagonal of the opposite quad, so the two surfaces differ by up to
    ///   about 0.2 % of the radius between vertices;
    /// - the same stroke without symmetry fails, so the comparison sees asymmetries.
    fn symmetry_is_exact_on_the_sphere(ctx) {
        let size = 384;
        let painter = Painter::new(&ctx).expect("shaders compile");
        let sphere = Fixture::new(&ctx, primitives::uv_sphere(48, 24), size);
        let cylinder = Fixture::new(&ctx, primitives::cylinder(48), size);
        let (sphere_eye, sphere_points) = (Vec3::new(1.4, 0.6, 2.6), [[100.0, 110.0], [150.0, 140.0], [170.0, 100.0]]);
        let (cylinder_eye, cylinder_points) = (Vec3::new(1.6, 0.3, 2.4), [[90.0, 120.0], [140.0, 140.0], [175.0, 115.0]]);
        // u → 1/2 − u: column x → size/2 − 1 − x; u → u + 1/3 for a third of a turn. The
        // sides of the cylinder are the lower half of its UV square.
        let mirror = |x: u32| (size / 2 + size - 1 - x) % size;
        let turn = |x: u32| (x + size / 3) % size;
        let side = 0..size / 2 - 4;
        let exact = |name: &str, differences: &[f32]| {
            assert!(differences.len() > 500, "{name}: {} painted texels compared", differences.len());
            let (median, p995) = (quantile(differences, 0.5), quantile(differences, 0.995));
            assert!(median < 1e-5 && p995 < 1e-3, "{name}: median {median}, 99.5th percentile {p995}");
        };

        let radial = symmetric_buffer(&ctx, &painter, &sphere, sphere_eye, &sphere_points, Symmetry { mirror: [false; 3], radial: 3 });
        exact("radial 3 on the sphere", &asymmetry(&radial, size, 0..size, turn));
        let mirrored = symmetric_buffer(&ctx, &painter, &cylinder, cylinder_eye, &cylinder_points, Symmetry { mirror: [true, false, false], radial: 0 });
        exact("mirror X on the cylinder", &asymmetry(&mirrored, size, side.clone(), mirror));

        let mirrored = symmetric_buffer(&ctx, &painter, &sphere, sphere_eye, &sphere_points, Symmetry { mirror: [true, false, false], radial: 0 });
        let differences = asymmetry(&mirrored, size, 0..size, mirror);
        let (median, p99) = (quantile(&differences, 0.5), quantile(&differences, 0.99));
        assert!(differences.len() > 500);
        assert!(median < 0.01 && p99 < 0.05, "mirror X on the sphere: median {median}, 99th percentile {p99}");

        // The control: without symmetry, the mirror image is unpainted.
        let plain = symmetric_buffer(&ctx, &painter, &cylinder, cylinder_eye, &cylinder_points, Symmetry::default());
        let differences = asymmetry(&plain, size, side, mirror);
        assert!(quantile(&differences, 0.5) > 0.1, "without symmetry the comparison fails");
    }
);

gpu_test!(
    /// A content composed from a texture of values (a procedural material) deposits the
    /// value of each texel.
    fn values_come_from_a_texture(ctx) {
        let fixture = Fixture::new(&ctx, primitives::uv_sphere(32, 16), 256);
        let painter = Painter::new(&ctx).expect("shaders compile");
        let view = fixture.map.create_view(&Default::default());
        let set = fixture.surface(&view);
        let size = fixture.size;
        let stroke = stroke(camera(Vec3::new(0.3, 0.8, 3.0)), brush(60.0, 0.4, None), Tool::Brush, &[[60.0, 128.0], [200.0, 128.0]]);
        let values: Vec<[f32; 4]> = (0..size * size)
            .map(|i| [f16((i % size) as f32 / size as f32), f16((i / size) as f32 / size as f32), 0.5, 1.0])
            .collect();
        let value_texture = content(&ctx, ContentLayout::Vector, size, &values);
        let value_view = value_texture.create_view(&Default::default());
        for layout in [ContentLayout::Vector, ContentLayout::Scalar] {
            let initial = vec![[0.0f32; 4]; (size * size) as usize];
            let texture = content(&ctx, layout, size, &initial);
            let mut pool = TexturePool::new(u64::MAX);
            let mut encoder = ctx.device.create_command_encoder(&Default::default());
            painter.replay_stroke(&ctx, &mut pool, &mut encoder, &stroke, &set, None, &[Content {
                texture: &texture,
                layout,
                value: [9.0; 4],
                values: Some(&value_view),
            }]);
            ctx.queue.submit([encoder.finish()]);
            let got = read_content(&ctx, &texture, layout);
            let (buffer, _, _) = rasterize(&ctx, &painter, &fixture, &view, &stroke, None);
            let mut painted = 0;
            for i in 0..(size * size) as usize {
                let a = buffer[i] * stroke.brush.opacity;
                let expected = deposit(layout, initial[i], values[i], a);
                for c in 0..4 {
                    assert!((got[i][c] - expected[c]).abs() <= 1.0 / 512.0, "{layout:?} texel {i}: {:?} against {expected:?}", got[i]);
                }
                painted += usize::from(a > 0.1);
            }
            assert!(painted > 500);
        }
    }
);
