//! Random user actions turned into commands, for property tests.

use patine_core::channel::Channel;
use patine_core::channel::ChannelDef;
use patine_core::command::Command;
use patine_core::commands::{
    InsertLayer, InsertMaskEffect, MoveLayer, MoveMaskEffect, PaintStroke, PaintTarget,
    RemoveLayer, RemoveMaskEffect, RenameLayer, RenameTextureSet, SetChannelBlend, SetChannelDef,
    SetEnvironment, SetFillSource, SetFillValue, SetHeightForce, SetLayerVisible, SetMask,
    SetMaskEffect, SetProjection, SetTextureSetResolution,
};
use patine_core::environment::EnvironmentSource;
use patine_core::ids::LayerId;
use patine_core::layer::{
    BlendMode, ChannelBlend, FillSource, LayerKind, LayerPosition, ProceduralRef, Projection,
    ProjectionKind,
};
use patine_core::mask::ParamValue;
use patine_core::mask::{LevelsParams, MaskEffect, MaskEffectEntry, MaskInitial, MaskStack};
use patine_core::project::{Project, Resolution, TextureSetId};
use proptest::prelude::*;

use super::{fill, folder, ids, stroke};

/// A user action, with indices resolved against the document when it runs.
#[derive(Clone, Debug)]
pub enum Op {
    /// Inserts a fill, a folder or a paint layer (`kind` modulo 3).
    Insert {
        set: u8,
        parent: u8,
        index: u8,
        kind: u8,
    },
    Remove {
        layer: u8,
    },
    Move {
        layer: u8,
        parent: u8,
        index: u8,
    },
    Rename {
        layer: u8,
        name: String,
    },
    Visible {
        layer: u8,
        visible: bool,
    },
    Blend {
        layer: u8,
        channel: u8,
        mode: u8,
        opacity: Option<f32>,
    },
    RenameSet {
        set: u8,
        name: String,
    },
    Resolution {
        set: u8,
        resolution: u8,
    },
    FillValue {
        layer: u8,
        channel: u8,
        value: Option<f32>,
    },
    ChannelDef {
        set: u8,
        channel: u8,
        default: Option<f32>,
    },
    /// Adds a black or white mask (`Some`), or removes the mask.
    Mask {
        layer: u8,
        white: Option<bool>,
    },
    MaskInsert {
        layer: u8,
        index: u8,
        kind: u8,
        value: f32,
    },
    MaskRemove {
        layer: u8,
        effect: u8,
    },
    MaskMove {
        layer: u8,
        effect: u8,
        index: u8,
    },
    MaskSet {
        layer: u8,
        effect: u8,
        mode: u8,
        opacity: f32,
        visible: bool,
        value: f32,
    },
    /// Makes a fill procedural (with a scale parameter), or uniform again.
    Procedural {
        layer: u8,
        scale: Option<f32>,
    },
    Projection {
        layer: u8,
        triplanar: bool,
        scale: f32,
    },
    /// Paints or erases a stroke on a paint layer or a paint mask effect (`target`
    /// chooses among them), optionally with an alpha image.
    Stroke {
        target: u8,
        seed: u64,
        eraser: bool,
        alpha: bool,
    },
    HeightForce {
        set: u8,
        force: f32,
    },
    Rotation(f32),
    Exposure(f32),
    Source(u8),
    Seal,
    Undo,
    Redo,
}

pub fn op() -> impl Strategy<Value = Op> {
    let name = "[a-z ]{0,8}";
    prop_oneof![
        3 => (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>())
            .prop_map(|(set, parent, index, kind)| Op::Insert { set, parent, index, kind }),
        1 => any::<u8>().prop_map(|layer| Op::Remove { layer }),
        2 => (any::<u8>(), any::<u8>(), any::<u8>())
            .prop_map(|(layer, parent, index)| Op::Move { layer, parent, index }),
        1 => (any::<u8>(), name).prop_map(|(layer, name)| Op::Rename { layer, name }),
        1 => (any::<u8>(), any::<bool>()).prop_map(|(layer, visible)| Op::Visible { layer, visible }),
        2 => (any::<u8>(), any::<u8>(), any::<u8>(), proptest::option::of(0.0f32..=1.0))
            .prop_map(|(layer, channel, mode, opacity)| Op::Blend { layer, channel, mode, opacity }),
        1 => (any::<u8>(), name).prop_map(|(set, name)| Op::RenameSet { set, name }),
        1 => (any::<u8>(), any::<u8>()).prop_map(|(set, resolution)| Op::Resolution { set, resolution }),
        2 => (any::<u8>(), any::<u8>(), proptest::option::of(-1.0f32..2.0))
            .prop_map(|(set, channel, default)| Op::ChannelDef { set, channel, default }),
        2 => (any::<u8>(), any::<u8>(), proptest::option::of(0.0f32..1.0))
            .prop_map(|(layer, channel, value)| Op::FillValue { layer, channel, value }),
        2 => (any::<u8>(), proptest::option::of(any::<bool>()))
            .prop_map(|(layer, white)| Op::Mask { layer, white }),
        3 => (any::<u8>(), any::<u8>(), any::<u8>(), 0.0f32..1.0)
            .prop_map(|(layer, index, kind, value)| Op::MaskInsert { layer, index, kind, value }),
        1 => (any::<u8>(), any::<u8>()).prop_map(|(layer, effect)| Op::MaskRemove { layer, effect }),
        1 => (any::<u8>(), any::<u8>(), any::<u8>())
            .prop_map(|(layer, effect, index)| Op::MaskMove { layer, effect, index }),
        2 => (any::<u8>(), any::<u8>(), any::<u8>(), 0.0f32..1.0, any::<bool>(), 0.0f32..1.0)
            .prop_map(|(layer, effect, mode, opacity, visible, value)| Op::MaskSet {
                layer, effect, mode, opacity, visible, value,
            }),
        2 => (any::<u8>(), proptest::option::of(1.0f32..16.0))
            .prop_map(|(layer, scale)| Op::Procedural { layer, scale }),
        1 => (any::<u8>(), any::<bool>(), 0.5f32..8.0)
            .prop_map(|(layer, triplanar, scale)| Op::Projection { layer, triplanar, scale }),
        3 => (any::<u8>(), any::<u64>(), any::<bool>(), any::<bool>())
            .prop_map(|(target, seed, eraser, alpha)| Op::Stroke { target, seed, eraser, alpha }),
        1 => (any::<u8>(), -1.0f32..4.0).prop_map(|(set, force)| Op::HeightForce { set, force }),
        2 => (-10.0f32..10.0).prop_map(Op::Rotation),
        2 => (-6.0f32..6.0).prop_map(Op::Exposure),
        1 => any::<u8>().prop_map(Op::Source),
        2 => Just(Op::Seal),
        2 => Just(Op::Undo),
        1 => Just(Op::Redo),
    ]
}

/// Every layer of the project, with its texture set, in a stable order.
pub fn layers(project: &Project) -> Vec<(TextureSetId, LayerId, bool, String)> {
    let mut found = Vec::new();
    project.visit_layers(&mut |set, layer| {
        let is_folder = matches!(layer.kind, LayerKind::Folder { .. });
        found.push((set, layer.id, is_folder, layer.name.clone()));
    });
    found
}

fn pick<T: Clone>(items: &[T], index: u8) -> Option<T> {
    (!items.is_empty()).then(|| items[index as usize % items.len()].clone())
}

/// A parent in `set`: the top level or one of its folders, chosen by `index`.
fn parent(project: &Project, set: TextureSetId, index: u8) -> Option<LayerId> {
    let folders: Vec<_> = layers(project)
        .into_iter()
        .filter(|(s, _, is_folder, _)| *s == set && *is_folder)
        .map(|(_, id, _, _)| Some(id))
        .collect();
    let mut choices = vec![None];
    choices.extend(folders);
    pick(&choices, index).flatten()
}

fn position(project: &Project, set: TextureSetId, parent_index: u8, index: u8) -> LayerPosition {
    let parent = parent(project, set, parent_index);
    let len = project
        .texture_set(set)
        .and_then(|s| s.stack.children(parent))
        .map_or(0, Vec::len);
    LayerPosition {
        parent,
        // One past the end is a valid insertion point; two past it is refused.
        index: index as usize % (len + 2),
    }
}

const CHANNELS: [Channel; 4] = [
    Channel::BaseColor,
    Channel::Roughness,
    Channel::Normal,
    Channel::User(2),
];
const MODES: [BlendMode; 4] = [
    BlendMode::Normal,
    BlendMode::Multiply,
    BlendMode::Overlay,
    BlendMode::NormalMapCombine,
];

/// Every paint layer and paint mask effect of the project, in a stable order.
fn paint_targets(project: &Project) -> Vec<(PaintTarget, String)> {
    let mut found = Vec::new();
    project.visit_layers(&mut |_, layer| {
        if matches!(layer.kind, LayerKind::Paint { .. }) {
            found.push((PaintTarget::Layer(layer.id), layer.name.clone()));
        }
        for entry in layer.mask.iter().flat_map(|mask| &mask.effects) {
            if matches!(entry.effect, MaskEffect::Paint { .. }) {
                found.push((PaintTarget::Mask(layer.id, entry.id), layer.name.clone()));
            }
        }
    });
    found
}

/// The effects of the mask of layer `id`, if it has a mask.
fn mask_effects(project: &Project, id: LayerId) -> Option<&[MaskEffectEntry]> {
    project
        .texture_sets
        .iter()
        .find_map(|set| set.stack.find(id))?
        .mask
        .as_ref()
        .map(|mask| mask.effects.as_slice())
}

/// The command for `op`, or `None` when it does not apply to this document.
pub fn command(project: &Project, op: &Op) -> Option<Box<dyn Command>> {
    let sets = project.texture_sets.len() as u8;
    let all = layers(project);
    let env = &project.environment;
    Some(match op {
        Op::Insert {
            set,
            parent,
            index,
            kind,
        } => {
            let set = TextureSetId(u32::from(set % sets));
            let mut ids = ids(project);
            let layer = match kind % 3 {
                0 => fill(ids.layer(), "fill"),
                1 => folder(ids.layer(), "folder"),
                _ => patine_core::layer::Layer::paint(
                    ids.layer(),
                    "paint",
                    &[Channel::BaseColor, Channel::Roughness],
                ),
            };
            Box::new(InsertLayer::new(
                set,
                position(project, set, *parent, *index),
                layer,
            ))
        }
        Op::Remove { layer } => {
            let (_, id, _, name) = pick(&all, *layer)?;
            Box::new(RemoveLayer::new(id, name))
        }
        Op::Move {
            layer,
            parent,
            index,
        } => {
            let (set, id, _, name) = pick(&all, *layer)?;
            Box::new(MoveLayer::new(
                id,
                position(project, set, *parent, *index),
                name,
            ))
        }
        Op::Rename { layer, name } => {
            let (_, id, _, _) = pick(&all, *layer)?;
            Box::new(RenameLayer::new(id, name.clone()))
        }
        Op::Visible { layer, visible } => {
            let (_, id, _, name) = pick(&all, *layer)?;
            Box::new(SetLayerVisible::new(id, *visible, name))
        }
        Op::Blend {
            layer,
            channel,
            mode,
            opacity,
        } => {
            let (_, id, _, name) = pick(&all, *layer)?;
            let blend = opacity.map(|opacity| ChannelBlend {
                mode: MODES[*mode as usize % MODES.len()],
                opacity,
            });
            let channel = CHANNELS[*channel as usize % CHANNELS.len()];
            Box::new(SetChannelBlend::new(id, channel, blend, name))
        }
        Op::RenameSet { set, name } => Box::new(RenameTextureSet::new(
            TextureSetId(u32::from(set % sets)),
            name.clone(),
        )),
        Op::Resolution { set, resolution } => Box::new(SetTextureSetResolution::new(
            TextureSetId(u32::from(set % sets)),
            Resolution::ALL[*resolution as usize % Resolution::ALL.len()],
        )),
        Op::FillValue {
            layer,
            channel,
            value,
        } => {
            let (_, id, _, name) = pick(&all, *layer)?;
            let channel = CHANNELS[*channel as usize % CHANNELS.len()];
            Box::new(SetFillValue::new(id, channel, value.map(|v| [v; 4]), name))
        }
        Op::ChannelDef {
            set,
            channel,
            default,
        } => {
            let set = TextureSetId(u32::from(set % sets));
            let channel = CHANNELS[*channel as usize % CHANNELS.len()];
            match default {
                Some(value) => {
                    let mut def = ChannelDef::standard(channel);
                    def.default = [*value; 4];
                    Box::new(SetChannelDef::set(set, def))
                }
                None => Box::new(SetChannelDef::remove(set, channel)),
            }
        }
        Op::Mask { layer, white } => {
            let (_, id, _, name) = pick(&all, *layer)?;
            let mask = white.map(|white| {
                MaskStack::new(if white {
                    MaskInitial::White
                } else {
                    MaskInitial::Black
                })
            });
            Box::new(SetMask::new(id, mask, name))
        }
        Op::MaskInsert {
            layer,
            index,
            kind,
            value,
        } => {
            let (_, id, _, name) = pick(&all, *layer)?;
            let len = mask_effects(project, id).map_or(0, |effects| effects.len());
            let effect = match kind % 4 {
                0 => MaskEffect::uniform(*value),
                1 => MaskEffect::Levels(LevelsParams {
                    gamma: 0.5 + value,
                    ..LevelsParams::default()
                }),
                2 => MaskEffect::paint(),
                _ => MaskEffect::invert(),
            };
            let entry = MaskEffectEntry::new(ids(project).effect(), effect);
            // One past the end is valid; two past it is refused.
            Box::new(InsertMaskEffect::new(
                id,
                *index as usize % (len + 2),
                entry,
                name,
            ))
        }
        Op::MaskRemove { layer, effect } => {
            let (_, id, _, name) = pick(&all, *layer)?;
            let entry = pick(mask_effects(project, id)?, *effect)?;
            Box::new(RemoveMaskEffect::new(id, entry.id, name))
        }
        Op::MaskMove {
            layer,
            effect,
            index,
        } => {
            let (_, id, _, name) = pick(&all, *layer)?;
            let effects = mask_effects(project, id)?;
            let entry = pick(effects, *effect)?;
            Box::new(MoveMaskEffect::new(
                id,
                entry.id,
                *index as usize % (effects.len() + 1),
                name,
            ))
        }
        Op::MaskSet {
            layer,
            effect,
            mode,
            opacity,
            visible,
            value,
        } => {
            let (_, id, _, name) = pick(&all, *layer)?;
            let mut entry = pick(mask_effects(project, id)?, *effect)?;
            entry.blend = MODES[*mode as usize % MODES.len()];
            entry.opacity = *opacity;
            entry.visible = *visible;
            if entry.effect.fill_value().is_some() {
                entry.effect = MaskEffect::uniform(*value);
            }
            Box::new(SetMaskEffect::new(id, entry, name))
        }
        Op::Procedural { layer, scale } => {
            let (_, id, _, name) = pick(&all, *layer)?;
            let source = match scale {
                Some(scale) => FillSource::Procedural(ProceduralRef {
                    id: "noise.perlin".into(),
                    version: 1,
                    params: [("scale".to_owned(), ParamValue::Float(*scale))].into(),
                }),
                None => FillSource::Uniform([(Channel::BaseColor, [0.5; 4])].into()),
            };
            Box::new(SetFillSource::new(id, source, name))
        }
        Op::Projection {
            layer,
            triplanar,
            scale,
        } => {
            let (_, id, _, name) = pick(&all, *layer)?;
            let projection = Projection {
                kind: if *triplanar {
                    ProjectionKind::Triplanar { blend: 0.5 }
                } else {
                    ProjectionKind::Uv
                },
                scale: [*scale; 2],
                ..Projection::default()
            };
            Box::new(SetProjection::new(id, projection, name))
        }
        Op::Stroke {
            target,
            seed,
            eraser,
            alpha,
        } => {
            let (target, name) = pick(&paint_targets(project), *target)?;
            let alpha = alpha.then(|| {
                // Two alphas only, so that strokes often share one.
                patine_core::resource::Resource::new(
                    "alpha.png",
                    patine_core::resource::ResourceKind::Image,
                    std::sync::Arc::from(&[(seed % 2) as u8, 7, 7][..]),
                )
            });
            Box::new(PaintStroke::new(
                target,
                stroke(*seed, *eraser, (*seed % 7) as usize),
                alpha,
                name,
            ))
        }
        Op::HeightForce { set, force } => Box::new(SetHeightForce::new(
            TextureSetId(u32::from(set % sets)),
            *force,
        )),
        Op::Rotation(radians) => Box::new(SetEnvironment::rotation(env, *radians)),
        Op::Exposure(ev) => Box::new(SetEnvironment::exposure(env, *ev)),
        Op::Source(index) => {
            let source = match index % 3 {
                0 => EnvironmentSource::Studio,
                1 => EnvironmentSource::Bundled("a.hdr".into()),
                _ => EnvironmentSource::Bundled("b.hdr".into()),
            };
            Box::new(SetEnvironment::source(env, source))
        }
        Op::Seal | Op::Undo | Op::Redo => return None,
    })
}
