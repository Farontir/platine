//! The paint passes on the test adapter: depth against ray casting, dabs and composition
//! against the CPU reference, live painting against replay (M8 criterion 2), and a stroke
//! across a UV seam of the cube without holes (M8 criterion 1).

use glam::{Mat4, Vec2, Vec3, Vec4};
use patine_core::channel::Channel;
use patine_core::layer::FillSource;
use patine_core::mesh::MeshAsset;
use patine_core::stroke::{BrushSnapshot, Dab, SizeSpace, Stroke, StrokeCamera, Symmetry, Tool};
use patine_gpu::pool::TexturePool;
use patine_gpu::{GpuContext, gpu_test, readback, wgpu};
use patine_mesh::bvh::{Bvh, Ray};
use patine_mesh::primitives;
use patine_paint::alphas;
use patine_paint::dynamics::gpu_dabs;
use patine_paint::projection::{GpuDab, SetGeometry, StrokeProjection};
use patine_paint::raster::{Content, LiveStroke, Painter, SetSurface, content_descriptor};
use patine_paint::reference::{
    ContentLayout, DepthImage, deposit, erase, stroke_texel, texel_view,
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
        .touched_tiles(&[target.projection], &dabs, fixture.size);
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
                    &[Content { texture: &texture, layout, value }],
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
            let live_content = Content { texture: &live_texture, layout: ContentLayout::Vector, value };
            let replay_content = Content { texture: &replay_texture, layout: ContentLayout::Vector, value };
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
