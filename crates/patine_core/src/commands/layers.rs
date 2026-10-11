//! Layer stack commands. Layers are designated by their identifier, unique in the
//! project; their place is a [`LayerPosition`] inside one texture set.

use std::any::Any;
use std::collections::HashSet;

use super::Applied;
use crate::channel::Channel;
use crate::command::{Command, CommandError, Label};
use crate::ids::{AnchorId, LayerId};
use crate::layer::{ChannelBlend, FillSource, Layer, LayerKind, LayerPosition};
use crate::mask::MaskEffect;
use crate::project::{Project, TextureSetId};

/// Every node identifier of `layer` and its descendants: layers, anchors and effects.
fn node_ids(layer: &Layer) -> Vec<u64> {
    let mut ids = Vec::new();
    layer.visit(&mut |layer| {
        ids.push(layer.id.0);
        ids.extend(layer.anchor.iter().map(|anchor| anchor.id.0));
        ids.extend(layer.effects.iter().map(|entry| entry.id.0));
        ids.extend(
            layer
                .mask
                .iter()
                .flat_map(|mask| &mask.effects)
                .map(|entry| entry.id.0),
        );
    });
    ids
}

pub(super) fn find_mut(project: &mut Project, id: LayerId) -> Result<&mut Layer, CommandError> {
    project
        .texture_sets
        .iter_mut()
        .find_map(|set| set.stack.find_mut(id))
        .ok_or(CommandError::MissingLayer(id))
}

fn children_mut(
    project: &mut Project,
    texture_set: TextureSetId,
    parent: Option<LayerId>,
) -> Result<&mut Vec<Layer>, CommandError> {
    let set = project
        .texture_set_mut(texture_set)
        .ok_or(CommandError::MissingTextureSet(texture_set.0))?;
    match parent {
        None => Ok(&mut set.stack.layers),
        Some(id) => set
            .stack
            .children_mut(Some(id))
            .ok_or(CommandError::NotAFolder(id)),
    }
}

fn locate(project: &Project, id: LayerId) -> Result<(TextureSetId, LayerPosition), CommandError> {
    project
        .texture_sets
        .iter()
        .find_map(|set| set.stack.position(id).map(|position| (set.id, position)))
        .ok_or(CommandError::MissingLayer(id))
}

/// Removes the layer at `position` and returns it, checking that it is `id`.
fn take(
    project: &mut Project,
    texture_set: TextureSetId,
    position: LayerPosition,
    id: LayerId,
) -> Result<Layer, CommandError> {
    let list = children_mut(project, texture_set, position.parent)?;
    if list.get(position.index).map(|layer| layer.id) != Some(id) {
        return Err(CommandError::StateMismatch(
            "layer not at its recorded position",
        ));
    }
    Ok(list.remove(position.index))
}

/// Moves the layer out of `layer` to `position`, checking the index. On error the layer
/// stays in `layer` and the document is unchanged.
fn put(
    project: &mut Project,
    texture_set: TextureSetId,
    position: LayerPosition,
    layer: &mut Option<Layer>,
) -> Result<(), CommandError> {
    let list = children_mut(project, texture_set, position.parent)?;
    if position.index > list.len() {
        return Err(CommandError::InvalidIndex {
            index: position.index,
            len: list.len(),
        });
    }
    let layer = layer
        .take()
        .ok_or(CommandError::StateMismatch("no layer to insert"))?;
    list.insert(position.index, layer);
    Ok(())
}

/// Adds a new layer (with its children, mask and effects).
///
/// Its identifiers must all be fresh (at least the project's `next_id`, as handed out by
/// [`crate::ids::IdSequence`] starting at `next_id`); `next_id` moves past them.
#[derive(Debug)]
pub struct InsertLayer {
    texture_set: TextureSetId,
    position: LayerPosition,
    /// The layer while not applied; `None` while applied.
    layer: Option<Layer>,
    id: LayerId,
    name: String,
    /// `next_id` before `apply`.
    previous_next_id: u64,
}

impl InsertLayer {
    /// Inserts `layer` at `position` in the texture set.
    pub fn new(texture_set: TextureSetId, position: LayerPosition, layer: Layer) -> Self {
        Self {
            texture_set,
            position,
            id: layer.id,
            name: layer.name.clone(),
            layer: Some(layer),
            previous_next_id: 0,
        }
    }
}

impl Command for InsertLayer {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let layer = self
            .layer
            .as_ref()
            .ok_or(CommandError::StateMismatch("already applied"))?;
        let ids = node_ids(layer);
        let mut unique = HashSet::new();
        if let Some(&id) = ids
            .iter()
            .find(|&&id| id < project.next_id || !unique.insert(id))
        {
            return Err(CommandError::IdNotFresh(id));
        }
        let next_id = ids.iter().max().map_or(project.next_id, |max| max + 1);
        put(project, self.texture_set, self.position, &mut self.layer)?;
        self.previous_next_id = project.next_id;
        project.next_id = next_id;
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError> {
        if self.layer.is_some() {
            return Err(CommandError::StateMismatch("not applied"));
        }
        let layer = take(project, self.texture_set, self.position, self.id)?;
        project.next_id = self.previous_next_id;
        self.layer = Some(layer);
        Ok(())
    }

    fn label(&self) -> Label {
        Label::new("command-add-layer").arg("name", &self.name)
    }

    fn heap_size(&self) -> usize {
        self.layer.as_ref().map_or(0, Layer::heap_size) + self.name.len()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Deletes a layer and its children. Refused while another layer depends on it (an
/// instance of it, or a mask reading one of its anchors).
#[derive(Debug)]
pub struct RemoveLayer {
    id: LayerId,
    /// Where it was and the layer itself, while applied.
    removed: Option<(TextureSetId, LayerPosition, Layer)>,
    name: String,
}

impl RemoveLayer {
    /// Removes the layer `id`. `name` is shown in the history.
    pub fn new(id: LayerId, name: impl Into<String>) -> Self {
        Self {
            id,
            removed: None,
            name: name.into(),
        }
    }
}

/// Whether a layer outside `subtree` depends on a layer or anchor inside it.
fn used_from_outside(project: &Project, subtree: &Layer) -> bool {
    let mut layers = HashSet::new();
    let mut anchors: HashSet<AnchorId> = HashSet::new();
    subtree.visit(&mut |layer| {
        layers.insert(layer.id);
        anchors.extend(layer.anchor.iter().map(|anchor| anchor.id));
    });
    let mut used = false;
    project.visit_layers(&mut |_, layer| {
        if layers.contains(&layer.id) {
            return;
        }
        if let LayerKind::Instance { source } = &layer.kind {
            used |= layers.contains(source);
        }
        for entry in layer.mask.iter().flat_map(|mask| &mask.effects) {
            if let MaskEffect::AnchorRef { anchor, .. } = &entry.effect {
                used |= anchors.contains(anchor);
            }
        }
    });
    used
}

impl Command for RemoveLayer {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        if self.removed.is_some() {
            return Err(CommandError::StateMismatch("already applied"));
        }
        let (texture_set, position) = locate(project, self.id)?;
        let layer = project
            .texture_sets
            .iter()
            .find_map(|set| set.stack.find(self.id))
            .ok_or(CommandError::MissingLayer(self.id))?;
        if used_from_outside(project, layer) {
            return Err(CommandError::LayerInUse(self.id));
        }
        let layer = take(project, texture_set, position, self.id)?;
        self.removed = Some((texture_set, position, layer));
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let (texture_set, position, layer) = self
            .removed
            .take()
            .ok_or(CommandError::StateMismatch("not applied"))?;
        let mut slot = Some(layer);
        let result = put(project, texture_set, position, &mut slot);
        if let Some(layer) = slot {
            self.removed = Some((texture_set, position, layer));
        }
        result
    }

    fn label(&self) -> Label {
        Label::new("command-remove-layer").arg("name", &self.name)
    }

    fn heap_size(&self) -> usize {
        self.removed
            .as_ref()
            .map_or(0, |(_, _, layer)| layer.heap_size())
            + self.name.len()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Moves a layer within its texture set.
#[derive(Debug)]
pub struct MoveLayer {
    id: LayerId,
    /// Destination, with the index counted after the layer was taken out.
    to: LayerPosition,
    /// Texture set and original position, while applied.
    from: Option<(TextureSetId, LayerPosition)>,
    name: String,
}

impl MoveLayer {
    /// Moves the layer `id` to `to`: inside folder `to.parent` (or the top level), at
    /// index `to.index` among the remaining layers there.
    pub fn new(id: LayerId, to: LayerPosition, name: impl Into<String>) -> Self {
        Self {
            id,
            to,
            from: None,
            name: name.into(),
        }
    }
}

impl Command for MoveLayer {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        if self.from.is_some() {
            return Err(CommandError::StateMismatch("already applied"));
        }
        let (texture_set, from) = locate(project, self.id)?;
        if let Some(parent) = self.to.parent {
            let set = project
                .texture_set(texture_set)
                .ok_or(CommandError::MissingTextureSet(texture_set.0))?;
            let moved = set
                .stack
                .find(self.id)
                .ok_or(CommandError::MissingLayer(self.id))?;
            if moved.contains(parent) {
                return Err(CommandError::MoveIntoItself(self.id));
            }
            if set.stack.children(Some(parent)).is_none() {
                return Err(CommandError::NotAFolder(parent));
            }
        }
        let mut slot = Some(take(project, texture_set, from, self.id)?);
        if let Err(error) = put(project, texture_set, self.to, &mut slot) {
            put(project, texture_set, from, &mut slot)
                .map_err(|_| CommandError::StateMismatch("cannot restore the layer"))?;
            return Err(error);
        }
        self.from = Some((texture_set, from));
        Ok(())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let (texture_set, from) = self
            .from
            .ok_or(CommandError::StateMismatch("not applied"))?;
        let mut slot = Some(take(project, texture_set, self.to, self.id)?);
        if let Err(error) = put(project, texture_set, from, &mut slot) {
            put(project, texture_set, self.to, &mut slot)
                .map_err(|_| CommandError::StateMismatch("cannot restore the layer"))?;
            return Err(error);
        }
        self.from = None;
        Ok(())
    }

    fn label(&self) -> Label {
        Label::new("command-move-layer").arg("name", &self.name)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Renames a layer.
#[derive(Debug)]
pub struct RenameLayer {
    id: LayerId,
    /// New name before `apply`, old name after.
    name: String,
    label_name: String,
    applied: Applied,
}

impl RenameLayer {
    /// Renames the layer `id` to `name`.
    pub fn new(id: LayerId, name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            id,
            label_name: name.clone(),
            name,
            applied: Applied::default(),
        }
    }

    fn swap(&mut self, project: &mut Project, applied: bool) -> Result<(), CommandError> {
        let layer = find_mut(project, self.id)?;
        self.applied.set(applied)?;
        std::mem::swap(&mut layer.name, &mut self.name);
        Ok(())
    }
}

impl Command for RenameLayer {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, true)
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, false)
    }

    fn label(&self) -> Label {
        Label::new("command-rename-layer").arg("name", &self.label_name)
    }

    fn heap_size(&self) -> usize {
        self.name.len() + self.label_name.len()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Shows or hides a layer.
#[derive(Debug)]
pub struct SetLayerVisible {
    id: LayerId,
    /// New visibility before `apply`, old one after.
    visible: bool,
    label_visible: bool,
    name: String,
    applied: Applied,
}

impl SetLayerVisible {
    /// Sets the visibility of the layer `id`. `name` is shown in the history.
    pub fn new(id: LayerId, visible: bool, name: impl Into<String>) -> Self {
        Self {
            id,
            visible,
            label_visible: visible,
            name: name.into(),
            applied: Applied::default(),
        }
    }

    fn swap(&mut self, project: &mut Project, applied: bool) -> Result<(), CommandError> {
        let layer = find_mut(project, self.id)?;
        self.applied.set(applied)?;
        std::mem::swap(&mut layer.visible, &mut self.visible);
        Ok(())
    }
}

impl Command for SetLayerVisible {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, true)
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, false)
    }

    fn label(&self) -> Label {
        Label::new(if self.label_visible {
            "command-show-layer"
        } else {
            "command-hide-layer"
        })
        .arg("name", &self.name)
    }

    fn heap_size(&self) -> usize {
        self.name.len()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Sets the blend mode and opacity of one channel of a layer, or stops the layer from
/// affecting that channel (`None`). Continuous changes of the same channel merge.
#[derive(Debug)]
pub struct SetChannelBlend {
    id: LayerId,
    channel: Channel,
    /// New blend before `apply`, old one after.
    blend: Option<ChannelBlend>,
    name: String,
    applied: Applied,
}

impl SetChannelBlend {
    /// Sets `channel` of the layer `id`. `name` is shown in the history.
    pub fn new(
        id: LayerId,
        channel: Channel,
        blend: Option<ChannelBlend>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            id,
            channel,
            blend,
            name: name.into(),
            applied: Applied::default(),
        }
    }

    fn swap(&mut self, project: &mut Project, applied: bool) -> Result<(), CommandError> {
        let layer = find_mut(project, self.id)?;
        self.applied.set(applied)?;
        let previous = match self.blend.take() {
            Some(blend) => layer.channels.insert(self.channel, blend),
            None => layer.channels.remove(&self.channel),
        };
        self.blend = previous;
        Ok(())
    }
}

impl Command for SetChannelBlend {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, true)
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, false)
    }

    fn label(&self) -> Label {
        Label::new("command-channel-blend")
            .arg("name", &self.name)
            .arg("channel", self.channel.id())
    }

    fn merge(&mut self, next: &dyn Command) -> bool {
        next.as_any()
            .downcast_ref::<SetChannelBlend>()
            .is_some_and(|next| next.id == self.id && next.channel == self.channel)
    }

    fn heap_size(&self) -> usize {
        self.name.len()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Sets the value a uniform fill gives to one channel, or removes it (`None`: the fill no
/// longer contributes to that channel). Continuous changes of the same channel merge.
#[derive(Debug)]
pub struct SetFillValue {
    id: LayerId,
    channel: Channel,
    /// New value before `apply`, old one after.
    value: Option<[f32; 4]>,
    name: String,
    applied: Applied,
}

impl SetFillValue {
    /// Sets `channel` of the fill layer `id`. `name` is shown in the history.
    pub fn new(
        id: LayerId,
        channel: Channel,
        value: Option<[f32; 4]>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            id,
            channel,
            value,
            name: name.into(),
            applied: Applied::default(),
        }
    }

    fn swap(&mut self, project: &mut Project, applied: bool) -> Result<(), CommandError> {
        let layer = find_mut(project, self.id)?;
        let LayerKind::Fill {
            source: FillSource::Uniform(values),
            ..
        } = &mut layer.kind
        else {
            return Err(CommandError::NotAUniformFill(self.id));
        };
        self.applied.set(applied)?;
        let previous = match self.value.take() {
            Some(value) => values.insert(self.channel, value),
            None => values.remove(&self.channel),
        };
        self.value = previous;
        Ok(())
    }
}

impl Command for SetFillValue {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, true)
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError> {
        self.swap(project, false)
    }

    fn label(&self) -> Label {
        Label::new("command-fill-value")
            .arg("name", &self.name)
            .arg("channel", self.channel.id())
    }

    fn merge(&mut self, next: &dyn Command) -> bool {
        next.as_any()
            .downcast_ref::<SetFillValue>()
            .is_some_and(|next| next.id == self.id && next.channel == self.channel)
    }

    fn heap_size(&self) -> usize {
        self.name.len()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
