//! Paint layers and paint mask effects in GpuDocument: the composition matches the CPU
//! reference; a stroke painted live equals its replay after saving and loading the
//! project (M8 criterion 2); undo and redo restore tiles without replaying, and fall
//! back to a replay when the saved tiles were evicted.

use glam::{Mat4, Vec3, Vec4};
use patine_compositor::compositor::Compositor;
use patine_compositor::gpu_document::{GpuDocument, PaintError};
use patine_compositor::reference::{ChannelKind, Texel, composite_texel};
use patine_core::channel::Channel;
use patine_core::commands::{InsertLayer, InsertMaskEffect, PaintStroke, PaintTarget, SetMask};
use patine_core::history::History;
use patine_core::ids::{EffectId, IdSequence, LayerId};
use patine_core::layer::{BlendMode, FillSource, Layer, LayerKind, LayerPosition, Projection};
use patine_core::mask::{MASK_VALUE_CHANNEL, MaskEffect, MaskEffectEntry, MaskInitial, MaskStack};
use patine_core::project::{Project, ProjectSettings, Resolution, TextureSetId};
use patine_core::stroke::{BrushSnapshot, Dab, SizeSpace, Stroke, StrokeCamera, Symmetry, Tool};
use patine_gpu::pool::TexturePool;
use patine_gpu::{GpuContext, gpu_test, readback, wgpu};
use patine_mesh::primitives;

const CHANNELS: [Channel; 2] = [Channel::BaseColor, Channel::Roughness];
const SET: TextureSetId = TextureSetId(0);

fn camera() -> StrokeCamera {
    let eye = Vec3::new(2.6, 2.2, 3.0);
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
        viewport: [256, 256],
    }
}

/// A stroke from `from` to `to` (viewport pixels), one dab every 3 pixels.
fn stroke(tool: Tool, values: &[(Channel, [f32; 4])], from: [f32; 2], to: [f32; 2]) -> Stroke {
    let steps = 40;
    Stroke {
        brush: BrushSnapshot {
            size: 30.0,
            size_space: SizeSpace::Screen,
            flow: 0.7,
            opacity: 0.85,
            hardness: 0.5,
            spacing: 0.1,
            angle: 0.0,
            size_jitter: 0.0,
            angle_jitter: 0.0,
            flow_jitter: 0.0,
            scatter: 0.0,
            follow_path: false,
            alpha: None,
            material: FillSource::Uniform(values.iter().copied().collect()),
            pressure: patine_core::stroke::PressureCurves::none(),
            tilt_angle: false,
            projection: BrushSnapshot::default_projection(),
        },
        tool,
        camera: camera(),
        space: patine_core::stroke::StrokeSpace::View,
        symmetry: Symmetry::default(),
        seed: 0,
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

fn red() -> Vec<(Channel, [f32; 4])> {
    vec![
        (Channel::BaseColor, [0.9, 0.1, 0.1, 1.0]),
        (Channel::Roughness, [0.2; 4]),
    ]
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

fn top(index: usize) -> LayerPosition {
    LayerPosition {
        parent: None,
        index,
    }
}

/// A cube with a base fill, a paint layer (id `paint`), a fill above it whose black mask
/// has a paint effect, and a folder holding another paint layer.
struct Scene {
    project: Project,
    history: History,
    paint: LayerId,
    masked: LayerId,
    effect: EffectId,
    inner: LayerId,
}

fn scene() -> Scene {
    let settings = ProjectSettings {
        resolution: Resolution::new(512).expect("supported"),
        ..ProjectSettings::default()
    };
    let mut project = Project::new(primitives::cube_cross(), "cube.glb".into(), settings);
    let mut history = History::default();
    let mut ids = IdSequence::starting_at(project.next_id);
    let base = fill(ids.layer(), [0.4, 0.4, 0.4], 0.6);
    let paint = ids.layer();
    let masked = ids.layer();
    let folder = ids.layer();
    let inner = ids.layer();
    let effect = ids.effect();
    let mut group = Layer::folder(folder, "folder", &CHANNELS);
    if let LayerKind::Folder { children } = &mut group.kind {
        children.push(Layer::paint(inner, "inner", &CHANNELS));
    }
    let mut multiply = fill(masked, [0.2, 0.5, 0.9], 0.9);
    for blend in multiply.channels.values_mut() {
        blend.mode = BlendMode::Multiply;
    }
    for (index, layer) in [
        base,
        Layer::paint(paint, "paint", &CHANNELS),
        multiply,
        group,
    ]
    .into_iter()
    .enumerate()
    {
        history
            .push(
                &mut project,
                Box::new(InsertLayer::new(SET, top(index), layer)),
            )
            .expect("insert");
    }
    history
        .push(
            &mut project,
            Box::new(SetMask::new(
                masked,
                Some(MaskStack::new(MaskInitial::Black)),
                "masked",
            )),
        )
        .expect("mask");
    history
        .push(
            &mut project,
            Box::new(InsertMaskEffect::new(
                masked,
                0,
                MaskEffectEntry::new(effect, MaskEffect::paint()),
                "masked",
            )),
        )
        .expect("paint effect");
    Scene {
        project,
        history,
        paint,
        masked,
        effect,
        inner,
    }
}

/// The GPU texels of `channel`'s final texture (level 0).
fn channel(
    ctx: &GpuContext,
    pool: &TexturePool,
    document: &GpuDocument,
    channel: Channel,
) -> Vec<[f32; 4]> {
    let set = document.texture_set(SET).expect("synced");
    let texture = pool
        .peek(set.channel(channel).expect("active").texture)
        .expect("pinned");
    match ChannelKind::of(channel) {
        ChannelKind::Scalar => readback::read_r16f_level(ctx, texture, 0, 0)
            .expect("readback")
            .into_iter()
            .map(|r| [r, 0.0, 0.0, 0.0])
            .collect(),
        _ => readback::read_rgba16f(ctx, texture).expect("readback"),
    }
}

/// A paint content as `[f32; 4]` texels (scalar contents in the first two).
fn paint_content(
    ctx: &GpuContext,
    pool: &TexturePool,
    document: &GpuDocument,
    target: PaintTarget,
    channel: Channel,
) -> Option<Vec<[f32; 4]>> {
    let texture = pool.peek(document.paints().texture(target, channel)?)?;
    Some(match texture.format() {
        wgpu::TextureFormat::Rgba16Float => readback::read_rgba16f(ctx, texture).expect("readback"),
        _ => readback::read_texture(ctx, texture)
            .expect("readback")
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| {
                let v = |i: usize| half::f16::from_le_bytes([b[i], b[i + 1]]).to_f32();
                [v(0), v(2), 0.0, 0.0]
            })
            .collect(),
    })
}

/// Paints `stroke` live into `target` in batches of `batch` dabs, then adds it with a
/// command, as the application does.
#[allow(clippy::too_many_arguments)]
fn paint_live(
    ctx: &GpuContext,
    pool: &mut TexturePool,
    compositor: &Compositor,
    document: &mut GpuDocument,
    scene: &mut Scene,
    target: PaintTarget,
    stroke: &Stroke,
    batch: usize,
    alpha: Option<(
        patine_core::resource::ResourceHash,
        patine_core::resource::Resource,
    )>,
) {
    document
        .begin_stroke(
            ctx,
            pool,
            compositor,
            &scene.project,
            target,
            Stroke {
                dabs: Vec::new(),
                ..stroke.clone()
            },
            alpha.as_ref().map(|(_, resource)| resource),
        )
        .expect("a paint target");
    for dabs in stroke.dabs.chunks(batch) {
        document.paint(ctx, pool, compositor, &scene.project, dabs);
    }
    let painted = document
        .end_stroke(pool, compositor)
        .expect("dabs were painted");
    assert_eq!(painted.dabs, stroke.dabs);
    scene
        .history
        .push(
            &mut scene.project,
            Box::new(PaintStroke::new(target, painted, alpha, "paint")),
        )
        .expect("the stroke goes into the log");
    document.sync(ctx, pool, compositor, &scene.project);
}

gpu_test!(
    /// Paint layers (brush and eraser), a paint mask effect and a paint layer in a
    /// folder: every texel of every channel matches the CPU reference fed with the paint
    /// contents.
    fn painted_stacks_match_the_reference(ctx) {
        let compositor = Compositor::new(&ctx).expect("shaders compile");
        let mut pool = TexturePool::new(u64::MAX);
        let mut document = GpuDocument::new();
        let mut scene = scene();
        document.sync(&ctx, &mut pool, &compositor, &scene.project);
        let strokes = [
            (PaintTarget::Layer(scene.paint), stroke(Tool::Brush, &red(), [40.0, 90.0], [210.0, 140.0])),
            (PaintTarget::Layer(scene.paint), stroke(Tool::Eraser, &red(), [120.0, 60.0], [130.0, 200.0])),
            (
                PaintTarget::Mask(scene.masked, scene.effect),
                stroke(Tool::Brush, &[(MASK_VALUE_CHANNEL, [1.0; 4])], [60.0, 180.0], [200.0, 70.0]),
            ),
            (
                PaintTarget::Layer(scene.inner),
                stroke(Tool::Brush, &[(Channel::BaseColor, [0.1, 0.8, 0.2, 1.0])], [80.0, 40.0], [90.0, 220.0]),
            ),
        ];
        for (target, stroke) in &strokes {
            paint_live(&ctx, &mut pool, &compositor, &mut document, &mut scene, *target, stroke, 5, None);
        }
        let set = &scene.project.texture_sets[0];
        let size = set.resolution.pixels() as usize;
        let paints: Vec<(PaintTarget, Channel, Vec<[f32; 4]>)> = [
            (PaintTarget::Layer(scene.paint), Channel::BaseColor),
            (PaintTarget::Layer(scene.paint), Channel::Roughness),
            (PaintTarget::Layer(scene.inner), Channel::BaseColor),
            (PaintTarget::Mask(scene.masked, scene.effect), MASK_VALUE_CHANNEL),
        ]
        .into_iter()
        .map(|(target, channel)| {
            let texels = paint_content(&ctx, &pool, &document, target, channel).expect("painted");
            (target, channel, texels)
        })
        .collect();
        assert!(paint_content(&ctx, &pool, &document, PaintTarget::Layer(scene.inner), Channel::Roughness).is_none());
        let mut painted = 0;
        for channel_id in CHANNELS {
            let gpu = channel(&ctx, &pool, &document, channel_id);
            let default = set.channels.iter().find(|def| def.channel == channel_id).expect("active").default;
            let components = if ChannelKind::of(channel_id) == ChannelKind::Scalar { 1 } else { 3 };
            for i in 0..size * size {
                let paint = |layer: LayerId, channel: Channel| {
                    paints
                        .iter()
                        .find(|(t, c, _)| *t == PaintTarget::Layer(layer) && *c == channel)
                        .map(|(_, _, texels)| texels[i])
                };
                let mask_paint = |layer: LayerId, effect: EffectId| {
                    paints
                        .iter()
                        .find(|(t, _, _)| *t == PaintTarget::Mask(layer, effect))
                        .map(|(_, _, texels)| [texels[i][0], texels[i][1]])
                };
                let texel = Texel {
                    content: &|_, _| None,
                    mask_fill: &|_, _| None,
                    paint: &paint,
                    mask_paint: &mask_paint,
                };
                let expected = composite_texel(&set.stack.layers, channel_id, default, &texel);
                for c in 0..components {
                    let error = (gpu[i][c] - expected[c]).abs() / expected[c].abs().max(1.0);
                    assert!(error <= 1.0 / 256.0, "{channel_id:?} texel {i}: {:?} against {expected:?}", gpu[i]);
                }
                painted += usize::from(paints.iter().any(|(_, _, t)| t[i][3] > 0.01 || t[i][1] > 0.01));
            }
        }
        assert!(painted > 1000, "the strokes paint ({painted} texels)");
    }
);

/// Every final texture of `document`, as bytes.
fn finals(ctx: &GpuContext, pool: &TexturePool, document: &GpuDocument) -> Vec<Vec<[f32; 4]>> {
    CHANNELS
        .iter()
        .map(|c| channel(ctx, pool, document, *c))
        .collect()
}

gpu_test!(
    /// M8 criterion 2: strokes painted live (in batches of various sizes, on a layer and
    /// on a mask) give the textures a fresh document gives after saving and loading the
    /// project, which replays every stroke.
    fn live_painting_equals_replay_after_loading(ctx) {
        let compositor = Compositor::new(&ctx).expect("shaders compile");
        let mut pool = TexturePool::new(u64::MAX);
        let mut document = GpuDocument::new();
        let mut scene = scene();
        document.sync(&ctx, &mut pool, &compositor, &scene.project);
        let unpainted = finals(&ctx, &pool, &document);
        for (batch, target, stroke) in [
            (1, PaintTarget::Layer(scene.paint), stroke(Tool::Brush, &red(), [30.0, 60.0], [220.0, 180.0])),
            (7, PaintTarget::Layer(scene.paint), stroke(Tool::Eraser, &red(), [100.0, 40.0], [150.0, 230.0])),
            (
                13,
                PaintTarget::Mask(scene.masked, scene.effect),
                stroke(Tool::Brush, &[(MASK_VALUE_CHANNEL, [0.8; 4])], [50.0, 200.0], [210.0, 50.0]),
            ),
        ] {
            paint_live(&ctx, &mut pool, &compositor, &mut document, &mut scene, target, &stroke, batch, None);
        }
        // A stroke with an alpha, which only reaches the project with its command.
        let (hash, resource) = patine_paint::alphas::find("alpha.splatter").expect("shipped").resource();
        let mut splatter = stroke(Tool::Brush, &red(), [60.0, 120.0], [200.0, 120.0]);
        splatter.brush.alpha = Some(hash);
        splatter.brush.size = 50.0;
        let target = PaintTarget::Layer(scene.paint);
        paint_live(&ctx, &mut pool, &compositor, &mut document, &mut scene, target, &splatter, 3, Some((hash, resource)));
        let live = finals(&ctx, &pool, &document);

        let path = std::env::temp_dir().join(format!("patine-paint-{}.patine", std::process::id()));
        patine_core::file::save(&scene.project, &path, None).expect("saved");
        let loaded = patine_core::file::load(&path).expect("loaded");
        let _ = std::fs::remove_file(&path);
        let mut fresh_pool = TexturePool::new(u64::MAX);
        let mut fresh = GpuDocument::new();
        fresh.sync(&ctx, &mut fresh_pool, &compositor, &loaded);
        let replayed = finals(&ctx, &fresh_pool, &fresh);
        for (c, (a, b)) in live.iter().zip(&replayed).enumerate() {
            let worst = a.iter().zip(b).flat_map(|(x, y)| (0..4).map(move |i| (x[i] - y[i]).abs())).fold(0.0f32, f32::max);
            assert!(worst <= 1.0 / 512.0, "{:?}: live and replay differ by {worst}", CHANNELS[c]);
        }
        assert_ne!(live, unpainted, "the strokes change the textures");
    }
);

gpu_test!(
    /// Undo restores the tiles saved before the stroke, without replaying; redo replays
    /// the one stroke; both give back the textures exactly. Painting nothing is refused
    /// where nothing can be painted.
    fn undo_and_redo_use_the_saved_tiles(ctx) {
        let compositor = Compositor::new(&ctx).expect("shaders compile");
        let mut pool = TexturePool::new(u64::MAX);
        let mut document = GpuDocument::new();
        let mut scene = scene();
        document.sync(&ctx, &mut pool, &compositor, &scene.project);
        let before = finals(&ctx, &pool, &document);
        let target = PaintTarget::Layer(scene.paint);
        paint_live(&ctx, &mut pool, &compositor, &mut document, &mut scene, target, &stroke(Tool::Brush, &red(), [40.0, 90.0], [210.0, 140.0]), 4, None);
        let after = finals(&ctx, &pool, &document);
        assert_ne!(before, after);
        assert_eq!(document.paints().replays(), 0, "the live stroke is adopted, not replayed");

        scene.history.undo(&mut scene.project).expect("undo");
        document.sync(&ctx, &mut pool, &compositor, &scene.project);
        assert_eq!(document.paints().replays(), 0, "undo restores tiles");
        assert_eq!(finals(&ctx, &pool, &document), before);
        scene.history.redo(&mut scene.project).expect("redo");
        document.sync(&ctx, &mut pool, &compositor, &scene.project);
        assert_eq!(document.paints().replays(), 1, "redo replays the one stroke");
        assert_eq!(finals(&ctx, &pool, &document), after);
        // The redone stroke saved its tiles: undo again restores without a replay.
        scene.history.undo(&mut scene.project).expect("undo");
        document.sync(&ctx, &mut pool, &compositor, &scene.project);
        assert_eq!(document.paints().replays(), 1);
        assert_eq!(finals(&ctx, &pool, &document), before);

        // A fill cannot be painted; a brush without the layer's channels paints nothing.
        let fill = scene.project.texture_sets[0].stack.layers[0].id;
        let any = stroke(Tool::Brush, &red(), [0.0, 0.0], [1.0, 1.0]);
        assert_eq!(
            document.begin_stroke(&ctx, &mut pool, &compositor, &scene.project, PaintTarget::Layer(fill), any.clone(), None),
            Err(PaintError::NotPaintable)
        );
        let metal = stroke(Tool::Brush, &[(Channel::Metallic, [1.0; 4])], [0.0, 0.0], [1.0, 1.0]);
        assert_eq!(
            document.begin_stroke(&ctx, &mut pool, &compositor, &scene.project, target, metal, None),
            Err(PaintError::NoChannel)
        );
        // A cancelled stroke leaves nothing.
        document.begin_stroke(&ctx, &mut pool, &compositor, &scene.project, target, any, None).expect("paintable");
        assert_eq!(document.painting(), Some(target));
        let dabs = stroke(Tool::Brush, &red(), [40.0, 90.0], [210.0, 140.0]).dabs;
        assert!(document.paint(&ctx, &mut pool, &compositor, &scene.project, &dabs));
        assert_ne!(finals(&ctx, &pool, &document), before);
        document.cancel_stroke(&ctx, &mut pool, &compositor, &scene.project);
        assert_eq!(document.painting(), None);
        assert_eq!(finals(&ctx, &pool, &document), before);
    }
);

gpu_test!(
    /// With a pool budget too small to keep saved tiles, undo replays the remaining
    /// strokes instead, with the same result.
    fn evicted_tiles_fall_back_to_a_replay(ctx) {
        let compositor = Compositor::new(&ctx).expect("shaders compile");
        let mut pool = TexturePool::new(1);
        let mut document = GpuDocument::new();
        let mut scene = scene();
        document.sync(&ctx, &mut pool, &compositor, &scene.project);
        let target = PaintTarget::Layer(scene.paint);
        paint_live(&ctx, &mut pool, &compositor, &mut document, &mut scene, target, &stroke(Tool::Brush, &red(), [40.0, 90.0], [210.0, 140.0]), 8, None);
        let first = finals(&ctx, &pool, &document);
        paint_live(&ctx, &mut pool, &compositor, &mut document, &mut scene, target, &stroke(Tool::Brush, &red(), [60.0, 200.0], [200.0, 60.0]), 8, None);
        assert!(pool.evicted() > 0);
        scene.history.undo(&mut scene.project).expect("undo");
        document.sync(&ctx, &mut pool, &compositor, &scene.project);
        assert_eq!(document.paints().replays(), 1, "the first stroke is replayed");
        let replayed = finals(&ctx, &pool, &document);
        for (a, b) in replayed.iter().zip(&first) {
            let worst = a.iter().zip(b).flat_map(|(x, y)| (0..4).map(move |i| (x[i] - y[i]).abs())).fold(0.0f32, f32::max);
            assert!(worst <= 1.0 / 512.0, "replay after eviction differs by {worst}");
        }
    }
);

/// Level 0 of the shading normal of the first texture set, if it has one.
fn shading(ctx: &GpuContext, pool: &TexturePool, document: &GpuDocument) -> Option<Vec<[f32; 4]>> {
    let shading = document.texture_set(SET)?.shading.as_ref()?;
    Some(readback::read_rgba16f(ctx, pool.peek(shading.texture)?).expect("readback"))
}

/// Checks the shading normal of `document` against the reference, from its height and
/// normal channels.
fn check_shading(ctx: &GpuContext, pool: &TexturePool, document: &GpuDocument, force: f32) {
    use patine_compositor::compositor::HEIGHT_SCALE;
    use patine_compositor::reference::height_normal;
    let heights: Vec<f32> = channel(ctx, pool, document, Channel::Height)
        .iter()
        .map(|t| t[0])
        .collect();
    let normals = channel(ctx, pool, document, Channel::Normal);
    let gpu = shading(ctx, pool, document).expect("a height channel gives a shading normal");
    let size = (heights.len() as f64).sqrt() as u32;
    let mut tilted = 0;
    for y in 0..size {
        for x in 0..size {
            let i = (y * size + x) as usize;
            let base = [normals[i][0], normals[i][1], normals[i][2]];
            let expected = height_normal(&heights, size, x, y, force * HEIGHT_SCALE, base);
            for c in 0..3 {
                assert!(
                    (gpu[i][c] - expected[c]).abs() <= 2.0 / 512.0,
                    "texel {x},{y}: {:?} against {expected:?}",
                    gpu[i]
                );
            }
            tilted += usize::from(expected[2] < 0.99);
        }
    }
    assert!(
        tilted > 200,
        "the relief tilts the normal ({tilted} texels)"
    );
}

gpu_test!(
    /// The shading normal combines the normal channel with the relief of the height
    /// channel, matches the reference, follows a stroke painted live on the height (as
    /// a new document gives it), and the height force.
    fn the_shading_normal_follows_the_painted_height(ctx) {
        use patine_core::commands::{SetChannelDef, SetHeightForce};
        let compositor = Compositor::new(&ctx).expect("shaders compile");
        let mut pool = TexturePool::new(u64::MAX);
        let mut document = GpuDocument::new();
        let mut scene = scene();
        let set = &scene.project.texture_sets[0];
        let all: Vec<Channel> = set.channels.iter().map(|def| def.channel).collect();
        assert!(all.contains(&Channel::Height) && all.contains(&Channel::Normal));
        // A tilted normal fill under a paint layer on every channel.
        let mut ids = IdSequence::starting_at(scene.project.next_id);
        let tilt = Layer::new(
            ids.layer(),
            "tilt",
            LayerKind::Fill {
                source: FillSource::Uniform([(Channel::Normal, [0.2, 0.1, 0.9747, 0.0])].into()),
                projection: Projection::default(),
            },
            &[Channel::Normal],
        );
        let relief = ids.layer();
        for (index, layer) in [tilt, Layer::paint(relief, "relief", &all)].into_iter().enumerate() {
            scene.history.push(&mut scene.project, Box::new(InsertLayer::new(SET, top(index), layer))).expect("insert");
        }
        document.sync(&ctx, &mut pool, &compositor, &scene.project);
        let flat = shading(&ctx, &pool, &document).expect("shading normal");
        let height = stroke(Tool::Brush, &[(Channel::Height, [0.8; 4])], [40.0, 90.0], [210.0, 140.0]);
        paint_live(&ctx, &mut pool, &compositor, &mut document, &mut scene, PaintTarget::Layer(relief), &height, 6, None);
        assert_ne!(shading(&ctx, &pool, &document).expect("shading normal"), flat, "the stroke shows in the shading");
        check_shading(&ctx, &pool, &document, 1.0);

        // A new document gives the same shading normal.
        let mut fresh_pool = TexturePool::new(u64::MAX);
        let mut fresh = GpuDocument::new();
        fresh.sync(&ctx, &mut fresh_pool, &compositor, &scene.project);
        let (live, replayed) = (shading(&ctx, &pool, &document).unwrap(), shading(&ctx, &fresh_pool, &fresh).unwrap());
        let worst = live.iter().zip(&replayed).flat_map(|(a, b)| (0..3).map(move |c| (a[c] - b[c]).abs())).fold(0.0f32, f32::max);
        assert!(worst <= 2.0 / 512.0, "live and new document differ by {worst}");

        // The height force.
        scene.history.push(&mut scene.project, Box::new(SetHeightForce::new(SET, 4.0))).expect("force");
        document.sync(&ctx, &mut pool, &compositor, &scene.project);
        check_shading(&ctx, &pool, &document, 4.0);

        // Without a height channel, no shading normal: the viewport shades with the
        // normal channel.
        scene.history.push(&mut scene.project, Box::new(SetChannelDef::remove(SET, Channel::Height))).expect("remove");
        document.sync(&ctx, &mut pool, &compositor, &scene.project);
        assert!(document.texture_set(SET).expect("synced").shading.is_none());
        assert!(document.texture_set(SET).expect("synced").shading_normal().is_some());
    }
);

gpu_test!(
    /// A brush painting a procedural material (copper, triplanar) deposits the
    /// material's texels on every channel it produces; the stroke painted live gives
    /// the textures a new document gives after saving and loading.
    fn procedural_materials_are_painted(ctx) {
        use patine_core::layer::ProceduralRef;
        let compositor = Compositor::new(&ctx).expect("shaders compile");
        let mut pool = TexturePool::new(u64::MAX);
        let mut document = GpuDocument::new();
        let mut scene = scene();
        let all: Vec<Channel> = scene.project.texture_sets[0].channels.iter().map(|def| def.channel).collect();
        let layer = LayerId(scene.project.next_id);
        scene
            .history
            .push(&mut scene.project, Box::new(InsertLayer::new(SET, top(0), Layer::paint(layer, "copper", &all))))
            .expect("insert");
        document.sync(&ctx, &mut pool, &compositor, &scene.project);
        let mut copper = stroke(Tool::Brush, &[], [40.0, 90.0], [210.0, 150.0]);
        copper.brush.size = 60.0;
        copper.brush.material = FillSource::Procedural(ProceduralRef {
            id: "material.copper".into(),
            version: 1,
            params: [("oxidation".to_owned(), patine_core::mask::ParamValue::Float(0.6))].into(),
        });
        let target = PaintTarget::Layer(layer);
        paint_live(&ctx, &mut pool, &compositor, &mut document, &mut scene, target, &copper, 5, None);
        for channel in [Channel::BaseColor, Channel::Metallic, Channel::Roughness, Channel::Normal, Channel::Height] {
            assert!(document.paints().texture(target, channel).is_some(), "{channel:?} is painted");
        }
        let colors = paint_content(&ctx, &pool, &document, target, Channel::BaseColor).expect("painted");
        // The stroke opacity is 0.85.
        let painted: Vec<[f32; 4]> = colors.iter().filter(|t| t[3] > 0.8).copied().collect();
        assert!(painted.len() > 500, "{} well covered texels", painted.len());
        let distinct = painted.iter().map(|t| (t[0] * 256.0) as i32).collect::<std::collections::HashSet<_>>();
        assert!(distinct.len() > 5, "the material varies across the stroke");
        let live = finals(&ctx, &pool, &document);

        let path = std::env::temp_dir().join(format!("patine-copper-{}.patine", std::process::id()));
        patine_core::file::save(&scene.project, &path, None).expect("saved");
        let loaded = patine_core::file::load(&path).expect("loaded");
        let _ = std::fs::remove_file(&path);
        let mut fresh_pool = TexturePool::new(u64::MAX);
        let mut fresh = GpuDocument::new();
        fresh.sync(&ctx, &mut fresh_pool, &compositor, &loaded);
        for (c, (a, b)) in live.iter().zip(&finals(&ctx, &fresh_pool, &fresh)).enumerate() {
            let worst = a.iter().zip(b).flat_map(|(x, y)| (0..4).map(move |i| (x[i] - y[i]).abs())).fold(0.0f32, f32::max);
            assert!(worst <= 1.0 / 512.0, "{:?}: live and replay differ by {worst}", CHANNELS[c]);
        }
    }
);
