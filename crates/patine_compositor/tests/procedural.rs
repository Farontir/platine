//! Procedural fills and procedural mask fills in GpuDocument: the composition matches the
//! CPU reference texel by texel, and the content cache only evaluates what changed.

use std::sync::Arc;

use patine_compositor::compositor::Compositor;
use patine_compositor::gpu_document::GpuDocument;
use patine_compositor::reference::{ChannelKind, Texel, composite_texel, round_f16};
use patine_core::channel::Channel;
use patine_core::commands::SetFillSource;
use patine_core::history::History;
use patine_core::ids::{EffectId, IdSequence, LayerId};
use patine_core::layer::{
    BlendMode, ChannelBlend, FillSource, Layer, LayerKind, Projection, ProjectionKind,
};
use patine_core::mask::{
    LevelsParams, MaskEffect, MaskEffectEntry, MaskInitial, MaskStack, ParamValue,
};
use patine_core::project::{Project, ProjectSettings, Resolution, TextureSetId};
use patine_gpu::pool::TexturePool;
use patine_gpu::{GpuContext, gpu_test, readback};
use patine_mesh::primitives;
use patine_procedural::def::Output;
use patine_procedural::evaluate::EvaluationUniforms;
use patine_procedural::library;
use patine_procedural::reference::gray::evaluate_gray;
use patine_procedural::reference::material::evaluate_material;
use patine_procedural::reference::surface::{Surface, surface_at};
use patine_procedural::surface::{bounds_frame, gpu_vertices, material_triangles};

const CHANNELS: [Channel; 3] = [Channel::BaseColor, Channel::Roughness, Channel::Normal];

fn project() -> Project {
    let settings = ProjectSettings {
        resolution: Resolution::new(512).expect("supported"),
        ..ProjectSettings::default()
    };
    Project::new(primitives::cube_cross(), "cube.glb".into(), settings)
}

fn procedural(id: &str, params: &[(&str, ParamValue)]) -> patine_core::layer::ProceduralRef {
    let mut instance = library::find(id).expect("shipped").instance();
    for (name, value) in params {
        instance.params.insert((*name).to_owned(), value.clone());
    }
    instance
}

fn uniform(id: LayerId, rgb: [f32; 3], roughness: f32, mode: BlendMode) -> Layer {
    let mut layer = Layer::new(
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
        &[Channel::BaseColor, Channel::Roughness],
    );
    for blend in layer.channels.values_mut() {
        blend.mode = mode;
    }
    layer
}

/// A stack of uniform fills, two materials (one in a folder), and masks with procedural
/// fills. Returns the project and the copper layer.
fn stack() -> (Project, LayerId) {
    let mut project = project();
    let mut ids = IdSequence::starting_at(project.next_id);
    let base = uniform(ids.layer(), [0.4, 0.4, 0.4], 0.5, BlendMode::Normal);
    let copper_id = ids.layer();
    let mut copper = Layer::fill_procedural(
        copper_id,
        "copper",
        procedural("material.copper", &[("oxidation", ParamValue::Float(0.3))]),
        &CHANNELS,
    );
    if let LayerKind::Fill { projection, .. } = &mut copper.kind {
        *projection = Projection {
            kind: ProjectionKind::Triplanar { blend: 0.5 },
            ..Projection::default()
        };
    }
    for blend in copper.channels.values_mut() {
        blend.opacity = 0.8;
    }
    let mut red = uniform(ids.layer(), [0.9, 0.2, 0.1], 0.2, BlendMode::Multiply);
    let mut noise = MaskStack::new(MaskInitial::White);
    noise.effects = vec![
        MaskEffectEntry::new(
            ids.effect(),
            MaskEffect::Fill {
                source: FillSource::Procedural(procedural(
                    "noise.perlin",
                    &[("scale", ParamValue::Int(4))],
                )),
            },
        ),
        MaskEffectEntry::new(
            ids.effect(),
            MaskEffect::Levels(LevelsParams {
                gamma: 1.6,
                ..LevelsParams::default()
            }),
        ),
    ];
    red.mask = Some(noise);
    let mut wood = Layer::fill_procedural(
        ids.layer(),
        "wood",
        procedural("material.wood", &[]),
        &CHANNELS,
    );
    let mut circles = MaskStack::new(MaskInitial::Black);
    circles.effects = vec![MaskEffectEntry {
        blend: BlendMode::Lighten,
        ..MaskEffectEntry::new(
            ids.effect(),
            MaskEffect::Fill {
                source: FillSource::Procedural(procedural(
                    "pattern.circles",
                    &[("softness", ParamValue::Float(0.3))],
                )),
            },
        )
    }];
    wood.mask = Some(circles);
    let mut folder = Layer::folder(ids.layer(), "folder", &CHANNELS);
    folder.kind = LayerKind::Folder {
        children: vec![wood],
    };
    for blend in folder.channels.values_mut() {
        *blend = ChannelBlend {
            mode: BlendMode::Normal,
            opacity: 0.7,
        };
    }
    project.texture_sets[0].stack.layers = vec![base, copper, red, folder];
    project.next_id = ids.next_raw();
    (project, copper_id)
}

/// Every layer of the project, by id.
fn find(project: &Project, id: LayerId) -> &Layer {
    project.texture_sets[0]
        .stack
        .find(id)
        .expect("a layer of the stack")
}

/// The CPU composition of `channel` at texel `texel`, from the GPU's surface map entry.
fn cpu(project: &Project, channel: Channel, surface: &Surface, size: u32) -> [f32; 4] {
    let (center, extent) = bounds_frame(&project.mesh);
    let content = |layer: LayerId, channel: Channel| {
        let LayerKind::Fill {
            source: FillSource::Procedural(instance),
            projection,
        } = &find(project, layer).kind
        else {
            return None;
        };
        let def = library::find(&instance.id)?;
        let index = Output::Channel(channel).index()?;
        let uniforms = EvaluationUniforms::new(projection, size, center, extent, false);
        let value =
            evaluate_material(&def.id, &def.resolve(instance), &uniforms, surface)[index as usize];
        Some(match ChannelKind::of(channel) {
            ChannelKind::Scalar => [round_f16(value[0]), 0.0, 0.0, 0.0],
            _ => value.map(round_f16),
        })
    };
    let mask_fill = |layer: LayerId, effect: EffectId| {
        let mask = find(project, layer).mask.as_ref()?;
        let entry = &mask.effects[mask.index_of(effect)?];
        let MaskEffect::Fill {
            source: FillSource::Procedural(instance),
        } = &entry.effect
        else {
            return None;
        };
        let def = library::find(&instance.id)?;
        let uniforms = EvaluationUniforms::new(&Projection::default(), size, center, extent, false);
        Some(round_f16(evaluate_gray(
            def,
            &def.resolve(instance),
            &uniforms,
            surface,
        )))
    };
    let set = &project.texture_sets[0];
    let default = set
        .channels
        .iter()
        .find(|def| def.channel == channel)
        .expect("an active channel")
        .default;
    let texel = Texel {
        content: &content,
        mask_fill: &mask_fill,
    };
    composite_texel(&set.stack.layers, channel, default, &texel)
}

/// Level 0 of the final texture of `channel`.
fn gpu(
    ctx: &GpuContext,
    pool: &TexturePool,
    document: &GpuDocument,
    channel: Channel,
) -> Vec<[f32; 4]> {
    let set = document.texture_set(TextureSetId(0)).expect("synced");
    let texture = pool
        .peek(set.channel(channel).expect("active").texture)
        .expect("pinned");
    if ChannelKind::of(channel) == ChannelKind::Scalar {
        readback::read_r16f_level(ctx, texture, 0, 0)
            .expect("readback")
            .into_iter()
            .map(|r| [r, 0.0, 0.0, 0.0])
            .collect()
    } else {
        readback::read_rgba16f(ctx, texture).expect("readback")
    }
}

gpu_test!(
    /// A stack with materials in triplanar and UV projection, a folder, and masks with
    /// procedural fills: every channel matches the CPU reference at sampled texels.
    fn procedural_stacks_match_the_reference(ctx) {
        let compositor = Compositor::new(&ctx).expect("shaders compile");
        let mut pool = TexturePool::new(u64::MAX);
        let mut document = GpuDocument::new();
        let (project, _) = stack();
        document.sync(&ctx, &mut pool, &compositor, &project);
        let size = project.texture_sets[0].resolution.pixels();
        let map = pool
            .peek(document.surface_map(TextureSetId(0)).expect("procedural content builds a surface map"))
            .expect("alive");
        let entries: Vec<u32> = bytemuck::cast_slice(&readback::read_texture(&ctx, map).expect("readback")).to_vec();
        let vertices = gpu_vertices(&project.mesh);
        let indices = material_triangles(&project.mesh, 0);
        let mut failures = Vec::new();
        for channel in CHANNELS {
            let texels = gpu(&ctx, &pool, &document, channel);
            let components = if ChannelKind::of(channel) == ChannelKind::Scalar { 1 } else { 3 };
            let mut worst = 0.0f32;
            // A deterministic spread of texels.
            for k in 0..500u32 {
                let i = (k.wrapping_mul(2654435761) % (size * size)) as usize;
                let (x, y) = (i as u32 % size, i as u32 / size);
                let surface = surface_at(&vertices, &indices, entries[i], [x, y], size as f32);
                let expected = cpu(&project, channel, &surface, size);
                for c in 0..components {
                    let error = (texels[i][c] - expected[c]).abs() / expected[c].abs().max(1.0);
                    worst = worst.max(error);
                }
            }
            if worst > 1.0 / 256.0 {
                failures.push(format!("{channel:?}: {worst}"));
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }
);

gpu_test!(
    /// Editing a material's parameter evaluates only its contents (one pass writes all
    /// its outputs) and blends from its layer; editing nothing evaluates nothing; undo
    /// restores.
    fn contents_are_cached(ctx) {
        let compositor = Compositor::new(&ctx).expect("shaders compile");
        let mut pool = TexturePool::new(u64::MAX);
        let mut document = GpuDocument::new();
        let (mut project, copper) = stack();
        let passes = |document: &mut GpuDocument, pool: &mut TexturePool, project: &Project| {
            let before = compositor.procedurals().passes();
            document.sync(&ctx, pool, &compositor, project);
            compositor.procedurals().passes() - before
        };
        // One pass per material, one per procedural mask fill.
        assert_eq!(passes(&mut document, &mut pool, &project), 4);
        assert_eq!(passes(&mut document, &mut pool, &project), 0);
        let original = gpu(&ctx, &pool, &document, Channel::BaseColor);

        let mut history = History::default();
        let LayerKind::Fill { source: FillSource::Procedural(instance), .. } = &find(&project, copper).kind else {
            panic!("copper is procedural");
        };
        let mut instance = instance.clone();
        instance.params.insert("variation".into(), ParamValue::Float(0.9));
        history
            .push(&mut project, Box::new(SetFillSource::new(copper, FillSource::Procedural(instance), "copper")))
            .unwrap();
        assert_eq!(passes(&mut document, &mut pool, &project), 1);
        assert_ne!(gpu(&ctx, &pool, &document, Channel::BaseColor), original);
        history.undo(&mut project).unwrap();
        assert_eq!(passes(&mut document, &mut pool, &project), 1);
        assert_eq!(gpu(&ctx, &pool, &document, Channel::BaseColor), original);
        // Unused contents are released with the layers.
        let memory = document.content_memory();
        assert!(memory > 0);
        project.texture_sets[0].stack.layers.truncate(1);
        document.sync(&ctx, &mut pool, &compositor, &project);
        assert_eq!(document.content_memory(), 0);
        let _ = Arc::strong_count(&project.mesh);
    }
);

gpu_test!(
    /// Under a pool budget too small for any cache, every allocation during a sync evicts
    /// discardable textures that commands recorded earlier in the same encoder still use
    /// (scratch, lower caches, masks, contents, surface map). The submit must stay valid
    /// and the result must not change, through an edit and its undo as well.
    fn a_tiny_pool_budget_gives_the_same_result(ctx) {
        let compositor = Compositor::new(&ctx).expect("shaders compile");
        let (mut project, copper) = stack();
        let render = |pool: &mut TexturePool, document: &mut GpuDocument, project: &Project| {
            document.sync(&ctx, pool, &compositor, project);
            CHANNELS.map(|channel| gpu(&ctx, pool, document, channel))
        };
        let mut unlimited = (TexturePool::new(u64::MAX), GpuDocument::new());
        let mut tiny = (TexturePool::new(1), GpuDocument::new());
        let expected = render(&mut unlimited.0, &mut unlimited.1, &project);
        assert_eq!(render(&mut tiny.0, &mut tiny.1, &project), expected);
        assert!(tiny.0.evicted() > 0, "the budget forces evictions");

        let mut history = History::default();
        let LayerKind::Fill { source: FillSource::Procedural(instance), .. } = &find(&project, copper).kind else {
            panic!("copper is procedural");
        };
        let mut instance = instance.clone();
        instance.params.insert("variation".into(), ParamValue::Float(0.9));
        history
            .push(&mut project, Box::new(SetFillSource::new(copper, FillSource::Procedural(instance), "copper")))
            .unwrap();
        let edited = render(&mut unlimited.0, &mut unlimited.1, &project);
        assert_ne!(edited, expected);
        assert_eq!(render(&mut tiny.0, &mut tiny.1, &project), edited);
        history.undo(&mut project).unwrap();
        assert_eq!(render(&mut tiny.0, &mut tiny.1, &project), expected);
        // Only the final channel textures, pinned, are left.
        let pinned: u64 = tiny.1.memory();
        assert_eq!(tiny.0.memory(), pinned);
    }
);
