//! Property tests of the history: any sequence of commands, undos and redos, followed by
//! undoing everything, gives back the initial document; redoing everything gives back
//! the final one. Every intermediate document is valid.

mod common;

use common::{fill, folder, ids};
use patine_core::channel::Channel;
use patine_core::command::Command;
use patine_core::commands::{
    InsertLayer, MoveLayer, RemoveLayer, RenameLayer, RenameTextureSet, SetChannelBlend,
    SetEnvironment, SetLayerVisible, SetTextureSetResolution,
};
use patine_core::environment::EnvironmentSource;
use patine_core::history::History;
use patine_core::ids::LayerId;
use patine_core::layer::{BlendMode, ChannelBlend, LayerKind, LayerPosition};
use patine_core::project::{Project, Resolution, TextureSetId};
use proptest::prelude::*;

/// A user action, with indices resolved against the document when it runs.
#[derive(Clone, Debug)]
enum Op {
    Insert {
        set: u8,
        parent: u8,
        index: u8,
        folder: bool,
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
    Rotation(f32),
    Exposure(f32),
    Source(u8),
    Seal,
    Undo,
    Redo,
}

fn op() -> impl Strategy<Value = Op> {
    let name = "[a-z ]{0,8}";
    prop_oneof![
        3 => (any::<u8>(), any::<u8>(), any::<u8>(), any::<bool>())
            .prop_map(|(set, parent, index, folder)| Op::Insert { set, parent, index, folder }),
        1 => any::<u8>().prop_map(|layer| Op::Remove { layer }),
        2 => (any::<u8>(), any::<u8>(), any::<u8>())
            .prop_map(|(layer, parent, index)| Op::Move { layer, parent, index }),
        1 => (any::<u8>(), name).prop_map(|(layer, name)| Op::Rename { layer, name }),
        1 => (any::<u8>(), any::<bool>()).prop_map(|(layer, visible)| Op::Visible { layer, visible }),
        2 => (any::<u8>(), any::<u8>(), any::<u8>(), proptest::option::of(0.0f32..=1.0))
            .prop_map(|(layer, channel, mode, opacity)| Op::Blend { layer, channel, mode, opacity }),
        1 => (any::<u8>(), name).prop_map(|(set, name)| Op::RenameSet { set, name }),
        1 => (any::<u8>(), any::<u8>()).prop_map(|(set, resolution)| Op::Resolution { set, resolution }),
        2 => (-10.0f32..10.0).prop_map(Op::Rotation),
        2 => (-6.0f32..6.0).prop_map(Op::Exposure),
        1 => any::<u8>().prop_map(Op::Source),
        2 => Just(Op::Seal),
        2 => Just(Op::Undo),
        1 => Just(Op::Redo),
    ]
}

/// Every layer of the project, with its texture set, in a stable order.
fn layers(project: &Project) -> Vec<(TextureSetId, LayerId, bool, String)> {
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

/// The command for `op`, or `None` when it does not apply to this document.
fn command(project: &Project, op: &Op) -> Option<Box<dyn Command>> {
    let sets = project.texture_sets.len() as u8;
    let all = layers(project);
    let env = &project.environment;
    Some(match op {
        Op::Insert {
            set,
            parent,
            index,
            folder: is_folder,
        } => {
            let set = TextureSetId(u32::from(set % sets));
            let mut ids = ids(project);
            let layer = if *is_folder {
                folder(ids.layer(), "folder")
            } else {
                fill(ids.layer(), "fill")
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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn undo_everything_restores_the_initial_document(ops in proptest::collection::vec(op(), 1..80)) {
        let initial = common::project();
        let mut project = initial.clone();
        let mut history = History::default();
        for op in &ops {
            match op {
                Op::Seal => history.seal(),
                Op::Undo => {
                    history.undo(&mut project).expect("undo");
                }
                Op::Redo => {
                    history.redo(&mut project).expect("redo");
                }
                _ => {
                    if let Some(command) = command(&project, op) {
                        let before = project.clone();
                        if history.push(&mut project, command).is_err() {
                            prop_assert_eq!(&project, &before, "a refused command changed the document");
                        }
                    }
                }
            }
            prop_assert_eq!(project.validate(), Ok(()), "after {:?}", op);
        }

        let final_document = project.clone();
        let applied = history.applied();
        history.go_to(&mut project, 0).expect("undo everything");
        prop_assert_eq!(&project, &initial);
        prop_assert!(!history.is_dirty());
        history.go_to(&mut project, applied).expect("redo everything");
        prop_assert_eq!(&project, &final_document);
    }
}
