//! Resolution independence (M10): painting at a low resolution, going 4 times higher and
//! back gives the original again (criterion 1), and the high resolution is sharp, not an
//! enlargement (criterion 2). Painted live at the high resolution, the same strokes give
//! the same content as their replay.
//!
//! The resolutions are 512 and 2048 on the software adapter (speed), 1024 and 4096 on
//! the real GPU (`PATINE_TEST_ADAPTER=hardware`), as in the criterion.

use glam::{Mat4, Vec3, Vec4};
use patine_compositor::compositor::Compositor;
use patine_compositor::gpu_document::GpuDocument;
use patine_compositor::reference::ChannelKind;
use patine_core::channel::Channel;
use patine_core::command::Command;
use patine_core::commands::{
    InsertLayer, InsertMaskEffect, PaintStroke, PaintTarget, SetMask, SetTextureSetResolution,
};
use patine_core::history::History;
use patine_core::ids::{EffectId, IdSequence, LayerId};
use patine_core::layer::{FillSource, Layer, LayerKind, LayerPosition, Projection};
use patine_core::mask::{MASK_VALUE_CHANNEL, MaskEffect, MaskEffectEntry, MaskInitial, MaskStack};
use patine_core::project::{Project, ProjectSettings, Resolution, TextureSetId};
use patine_core::stroke::{
    BrushSnapshot, Dab, PressureCurves, SizeSpace, Stroke, StrokeCamera, StrokeSpace, Symmetry,
    Tool,
};
use patine_gpu::pool::TexturePool;
use patine_gpu::{GpuContext, gpu_test, readback, wgpu};
use patine_mesh::primitives;

const CHANNELS: [Channel; 2] = [Channel::BaseColor, Channel::Roughness];
const SET: TextureSetId = TextureSetId(0);
/// Side of the stroke camera's viewport.
const VIEW: u32 = 1024;

/// The low and the high resolution of the test.
fn resolutions() -> (Resolution, Resolution) {
    let hardware = std::env::var("PATINE_TEST_ADAPTER").is_ok_and(|v| v == "hardware");
    let (low, high) = if hardware { (1024, 4096) } else { (512, 2048) };
    (
        Resolution::new(low).expect("supported"),
        Resolution::new(high).expect("supported"),
    )
}

/// A close-up of the cube's +Z face: a low-resolution texel covers several pixels.
fn camera() -> StrokeCamera {
    let eye = Vec3::new(0.3, 0.4, 2.2);
    let forward = (-eye).normalize();
    let side = forward.cross(Vec3::Y).normalize();
    let up = side.cross(forward);
    let view = Mat4::from_cols(
        Vec4::new(side.x, up.x, -forward.x, 0.0),
        Vec4::new(side.y, up.y, -forward.y, 0.0),
        Vec4::new(side.z, up.z, -forward.z, 0.0),
        Vec4::new(-side.dot(eye), -up.dot(eye), forward.dot(eye), 1.0),
    );
    let f = 1.0 / 0.4f32.tan();
    let proj = Mat4::from_cols(
        Vec4::new(f, 0.0, 0.0, 0.0),
        Vec4::new(0.0, f, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 0.0, -1.0),
        Vec4::new(0.0, 0.0, 0.05, 0.0),
    );
    StrokeCamera {
        view_proj: (proj * view).to_cols_array_2d(),
        viewport: [VIEW, VIEW],
    }
}

/// A stroke of `size` pixels from `from` to `to`, one dab every 4 pixels or so.
fn stroke(values: &[(Channel, [f32; 4])], from: [f32; 2], to: [f32; 2], hard: bool) -> Stroke {
    let steps = 120;
    Stroke {
        brush: BrushSnapshot {
            size: if hard { 160.0 } else { 90.0 },
            size_space: SizeSpace::Screen,
            flow: if hard { 1.0 } else { 0.6 },
            opacity: 1.0,
            hardness: if hard { 1.0 } else { 0.4 },
            spacing: 0.05,
            angle: 0.0,
            size_jitter: if hard { 0.0 } else { 0.4 },
            angle_jitter: 0.0,
            flow_jitter: if hard { 0.0 } else { 0.3 },
            scatter: if hard { 0.0 } else { 0.5 },
            follow_path: false,
            alpha: None,
            material: FillSource::Uniform(values.iter().copied().collect()),
            pressure: PressureCurves::none(),
            tilt_angle: false,
            projection: BrushSnapshot::default_projection(),
        },
        tool: Tool::Brush,
        camera: camera(),
        space: StrokeSpace::View,
        symmetry: Symmetry::default(),
        seed: 77,
        dabs: (0..=steps)
            .map(|i| {
                let t = i as f32 / steps as f32;
                Dab {
                    position: [
                        from[0] + (to[0] - from[0]) * t,
                        from[1] + (to[1] - from[1]) * t,
                    ],
                    pressure: 1.0,
                    tilt: [0.0; 2],
                    time: t,
                }
            })
            .collect(),
    }
}

/// The strokes of the scene: a hard one and a jittered one on the paint layer, then one
/// into the mask.
fn strokes(scene: &Scene) -> Vec<(PaintTarget, Stroke)> {
    let red = [
        (Channel::BaseColor, [0.9, 0.1, 0.1, 1.0]),
        (Channel::Roughness, [0.2; 4]),
    ];
    let green = [(Channel::BaseColor, [0.1, 0.8, 0.2, 1.0])];
    vec![
        (
            PaintTarget::Layer(scene.paint),
            stroke(&red, [250.0, 380.0], [780.0, 560.0], true),
        ),
        (
            PaintTarget::Layer(scene.paint),
            stroke(&green, [300.0, 700.0], [700.0, 300.0], false),
        ),
        (
            PaintTarget::Mask(scene.masked, scene.effect),
            stroke(
                &[(MASK_VALUE_CHANNEL, [1.0; 4])],
                [400.0, 250.0],
                [600.0, 800.0],
                true,
            ),
        ),
    ]
}

struct Scene {
    project: Project,
    history: History,
    paint: LayerId,
    masked: LayerId,
    effect: EffectId,
}

fn fill(id: LayerId, rgb: [f32; 3], roughness: f32) -> Layer {
    Layer::new(
        id,
        "fill",
        LayerKind::Fill {
            source: FillSource::Uniform(
                [
                    (Channel::BaseColor, [rgb[0], rgb[1], rgb[2], 1.0]),
                    (Channel::Roughness, [roughness; 4]),
                ]
                .into(),
            ),
            projection: Projection::default(),
        },
        &CHANNELS,
    )
}

/// A cube at `resolution` with a base fill, a paint layer and a blue fill above it whose
/// black mask has a paint effect.
fn scene(resolution: Resolution) -> Scene {
    let settings = ProjectSettings {
        resolution,
        ..ProjectSettings::default()
    };
    let mut project = Project::new(primitives::cube_cross(), "cube.glb".into(), settings);
    let mut history = History::default();
    let mut ids = IdSequence::starting_at(project.next_id);
    let base = fill(ids.layer(), [0.4, 0.4, 0.4], 0.6);
    let paint = ids.layer();
    let masked = ids.layer();
    let effect = ids.effect();
    let layers = [
        base,
        Layer::paint(paint, "paint", &CHANNELS),
        fill(masked, [0.2, 0.5, 0.9], 0.9),
    ];
    for (index, layer) in layers.into_iter().enumerate() {
        let position = LayerPosition {
            parent: None,
            index,
        };
        history
            .push(
                &mut project,
                Box::new(InsertLayer::new(SET, position, layer)),
            )
            .expect("insert");
    }
    let mask = SetMask::new(masked, Some(MaskStack::new(MaskInitial::Black)), "masked");
    history.push(&mut project, Box::new(mask)).expect("mask");
    let entry = MaskEffectEntry::new(effect, MaskEffect::paint());
    let insert = InsertMaskEffect::new(masked, 0, entry, "masked");
    history
        .push(&mut project, Box::new(insert))
        .expect("paint effect");
    Scene {
        project,
        history,
        paint,
        masked,
        effect,
    }
}

struct Gpu {
    compositor: Compositor,
    pool: TexturePool,
    document: GpuDocument,
}

impl Gpu {
    fn new(ctx: &GpuContext) -> Self {
        Self {
            compositor: Compositor::new(ctx).expect("shaders compile"),
            pool: TexturePool::new(u64::MAX),
            document: GpuDocument::new(),
        }
    }

    fn sync(&mut self, ctx: &GpuContext, project: &Project) {
        self.document
            .sync(ctx, &mut self.pool, &self.compositor, project);
    }

    /// Paints `stroke` live into `target` by batches of 7 dabs, then adds it with a
    /// command, as the application does.
    fn paint_live(
        &mut self,
        ctx: &GpuContext,
        scene: &mut Scene,
        target: PaintTarget,
        stroke: &Stroke,
    ) {
        let empty = Stroke {
            dabs: Vec::new(),
            ..stroke.clone()
        };
        self.document
            .begin_stroke(
                ctx,
                &mut self.pool,
                &self.compositor,
                &scene.project,
                target,
                empty,
                None,
            )
            .expect("a paint target");
        for dabs in stroke.dabs.chunks(7) {
            self.document
                .paint(ctx, &mut self.pool, &self.compositor, &scene.project, dabs);
        }
        let painted = self
            .document
            .end_stroke(&mut self.pool, &self.compositor)
            .expect("dabs were painted");
        let command = PaintStroke::new(target, painted, None, "paint");
        scene
            .history
            .push(&mut scene.project, Box::new(command))
            .expect("the stroke goes into the log");
        self.sync(ctx, &scene.project);
    }

    /// The final channels, then the paint layer's coverage and the mask's paint.
    fn state(&self, ctx: &GpuContext, scene: &Scene) -> Vec<Vec<f32>> {
        let set = self.document.texture_set(SET).expect("synced");
        let mut state: Vec<Vec<f32>> = CHANNELS
            .iter()
            .map(|channel| {
                let texture = self
                    .pool
                    .peek(set.channel(*channel).expect("active").texture)
                    .expect("pinned");
                match ChannelKind::of(*channel) {
                    ChannelKind::Scalar => {
                        readback::read_r16f_level(ctx, texture, 0, 0).expect("readback")
                    }
                    _ => readback::read_rgba16f(ctx, texture)
                        .expect("readback")
                        .into_iter()
                        .flatten()
                        .collect(),
                }
            })
            .collect();
        state.push(self.coverage(ctx, PaintTarget::Layer(scene.paint), Channel::BaseColor));
        state.push(self.coverage(
            ctx,
            PaintTarget::Mask(scene.masked, scene.effect),
            MASK_VALUE_CHANNEL,
        ));
        state
    }

    /// The coverage of a paint content, one value per texel (0 before its first stroke).
    fn coverage(&self, ctx: &GpuContext, target: PaintTarget, channel: Channel) -> Vec<f32> {
        let Some(id) = self.document.paints().texture(target, channel) else {
            let set = self.document.texture_set(SET).expect("synced");
            let size = set.channels[0].size as usize;
            return vec![0.0; size * size];
        };
        let texture = self.pool.peek(id).expect("pinned");
        match texture.format() {
            wgpu::TextureFormat::Rgba16Float => readback::read_rgba16f(ctx, texture)
                .expect("readback")
                .into_iter()
                .map(|t| t[3])
                .collect(),
            _ => readback::read_texture(ctx, texture)
                .expect("readback")
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| half::f16::from_le_bytes([b[2], b[3]]).to_f32())
                .collect(),
        }
    }
}

/// Largest difference between two states.
fn largest_difference(a: &[Vec<f32>], b: &[Vec<f32>]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .flat_map(|(a, b)| {
            assert_eq!(a.len(), b.len(), "same sizes");
            a.iter().zip(b).map(|(a, b)| (a - b).abs())
        })
        .fold(0.0, f32::max)
}

/// `values` (`size`²) enlarged `factor` times by bilinear filtering between texel
/// centers, as a texture sampler would.
fn enlarge(values: &[f32], size: usize, factor: usize) -> Vec<f32> {
    let big = size * factor;
    let at = |x: isize, y: isize| {
        let x = x.clamp(0, size as isize - 1) as usize;
        let y = y.clamp(0, size as isize - 1) as usize;
        values[y * size + x]
    };
    let mut out = vec![0.0; big * big];
    for y in 0..big {
        for x in 0..big {
            let u = (x as f32 + 0.5) / factor as f32 - 0.5;
            let v = (y as f32 + 0.5) / factor as f32 - 0.5;
            let (x0, y0) = (u.floor(), v.floor());
            let (fx, fy) = (u - x0, v - y0);
            let (x0, y0) = (x0 as isize, y0 as isize);
            let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1, y0) * fx;
            let bottom = at(x0, y0 + 1) * (1.0 - fx) + at(x0 + 1, y0 + 1) * fx;
            out[y * big + x] = top * (1.0 - fy) + bottom * fy;
        }
    }
    out
}

/// Texels in the middle of an edge: coverage between 0.1 and 0.9.
fn transitions(coverage: &[f32]) -> usize {
    coverage.iter().filter(|c| **c > 0.1 && **c < 0.9).count()
}

gpu_test!(
    /// Criteria 1 and 2 of M10, and undo after a round trip.
    fn painting_survives_a_resolution_round_trip(ctx) {
        let (low, high) = resolutions();
        let mut gpu = Gpu::new(&ctx);
        let mut scene = scene(low);
        gpu.sync(&ctx, &scene.project);
        let strokes = strokes(&scene);
        let mut before_last = Vec::new();
        for (i, (target, stroke)) in strokes.iter().enumerate() {
            if i + 1 == strokes.len() {
                before_last = gpu.state(&ctx, &scene);
            }
            gpu.paint_live(&ctx, &mut scene, *target, stroke);
        }
        let original = gpu.state(&ctx, &scene);
        let painted = transitions(&original[2]) + original[2].iter().filter(|c| **c >= 0.9).count();
        assert!(painted > 1000, "the strokes paint ({painted} texels)");

        // Up: everything replayed at the high resolution.
        let replays = gpu.document.paints().replays();
        let up = SetTextureSetResolution::new(SET, high);
        scene.history.push(&mut scene.project, Box::new(up)).expect("resolution");
        gpu.sync(&ctx, &scene.project);
        assert_eq!(gpu.document.paints().replays() - replays, strokes.len() as u64);
        let enlarged_state = gpu.state(&ctx, &scene);

        // The same strokes painted live at the high resolution give the same content.
        let mut direct_gpu = Gpu::new(&ctx);
        let mut direct = self::scene(high);
        direct_gpu.sync(&ctx, &direct.project);
        for (target, stroke) in &strokes {
            direct_gpu.paint_live(&ctx, &mut direct, *target, stroke);
        }
        let direct_state = direct_gpu.state(&ctx, &direct);
        let difference = largest_difference(&enlarged_state, &direct_state);
        assert!(difference <= 1.0 / 512.0, "replayed and painted at {high:?} differ by {difference}");
        drop(direct_gpu);

        // Criterion 2: sharp, not enlarged. The hard stroke of the mask (hardness 1: an edge
        // of one screen pixel, finer than a low-resolution texel) has a quarter of the
        // texels in transition at the high resolution, against an enlargement of the low
        // one.
        let factor = (high.pixels() / low.pixels()) as usize;
        let enlarged = enlarge(&original[3], low.pixels() as usize, factor);
        let sharp = transitions(&enlarged_state[3]);
        let blurred = transitions(&enlarged);
        assert!(
            (sharp as f32) < 0.4 * blurred as f32,
            "{sharp} texels in transition at {high:?}, {blurred} in the enlargement"
        );
        println!("{high:?}: {sharp} texels in transition, {blurred} in the enlargement");
        // The witness: the enlargement itself would fail the same test.
        assert!(transitions(&enlarged) as f32 >= 0.4 * blurred as f32);

        // Criterion 1: back to the low resolution, the original again.
        let down = SetTextureSetResolution::new(SET, low);
        scene.history.push(&mut scene.project, Box::new(down)).expect("resolution");
        gpu.sync(&ctx, &scene.project);
        let back = gpu.state(&ctx, &scene);
        let difference = largest_difference(&back, &original);
        assert!(difference <= 1.0 / 512.0, "the round trip changed the paint by {difference}");
        println!("round trip: largest difference {difference}");

        // Undo the two resolution changes and the last stroke: replayed, without the tiles
        // saved at the first resolution (released with the rasters).
        for _ in 0..3 {
            assert!(scene.history.undo(&mut scene.project).expect("undo"));
        }
        gpu.sync(&ctx, &scene.project);
        let undone = gpu.state(&ctx, &scene);
        // The mask had no paint before its stroke: compare the rest.
        let difference = largest_difference(&undone[..3], &before_last[..3]);
        assert!(difference <= 1.0 / 512.0, "undo after the round trip differs by {difference}");
        assert!(undone[3].iter().all(|c| *c == 0.0), "the mask stroke is undone");
    }
);

gpu_test!(
    /// The command alone: applying and reverting a resolution change restores the
    /// texture set exactly.
    fn resolution_commands_revert(_ctx) {
        let (low, high) = resolutions();
        let mut project = scene(low).project;
        let before = project.clone();
        let mut command = SetTextureSetResolution::new(SET, high);
        command.apply(&mut project).expect("apply");
        assert_eq!(project.texture_sets[0].resolution, high);
        command.revert(&mut project).expect("revert");
        assert_eq!(project, before);
    }
);
