//! Behavior of each command and of the history.

mod common;

use std::sync::Arc;

use common::{fill, folder, ids, project};
use patine_core::channel::Channel;
use patine_core::command::{Command, CommandError};
use patine_core::commands::{
    InsertLayer, MoveLayer, RemoveLayer, RenameLayer, RenameTextureSet, SetChannelBlend,
    SetEnvironment, SetLayerVisible, SetTextureSetResolution,
};
use patine_core::environment::EnvironmentSource;
use patine_core::history::History;
use patine_core::ids::LayerId;
use patine_core::layer::{BlendMode, ChannelBlend, LayerKind, LayerPosition};
use patine_core::project::{Project, Resolution, TextureSetId};
use patine_core::resource::{Resource, ResourceKind};

const SET: TextureSetId = TextureSetId(0);

fn top(index: usize) -> LayerPosition {
    LayerPosition {
        parent: None,
        index,
    }
}

/// Applies `command`, checks that revert restores `before`, and that applying again
/// gives the same result. Returns the project after the command.
fn round_trip(before: &Project, mut command: Box<dyn Command>) -> Project {
    let mut project = before.clone();
    command.apply(&mut project).expect("apply");
    let after = project.clone();
    project.validate().expect("valid after apply");
    assert_eq!(
        command.apply(&mut project),
        Err(CommandError::StateMismatch("already applied")),
        "{command:?}"
    );
    command.revert(&mut project).expect("revert");
    assert_eq!(&project, before, "revert of {command:?}");
    assert!(command.revert(&mut project).is_err(), "{command:?}");
    command.apply(&mut project).expect("redo");
    assert_eq!(project, after, "redo of {command:?}");
    after
}

/// A project with a folder (id 1) holding a fill (id 2), and a fill (id 3) above it.
fn layered() -> Project {
    let mut project = project();
    let mut ids = ids(&project);
    let mut group = folder(ids.layer(), "group");
    let inner = fill(ids.layer(), "inner");
    if let LayerKind::Folder { children } = &mut group.kind {
        children.push(inner);
    }
    let outer = fill(ids.layer(), "outer");
    for (index, layer) in [group, outer].into_iter().enumerate() {
        InsertLayer::new(SET, top(index), layer)
            .apply(&mut project)
            .expect("insert");
    }
    assert_eq!(project.next_id, 4);
    project
}

#[test]
fn texture_set_commands_round_trip() {
    let project = project();
    let after = round_trip(&project, Box::new(RenameTextureSet::new(SET, "body")));
    assert_eq!(after.texture_sets[0].name, "body");
    let resolution = Resolution::new(4096).unwrap();
    let after = round_trip(
        &project,
        Box::new(SetTextureSetResolution::new(SET, resolution)),
    );
    assert_eq!(after.texture_sets[0].resolution, resolution);
    let mut missing = RenameTextureSet::new(TextureSetId(7), "x");
    assert_eq!(
        missing.apply(&mut project.clone()),
        Err(CommandError::MissingTextureSet(7))
    );
}

#[test]
fn environment_commands_round_trip() {
    let project = project();
    let env = &project.environment;
    let after = round_trip(&project, Box::new(SetEnvironment::rotation(env, 7.0)));
    assert!((after.environment.rotation - (7.0 - std::f32::consts::TAU)).abs() < 1e-6);
    round_trip(&project, Box::new(SetEnvironment::exposure(env, -1.5)));
    let bundled = EnvironmentSource::Bundled("studio_small_08_1k.hdr".into());
    round_trip(&project, Box::new(SetEnvironment::source(env, bundled)));
}

#[test]
fn imported_environment_stores_its_resource_once() {
    let project = project();
    let (hash, resource) = Resource::new(
        "sky.hdr",
        ResourceKind::Environment,
        Arc::from(&b"#?RADIANCE"[..]),
    );
    let command = SetEnvironment::imported(&project.environment, hash, resource.clone());
    let after = round_trip(&project, Box::new(command));
    assert_eq!(after.environment.source, EnvironmentSource::Resource(hash));
    assert_eq!(after.resources.get(&hash), Some(&resource));

    // Importing it again does not remove it on undo: the first import still uses it.
    let mut again = SetEnvironment::imported(&after.environment, hash, resource.clone());
    let mut project = after.clone();
    again.apply(&mut project).unwrap();
    again.revert(&mut project).unwrap();
    assert_eq!(project, after);

    // Bytes that do not match the hash are refused.
    let forged = Resource {
        bytes: Arc::from(&b"other"[..]),
        ..resource
    };
    let mut command = SetEnvironment::imported(&project.environment, hash, forged);
    assert_eq!(
        command.apply(&mut project.clone()),
        Err(CommandError::ResourceHash)
    );
}

#[test]
fn insert_layer_takes_fresh_ids() {
    let project = layered();
    let mut ids = ids(&project);
    let after = round_trip(
        &project,
        Box::new(InsertLayer::new(
            SET,
            LayerPosition {
                parent: Some(LayerId(1)),
                index: 0,
            },
            fill(ids.layer(), "new"),
        )),
    );
    assert_eq!(after.next_id, 5);
    let group = after.texture_sets[0].stack.find(LayerId(1)).unwrap();
    assert_eq!(group.children()[0].id, LayerId(4));

    let mut reused = InsertLayer::new(SET, top(0), fill(LayerId(2), "dup"));
    assert_eq!(
        reused.apply(&mut project.clone()),
        Err(CommandError::IdNotFresh(2))
    );
    let mut past_end = InsertLayer::new(SET, top(5), fill(LayerId(9), "far"));
    assert_eq!(
        past_end.apply(&mut project.clone()),
        Err(CommandError::InvalidIndex { index: 5, len: 2 })
    );
    let mut into_fill = InsertLayer::new(
        SET,
        LayerPosition {
            parent: Some(LayerId(3)),
            index: 0,
        },
        fill(LayerId(9), "x"),
    );
    assert_eq!(
        into_fill.apply(&mut project.clone()),
        Err(CommandError::NotAFolder(LayerId(3)))
    );
}

#[test]
fn remove_layer_restores_position_and_children() {
    let project = layered();
    let after = round_trip(&project, Box::new(RemoveLayer::new(LayerId(1), "group")));
    assert_eq!(after.texture_sets[0].stack.layers.len(), 1);
    assert!(after.texture_sets[0].stack.find(LayerId(2)).is_none());

    // An instance in the other texture set keeps its source alive.
    let mut project = project;
    let mut instance = fill(LayerId(4), "instance");
    instance.kind = LayerKind::Instance { source: LayerId(2) };
    InsertLayer::new(TextureSetId(1), top(0), instance)
        .apply(&mut project)
        .unwrap();
    project.validate().unwrap();
    let mut remove = RemoveLayer::new(LayerId(1), "group");
    assert_eq!(
        remove.apply(&mut project.clone()),
        Err(CommandError::LayerInUse(LayerId(1)))
    );
}

#[test]
fn move_layer_in_and_out_of_folders() {
    let project = layered();
    let into_group = MoveLayer::new(
        LayerId(3),
        LayerPosition {
            parent: Some(LayerId(1)),
            index: 1,
        },
        "outer",
    );
    let after = round_trip(&project, Box::new(into_group));
    let group = after.texture_sets[0].stack.find(LayerId(1)).unwrap();
    let children: Vec<_> = group.children().iter().map(|l| l.id).collect();
    assert_eq!(children, [LayerId(2), LayerId(3)]);

    let to_bottom = MoveLayer::new(LayerId(3), top(0), "outer");
    let after = round_trip(&project, Box::new(to_bottom));
    assert_eq!(after.texture_sets[0].stack.layers[0].id, LayerId(3));

    let mut into_itself = MoveLayer::new(
        LayerId(1),
        LayerPosition {
            parent: Some(LayerId(1)),
            index: 0,
        },
        "group",
    );
    assert_eq!(
        into_itself.apply(&mut project.clone()),
        Err(CommandError::MoveIntoItself(LayerId(1)))
    );
    let mut too_far = MoveLayer::new(LayerId(3), top(2), "outer");
    let mut unchanged = project.clone();
    assert_eq!(
        too_far.apply(&mut unchanged),
        Err(CommandError::InvalidIndex { index: 2, len: 1 })
    );
    assert_eq!(
        unchanged, project,
        "a refused move leaves the document as it was"
    );
}

#[test]
fn layer_property_commands_round_trip() {
    let project = layered();
    let after = round_trip(&project, Box::new(RenameLayer::new(LayerId(2), "rust")));
    assert_eq!(
        after.texture_sets[0].stack.find(LayerId(2)).unwrap().name,
        "rust"
    );
    let after = round_trip(
        &project,
        Box::new(SetLayerVisible::new(LayerId(3), false, "outer")),
    );
    assert!(
        !after.texture_sets[0]
            .stack
            .find(LayerId(3))
            .unwrap()
            .visible
    );
    let blend = ChannelBlend {
        mode: BlendMode::Multiply,
        opacity: 0.4,
    };
    for value in [Some(blend), None] {
        let command = SetChannelBlend::new(LayerId(3), Channel::Roughness, value, "outer");
        let after = round_trip(&project, Box::new(command));
        let layer = after.texture_sets[0].stack.find(LayerId(3)).unwrap();
        assert_eq!(layer.channels.get(&Channel::Roughness), value.as_ref());
    }
    let mut missing = RenameLayer::new(LayerId(42), "x");
    assert_eq!(
        missing.apply(&mut project.clone()),
        Err(CommandError::MissingLayer(LayerId(42)))
    );
}

#[test]
fn a_gesture_makes_one_entry_until_sealed() {
    let mut project = project();
    let mut history = History::default();
    for step in 1..=10 {
        let command = SetEnvironment::rotation(&project.environment, step as f32 * 0.1);
        history.push(&mut project, Box::new(command)).unwrap();
    }
    assert_eq!(history.applied(), 1);
    history.seal();
    let command = SetEnvironment::rotation(&project.environment, 2.0);
    history.push(&mut project, Box::new(command)).unwrap();
    // Another field does not merge either.
    let command = SetEnvironment::exposure(&project.environment, 1.0);
    history.push(&mut project, Box::new(command)).unwrap();
    assert_eq!(history.applied(), 3);

    history.undo(&mut project).unwrap();
    history.undo(&mut project).unwrap();
    assert!((project.environment.rotation - 1.0).abs() < 1e-6);
    history.undo(&mut project).unwrap();
    assert_eq!(project.environment.rotation, 0.0);
    assert_eq!(history.undo(&mut project), Ok(false));
}

#[test]
fn undo_after_redo_and_new_command_drops_redo() {
    let mut project = project();
    let initial = project.clone();
    let mut history = History::default();
    for name in ["a", "b", "c"] {
        history.seal();
        history
            .push(&mut project, Box::new(RenameTextureSet::new(SET, name)))
            .unwrap();
    }
    history.go_to(&mut project, 1).unwrap();
    assert_eq!(project.texture_sets[0].name, "a");
    assert_eq!(history.labels().len(), 3);
    history.go_to(&mut project, 3).unwrap();
    assert_eq!(project.texture_sets[0].name, "c");
    history.go_to(&mut project, 0).unwrap();
    assert_eq!(project, initial);
    assert!(history.can_redo());
    history
        .push(&mut project, Box::new(RenameTextureSet::new(SET, "d")))
        .unwrap();
    assert!(!history.can_redo());
    assert_eq!(history.labels().len(), 1);
}

#[test]
fn refused_commands_change_nothing() {
    let mut project = project();
    let before = project.clone();
    let mut history = History::default();
    let error = history.push(
        &mut project,
        Box::new(RenameTextureSet::new(TextureSetId(9), "x")),
    );
    assert_eq!(error, Err(CommandError::MissingTextureSet(9)));
    assert_eq!(project, before);
    assert!(!history.can_undo());
    assert!(!history.is_dirty());
}

#[test]
fn dirty_follows_the_saved_state() {
    let mut project = project();
    let mut history = History::default();
    assert!(!history.is_dirty());
    let rotate =
        |project: &Project, value| Box::new(SetEnvironment::rotation(&project.environment, value));
    let command = rotate(&project, 0.5);
    history.push(&mut project, command).unwrap();
    assert!(history.is_dirty());
    history.mark_saved();
    assert!(!history.is_dirty());
    // Saving seals the gesture: continuing it starts a new entry, so the saved state
    // stays reachable by undo.
    let command = rotate(&project, 0.6);
    history.push(&mut project, command).unwrap();
    assert_eq!(history.applied(), 2);
    assert!(history.is_dirty());
    history.undo(&mut project).unwrap();
    assert!(!history.is_dirty(), "back to the saved state");
    history.undo(&mut project).unwrap();
    assert!(history.is_dirty(), "before the saved state");
    history.redo(&mut project).unwrap();
    history.redo(&mut project).unwrap();
    assert!(history.is_dirty());

    // A merge gives the entry a new state: the save made before the merge no longer
    // matches it.
    history.mark_saved();
    let mut merged = History::default();
    let command = rotate(&project, 1.0);
    merged.push(&mut project, command).unwrap();
    let saved_state = merged.current_state();
    let command = rotate(&project, 1.1);
    merged.push(&mut project, command).unwrap();
    assert_eq!(merged.applied(), 1);
    assert_ne!(merged.current_state(), saved_state);
    history.mark_unsaved();
    assert!(history.is_dirty());
}

#[test]
fn budget_evicts_the_oldest_entries() {
    let mut project = project();
    // Each rename costs its two names plus the fixed entry overhead (64 bytes).
    let mut history = History::with_budget(300);
    for index in 0..10 {
        history.seal();
        let name = format!("name {index:02}");
        history
            .push(&mut project, Box::new(RenameTextureSet::new(SET, name)))
            .unwrap();
    }
    assert!(history.memory() <= 300, "{}", history.memory());
    assert!(history.applied() < 10 && history.applied() >= 1);
    assert!(history.truncated());
    while history.undo(&mut project).unwrap() {}
    // The oldest reachable state is the one after the evicted renames.
    assert_ne!(project.texture_sets[0].name, "paint");
    assert!(history.is_dirty());
}
