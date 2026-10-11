//! Evaluation of layer stacks by GpuDocument: random stacks (with random masks) against
//! the CPU reference, and the lower cache (only the edited layer and those above are
//! blended again).

use patine_compositor::compositor::Compositor;
use patine_compositor::gpu_document::GpuDocument;
use patine_compositor::reference::{self, ALL_MODES, ChannelKind};
use patine_core::channel::Channel;
use patine_core::command::Command;
use patine_core::commands::SetFillValue;
use patine_core::history::History;
use patine_core::ids::{EffectId, LayerId};
use patine_core::layer::{BlendMode, ChannelBlend, FillSource, Layer, LayerKind, Projection};
use patine_core::mask::{LevelsParams, MaskEffect, MaskEffectEntry, MaskInitial, MaskStack};
use patine_core::mesh::{MeshAsset, Submesh};
use patine_core::project::{Project, ProjectSettings, Resolution, TextureSetId};
use patine_gpu::pool::TexturePool;
use patine_gpu::{GpuContext, gpu_test, readback};
use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};

const CHANNELS: [Channel; 3] = [Channel::BaseColor, Channel::Roughness, Channel::Normal];

/// Modes of the random stacks. These tests check the composition (order, folders,
/// opacity, visibility, lower cache); the exactness of every mode, pass by pass, is
/// M5 criterion 1 (tests/blend.rs). Excluded here:
/// - Divide, Color Dodge and Color Burn divide by the source (or 1 − source): a small
///   divisor multiplies the f16 rounding of every layer below by up to 1/ε, so a stack's
///   error has no useful bound;
/// - Hue, Saturation, Color and Luminosity take the hue of nearly gray colors, which
///   flips with the last bit of rounding (a discontinuity of the W3C formulas).
fn stack_modes() -> Vec<BlendMode> {
    ALL_MODES
        .into_iter()
        .filter(|mode| {
            !matches!(
                mode,
                BlendMode::Divide
                    | BlendMode::ColorDodge
                    | BlendMode::ColorBurn
                    | BlendMode::Hue
                    | BlendMode::Saturation
                    | BlendMode::Color
                    | BlendMode::Luminosity
            )
        })
        .collect()
}

fn project() -> Project {
    let mesh = MeshAsset {
        name: "quad".into(),
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 3],
        uv0: Some(vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]),
        tangents: Some(vec![[1.0, 0.0, 0.0, 1.0]; 3]),
        colors: None,
        triangles: vec![[0, 1, 2]],
        triangle_polygon: vec![0],
        triangle_object: vec![0],
        objects: vec!["quad".into()],
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
    Project::new(mesh, "quad.glb".into(), settings)
}

/// The composed value of `channel` (the 1×1 top mip: the composition is uniform).
fn composed(
    ctx: &GpuContext,
    pool: &TexturePool,
    document: &GpuDocument,
    channel: Channel,
) -> [f32; 4] {
    let set = document.texture_set(TextureSetId(0)).expect("synced");
    let texture = pool
        .peek(set.channel(channel).expect("active channel").texture)
        .expect("pinned");
    let top = texture.mip_level_count() - 1;
    if ChannelKind::of(channel) == ChannelKind::Scalar {
        let r = readback::read_r16f_level(ctx, texture, top, 0).expect("readback")[0];
        [r, 0.0, 0.0, 0.0]
    } else {
        readback::read_rgba16f_level(ctx, texture, top, 0).expect("readback")[0]
    }
}

fn within(gpu: [f32; 4], cpu: [f32; 4], channel: Channel) -> bool {
    let components = if ChannelKind::of(channel) == ChannelKind::Scalar {
        1
    } else {
        3
    };
    (0..components).all(|i| (gpu[i] - cpu[i]).abs() <= cpu[i].abs().max(1.0) / 256.0)
}

/// A random mask effect: kind (0 fill, 1 levels, 2 invert), value or levels, blend.
#[derive(Clone, Debug)]
struct EffectSpec {
    kind: u8,
    value: f32,
    /// in_low, input range, gamma, out_low, out_high.
    levels: [f32; 5],
    mode: usize,
    opacity: f32,
    visible: bool,
}

/// A random mask: white or black, then its effects.
#[derive(Clone, Debug)]
struct MaskSpec {
    white: bool,
    effects: Vec<EffectSpec>,
}

/// Masks of the random stacks. Levels keep an input range of at least 0.2 and a gamma in
/// [0.5, 2]: a narrower range or a steeper curve multiplies the f16 rounding of the mask
/// below without bound (each levels pass alone is checked over its whole range in
/// tests/masks.rs).
fn mask_spec() -> impl Strategy<Value = Option<MaskSpec>> {
    let effect = (
        0u8..3,
        0.0f32..=1.0,
        (
            0.0f32..0.6,
            0.2f32..0.4,
            0.5f32..2.0,
            0.0f32..=1.0,
            0.0f32..=1.0,
        ),
        0..stack_modes().len(),
        0.0f32..=1.0,
        prop::bool::weighted(0.9),
    )
        .prop_map(|(kind, value, levels, mode, opacity, visible)| EffectSpec {
            kind,
            value,
            levels: [levels.0, levels.1, levels.2, levels.3, levels.4],
            mode,
            opacity,
            visible,
        });
    prop::option::weighted(
        0.5,
        (any::<bool>(), prop::collection::vec(effect, 0..4))
            .prop_map(|(white, effects)| MaskSpec { white, effects }),
    )
}

/// A random fill or folder, as data for the strategy.
#[derive(Clone, Debug)]
enum Node {
    Fill {
        values: [f32; 3],
        tilt: [f32; 2],
        mode: usize,
        opacity: f32,
        visible: bool,
        mask: Option<MaskSpec>,
    },
    Folder {
        children: Vec<Node>,
        opacity: f32,
        disabled: bool,
        mask: Option<MaskSpec>,
    },
}

fn node() -> impl Strategy<Value = Node> {
    let fill = (
        prop::array::uniform3(0.0f32..1.0),
        prop::array::uniform2(-0.6f32..0.6),
        0..stack_modes().len(),
        0.0f32..=1.0,
        prop::bool::weighted(0.9),
        mask_spec(),
    )
        .prop_map(|(values, tilt, mode, opacity, visible, mask)| Node::Fill {
            values,
            tilt,
            mode,
            opacity,
            visible,
            mask,
        });
    fill.prop_recursive(2, 12, 4, |inner| {
        (
            prop::collection::vec(inner, 1..4),
            0.0f32..=1.0,
            prop::bool::weighted(0.1),
            mask_spec(),
        )
            .prop_map(|(children, opacity, disabled, mask)| Node::Folder {
                children,
                opacity,
                disabled,
                mask,
            })
    })
}

/// Builds the mask of `spec`, with effect ids from `next`.
fn mask(spec: &MaskSpec, next: &mut u64) -> MaskStack {
    let effects = spec
        .effects
        .iter()
        .map(|effect| {
            *next += 1;
            let [in_low, range, gamma, out_low, out_high] = effect.levels;
            let operation = match effect.kind {
                0 => MaskEffect::uniform(effect.value),
                1 => MaskEffect::Levels(LevelsParams {
                    in_low,
                    in_high: in_low + range,
                    gamma,
                    out_low,
                    out_high,
                }),
                _ => MaskEffect::invert(),
            };
            MaskEffectEntry {
                id: EffectId(*next),
                visible: effect.visible,
                blend: stack_modes()[effect.mode],
                opacity: effect.opacity,
                effect: operation,
            }
        })
        .collect();
    MaskStack {
        initial: if spec.white {
            MaskInitial::White
        } else {
            MaskInitial::Black
        },
        effects,
    }
}

/// Builds the layer of `node`, with ids from `next`.
fn layer(node: &Node, next: &mut u64) -> Layer {
    *next += 1;
    let id = LayerId(*next);
    match node {
        Node::Fill {
            values,
            tilt,
            mode,
            opacity,
            visible,
            mask: mask_spec,
        } => {
            let z = (1.0 - tilt[0] * tilt[0] - tilt[1] * tilt[1])
                .max(0.1)
                .sqrt();
            let normal = {
                let l = (tilt[0] * tilt[0] + tilt[1] * tilt[1] + z * z).sqrt();
                [tilt[0] / l, tilt[1] / l, z / l, 0.0]
            };
            let content = [
                (Channel::BaseColor, [values[0], values[1], values[2], 1.0]),
                (Channel::Roughness, [values[1]; 4]),
                (Channel::Normal, normal),
            ];
            let mut layer = Layer::new(
                id,
                "fill",
                LayerKind::Fill {
                    source: FillSource::Uniform(content.into()),
                    projection: Projection::default(),
                },
                &CHANNELS,
            );
            let mode = stack_modes()[*mode];
            for (channel, blend) in &mut layer.channels {
                // Normals get the normal modes; the others get any mode.
                blend.mode = match (channel, mode) {
                    (Channel::Normal, BlendMode::NormalMapDetail | BlendMode::Disable) => mode,
                    (Channel::Normal, _) => BlendMode::NormalMapCombine,
                    _ => mode,
                };
                blend.opacity = *opacity;
            }
            layer.visible = *visible;
            layer.mask = mask_spec.as_ref().map(|spec| mask(spec, next));
            layer
        }
        Node::Folder {
            children,
            opacity,
            disabled,
            mask: mask_spec,
        } => {
            let children = children.iter().map(|child| layer(child, next)).collect();
            let mut folder = Layer::folder(id, "folder", &CHANNELS);
            folder.kind = LayerKind::Folder { children };
            for blend in folder.channels.values_mut() {
                *blend = ChannelBlend {
                    mode: if *disabled {
                        BlendMode::Disable
                    } else {
                        BlendMode::Normal
                    },
                    opacity: *opacity,
                };
            }
            folder.mask = mask_spec.as_ref().map(|spec| mask(spec, next));
            folder
        }
    }
}

gpu_test!(
    /// Random stacks of masked fills and folders compose on the GPU like the CPU
    /// reference (which stores every intermediate value in f16, as the GPU does): M6
    /// acceptance criterion 1 for whole stacks.
    fn random_stacks_match_the_reference(ctx) {
        let compositor = Compositor::new(&ctx).expect("shaders compile");
        // The test runner takes a shared closure.
        let state = std::cell::RefCell::new((TexturePool::new(u64::MAX), GpuDocument::new()));
        let mut runner = TestRunner::new(Config {
            cases: 24,
            ..Config::default()
        });
        runner
            .run(&prop::collection::vec(node(), 1..7), |nodes| {
                let mut project = project();
                let mut next = 0;
                project.texture_sets[0].stack.layers = nodes.iter().map(|n| layer(n, &mut next)).collect();
                project.next_id = next + 1;
                let (pool, document) = &mut *state.borrow_mut();
                document.sync(&ctx, pool, &compositor, &project);
                let set = &project.texture_sets[0];
                for channel in CHANNELS {
                    let default = set.channels.iter().find(|d| d.channel == channel).unwrap().default;
                    let cpu = reference::composite_stored(&set.stack.layers, channel, default);
                    let gpu = composed(&ctx, pool, document, channel);
                    prop_assert!(within(gpu, cpu, channel), "{channel:?}: gpu {gpu:?}, cpu {cpu:?}");
                }
                Ok(())
            })
            .unwrap();
    }
);

fn uniform_fill(id: u64, value: f32) -> Layer {
    Layer::new(
        LayerId(id),
        "fill",
        LayerKind::Fill {
            source: FillSource::Uniform([(Channel::BaseColor, [value, value, value, 1.0])].into()),
            projection: Projection::default(),
        },
        &[Channel::BaseColor],
    )
}

gpu_test!(
    /// Editing a layer blends only it and the layers above, from the lower cache; layers
    /// that do not change a channel cost nothing for it; undo restores the composition.
    fn the_lower_cache_limits_the_work(ctx) {
        let compositor = Compositor::new(&ctx).expect("shaders compile");
        let mut pool = TexturePool::new(u64::MAX);
        let mut document = GpuDocument::new();
        let mut project = project();
        project.texture_sets[0].stack.layers = (0..10)
            .map(|i| {
                let mut layer = uniform_fill(i + 1, 0.1 * i as f32);
                layer.channels.get_mut(&Channel::BaseColor).unwrap().opacity = 0.5;
                layer
            })
            .collect();
        project.next_id = 11;
        let passes = |document: &mut GpuDocument, pool: &mut TexturePool, project: &Project| {
            let before = compositor.blend_passes();
            document.sync(&ctx, pool, &compositor, project);
            compositor.blend_passes() - before
        };
        assert_eq!(passes(&mut document, &mut pool, &project), 10, "first evaluation");
        assert_eq!(passes(&mut document, &mut pool, &project), 0, "nothing changed");
        let original = composed(&ctx, &pool, &document, Channel::BaseColor);

        let mut history = History::default();
        let edit = |project: &mut Project, history: &mut History, layer: u64, value: f32| {
            history.seal();
            let command = SetFillValue::new(LayerId(layer), Channel::BaseColor, Some([value; 4]), "fill");
            history.push(project, Box::new(command)).unwrap();
        };
        // Layer 8 is index 7: the cache below it is built once (7 blends), then only
        // layers 7 to 9 are blended (3).
        edit(&mut project, &mut history, 8, 0.9);
        assert_eq!(passes(&mut document, &mut pool, &project), 10);
        edit(&mut project, &mut history, 8, 0.2);
        assert_eq!(passes(&mut document, &mut pool, &project), 3);
        edit(&mut project, &mut history, 9, 0.4);
        assert_eq!(passes(&mut document, &mut pool, &project), 3, "above the cache");
        // Lower down, the cache is rebuilt there.
        edit(&mut project, &mut history, 3, 0.7);
        assert_eq!(passes(&mut document, &mut pool, &project), 10);
        edit(&mut project, &mut history, 3, 0.3);
        assert_eq!(passes(&mut document, &mut pool, &project), 8);
        let expected = reference::composite_stored(
            &project.texture_sets[0].stack.layers,
            Channel::BaseColor,
            [0.5, 0.5, 0.5, 1.0],
        );
        let gpu = composed(&ctx, &pool, &document, Channel::BaseColor);
        assert!(within(gpu, expected, Channel::BaseColor), "{gpu:?} vs {expected:?}");

        // Undo restores the composition.
        history.go_to(&mut project, 0).unwrap();
        passes(&mut document, &mut pool, &project);
        assert_eq!(composed(&ctx, &pool, &document, Channel::BaseColor), original);
    }
);

#[test]
fn commands_are_used_as_in_the_application() {
    // SetFillValue is the command the Properties panel issues.
    let mut project = project();
    project.texture_sets[0].stack.layers = vec![uniform_fill(1, 0.5)];
    project.next_id = 2;
    let mut command = SetFillValue::new(LayerId(1), Channel::BaseColor, Some([0.2; 4]), "fill");
    command.apply(&mut project).unwrap();
    assert_eq!(
        reference::composite(
            &project.texture_sets[0].stack.layers,
            Channel::BaseColor,
            [0.0; 4]
        ),
        [0.2; 4]
    );
}
