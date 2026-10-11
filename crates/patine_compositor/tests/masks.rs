//! M6 acceptance criterion 1: every mask effect, in every blend mode, matches the CPU
//! reference to 1/512 over random masks. Also checks the mask cache (only the edited mask
//! is evaluated again, then the blends from its layer), undo, and the release of mask
//! textures.

use patine_compositor::compositor::{Compositor, Ping, Region};
use patine_compositor::gpu_document::GpuDocument;
use patine_compositor::reference::{self, ALL_MODES, round_f16};
use patine_core::channel::Channel;
use patine_core::commands::{InsertMaskEffect, RemoveLayer, SetMask, SetMaskEffect};
use patine_core::history::History;
use patine_core::ids::{EffectId, IdSequence, LayerId};
use patine_core::layer::{BlendMode, FillSource, Layer, LayerKind, Projection};
use patine_core::mask::{
    FilterKind, LevelsParams, MaskEffect, MaskEffectEntry, MaskInitial, MaskStack,
};
use patine_core::mesh::{MeshAsset, Submesh};
use patine_core::project::{Project, ProjectSettings, Resolution, TextureSetId};
use patine_gpu::pool::TexturePool;
use patine_gpu::{GpuContext, gpu_test, readback, wgpu};

const SIZE: u32 = 16;

/// Deterministic pseudo-random numbers in [0, 1).
struct Random(u64);

impl Random {
    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / (1u64 << 24) as f32
    }
}

fn texture(ctx: &GpuContext, texels: &[f32]) -> wgpu::Texture {
    let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mask test"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let bytes: Vec<u8> = texels
        .iter()
        .flat_map(|v| half::f16::from_f32(*v).to_le_bytes())
        .collect();
    ctx.queue.write_texture(
        texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(SIZE * 2),
            rows_per_image: None,
        },
        texture.size(),
    );
    texture
}

/// Runs one mask effect pass over `below`; returns whether it ran and the result (the
/// target starts as a copy of `below`, as in the compositor's ping-pong).
fn run(
    ctx: &GpuContext,
    compositor: &Compositor,
    below: &[f32],
    entry: &MaskEffectEntry,
) -> (bool, Vec<f32>) {
    let source = texture(ctx, below);
    let target = texture(ctx, below);
    let mut encoder = ctx.device.create_command_encoder(&Default::default());
    let ran = compositor.mask_effect(
        ctx,
        &mut encoder,
        Ping {
            read: &source.create_view(&Default::default()),
            write: &target.create_view(&Default::default()),
        },
        entry,
        &Region::full(SIZE),
    );
    ctx.queue.submit([encoder.finish()]);
    let result = readback::read_r16f_level(ctx, &target, 0, 0).expect("readback");
    (ran, result)
}

fn entry(effect: MaskEffect, blend: BlendMode, opacity: f32) -> MaskEffectEntry {
    MaskEffectEntry {
        blend,
        opacity,
        ..MaskEffectEntry::new(EffectId(1), effect)
    }
}

/// The operations checked: fills, levels over their whole range (narrow and empty input
/// ranges, steep gammas, reversed output), inversion.
fn effects() -> Vec<MaskEffect> {
    let levels = |in_low, in_high, gamma, out_low, out_high| {
        MaskEffect::Levels(LevelsParams {
            in_low,
            in_high,
            gamma,
            out_low,
            out_high,
        })
    };
    vec![
        MaskEffect::uniform(0.0),
        MaskEffect::uniform(0.3),
        MaskEffect::uniform(1.0),
        levels(0.0, 1.0, 1.0, 0.0, 1.0),
        levels(0.2, 0.8, 1.0, 0.0, 1.0),
        levels(0.4, 0.45, 1.0, 0.0, 1.0),
        levels(0.5, 0.5, 1.0, 0.0, 1.0),
        levels(0.0, 1.0, 0.2, 0.0, 1.0),
        levels(0.0, 1.0, 5.0, 0.1, 0.9),
        levels(0.1, 0.9, 1.7, 0.9, 0.1),
        MaskEffect::invert(),
    ]
}

gpu_test!(
    fn every_mask_effect_matches_the_reference(ctx) {
        let compositor = Compositor::new(&ctx).expect("shaders compile");
        let mut random = Random(11);
        // Random masks with the exact ends and the levels thresholds.
        let mut below: Vec<f32> = (0..SIZE * SIZE).map(|_| round_f16(random.next())).collect();
        below[..5].copy_from_slice(&[0.0, 1.0, 0.5, 0.4, 0.45]);
        let mut failures = Vec::new();
        for mode in ALL_MODES {
            for effect in effects() {
                for opacity in [0.0, 0.4, 1.0] {
                    let entry = entry(effect.clone(), mode, opacity);
                    let (ran, gpu) = run(&ctx, &compositor, &below, &entry);
                    assert_eq!(ran, mode != BlendMode::Disable, "{mode:?}");
                    for (i, (&m, &g)) in below.iter().zip(&gpu).enumerate() {
                        let cpu = reference::mask_effect(&entry, m);
                        if (g - cpu).abs() > 1.0 / 512.0 {
                            failures.push(format!("{mode:?} {effect:?} {opacity} texel {i}: m {m} gpu {g} cpu {cpu}"));
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{} failures, first: {:#?}", failures.len(), &failures[..failures.len().min(5)]);
    }
);

gpu_test!(
    /// Hidden effects and effects of later milestones encode nothing: the mask below
    /// stays in the read texture.
    fn skipped_effects_encode_nothing(ctx) {
        let compositor = Compositor::new(&ctx).expect("shaders compile");
        let below = vec![0.25; (SIZE * SIZE) as usize];
        let mut hidden = entry(MaskEffect::invert(), BlendMode::Normal, 1.0);
        hidden.visible = false;
        let blur = entry(
            MaskEffect::Filter {
                kind: FilterKind::GaussianBlur,
                params: Default::default(),
            },
            BlendMode::Normal,
            1.0,
        );
        for skipped in [hidden, blur] {
            let before = compositor.mask_passes();
            assert!(!run(&ctx, &compositor, &below, &skipped).0);
            assert_eq!(compositor.mask_passes(), before);
        }
    }
);

fn project() -> Project {
    let mesh = MeshAsset {
        name: "tri".into(),
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 3],
        uv0: Some(vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]),
        tangents: Some(vec![[1.0, 0.0, 0.0, 1.0]; 3]),
        colors: None,
        triangles: vec![[0, 1, 2]],
        triangle_polygon: vec![0],
        triangle_object: vec![0],
        objects: vec!["tri".into()],
        materials: vec!["a".into()],
        submeshes: vec![Submesh {
            material: 0,
            triangles: 0..1,
        }],
    };
    let settings = ProjectSettings {
        resolution: Resolution::new(512).expect("supported"),
        ..ProjectSettings::default()
    };
    Project::new(mesh, "tri.glb".into(), settings)
}

fn roughness_fill(id: LayerId, value: f32) -> Layer {
    let mut layer = Layer::new(
        id,
        "fill",
        LayerKind::Fill {
            source: FillSource::Uniform([(Channel::Roughness, [value; 4])].into()),
            projection: Projection::default(),
        },
        &[Channel::Roughness],
    );
    layer
        .channels
        .get_mut(&Channel::Roughness)
        .expect("the fill affects the roughness")
        .opacity = 0.8;
    layer
}

/// The composed roughness (top mip: the composition is uniform).
fn roughness(ctx: &GpuContext, pool: &TexturePool, document: &GpuDocument) -> f32 {
    let set = document.texture_set(TextureSetId(0)).expect("synced");
    let texture = pool
        .peek(set.channel(Channel::Roughness).expect("active").texture)
        .expect("pinned");
    readback::read_r16f_level(ctx, texture, texture.mip_level_count() - 1, 0).expect("readback")[0]
}

/// Value of the mask of `id` (top mip).
fn mask_value(ctx: &GpuContext, document: &GpuDocument, id: LayerId) -> f32 {
    let view = document.mask(id).expect("masked");
    let texture = view.texture();
    readback::read_r16f_level(ctx, texture, texture.mip_level_count() - 1, 0).expect("readback")[0]
}

gpu_test!(
    /// Editing a mask evaluates only that mask, then blends from its layer; masks follow
    /// undo, and their textures are released with them.
    fn masks_are_cached_and_released(ctx) {
        let compositor = Compositor::new(&ctx).expect("shaders compile");
        let mut pool = TexturePool::new(u64::MAX);
        let mut document = GpuDocument::new();
        let mut project = project();
        let mut ids = IdSequence::starting_at(project.next_id);
        project.texture_sets[0].stack.layers = (0..6)
            .map(|i| roughness_fill(ids.layer(), 0.15 * i as f32))
            .collect();
        project.next_id = ids.next_raw();
        let layer = |index: usize, project: &Project| project.texture_sets[0].stack.layers[index].id;
        let (first, fourth) = (layer(0, &project), layer(3, &project));
        let default = project.texture_sets[0]
            .channels
            .iter()
            .find(|def| def.channel == Channel::Roughness)
            .unwrap()
            .default;
        let check = |document: &GpuDocument, pool: &TexturePool, project: &Project| {
            let cpu = reference::composite_stored(&project.texture_sets[0].stack.layers, Channel::Roughness, default)[0];
            let gpu = roughness(&ctx, pool, document);
            assert!((gpu - cpu).abs() <= 1.0 / 512.0, "gpu {gpu}, cpu {cpu}");
        };
        // (mask passes, blend passes) of a sync.
        let passes = |document: &mut GpuDocument, pool: &mut TexturePool, project: &Project| {
            let before = (compositor.mask_passes(), compositor.blend_passes());
            document.sync(&ctx, pool, &compositor, project);
            (compositor.mask_passes() - before.0, compositor.blend_passes() - before.1)
        };
        assert_eq!(passes(&mut document, &mut pool, &project), (0, 6));
        let unmasked = pool.memory();
        let original = roughness(&ctx, &pool, &document);

        let mut history = History::default();
        let mut push = |project: &mut Project, command: Box<dyn patine_core::command::Command>| {
            history.seal();
            history.push(project, command).unwrap();
        };
        // A black mask on the fourth layer hides it: no effect pass, a new texture, and
        // the blends from that layer (the lower cache is built below it).
        push(&mut project, Box::new(SetMask::new(fourth, Some(MaskStack::new(MaskInitial::Black)), "fill")));
        let changed = document.sync(&ctx, &mut pool, &compositor, &project);
        assert_eq!(changed, [TextureSetId(0)], "a new mask texture: rebind");
        assert_eq!(mask_value(&ctx, &document, fourth), 0.0);
        check(&document, &pool, &project);
        assert!(pool.memory() > unmasked);

        // Two effects, then editing the top one: two mask passes, three blends (the
        // fourth layer and the two above it, from the lower cache).
        let mut sequence = IdSequence::starting_at(project.next_id);
        let fill = MaskEffectEntry::new(sequence.effect(), MaskEffect::uniform(0.6));
        push(&mut project, Box::new(InsertMaskEffect::new(fourth, 0, fill, "fill")));
        let mut sequence = IdSequence::starting_at(project.next_id);
        let mut levels = MaskEffectEntry::new(
            sequence.effect(),
            MaskEffect::Levels(LevelsParams { gamma: 2.0, ..LevelsParams::default() }),
        );
        push(&mut project, Box::new(InsertMaskEffect::new(fourth, 1, levels.clone(), "fill")));
        assert_eq!(passes(&mut document, &mut pool, &project), (2, 3));
        levels.opacity = 0.5;
        push(&mut project, Box::new(SetMaskEffect::new(fourth, levels, "fill")));
        assert_eq!(passes(&mut document, &mut pool, &project), (2, 3));
        let expected = reference::mask_stored(project.texture_sets[0].stack.layers[3].mask.as_ref().unwrap());
        assert!((mask_value(&ctx, &document, fourth) - expected).abs() <= 1.0 / 512.0);
        check(&document, &pool, &project);
        // Nothing changed: no pass at all.
        assert_eq!(passes(&mut document, &mut pool, &project), (0, 0));

        // A white mask below: its texture is cleared (no effect pass), and the blends
        // start at the first layer.
        push(&mut project, Box::new(SetMask::new(first, Some(MaskStack::new(MaskInitial::White)), "fill")));
        assert_eq!(passes(&mut document, &mut pool, &project), (0, 6));
        assert_eq!(mask_value(&ctx, &document, first), 1.0);
        check(&document, &pool, &project);

        // Deleting a masked layer releases its mask.
        let masked_memory = pool.memory();
        push(&mut project, Box::new(RemoveLayer::new(first, "fill")));
        let changed = document.sync(&ctx, &mut pool, &compositor, &project);
        assert_eq!(changed, [TextureSetId(0)], "a released mask: rebind");
        assert!(document.mask(first).is_none());
        assert!(pool.memory() < masked_memory);
        check(&document, &pool, &project);

        // Undoing everything gives the original composition back, without masks.
        history.go_to(&mut project, 0).unwrap();
        document.sync(&ctx, &mut pool, &compositor, &project);
        assert_eq!(roughness(&ctx, &pool, &document), original);
        assert!(document.mask(fourth).is_none());
        assert_eq!(document.mask_memory(), 0);
        assert_eq!(pool.memory(), unmasked);
    }
);
