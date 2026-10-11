//! Layers and layer stacks.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::channel::{Channel, ChannelMap};
use crate::ids::{AnchorId, LayerId};
use crate::mask::{EffectEntry, MaskStack, Params};
use crate::resource::ResourceHash;
use crate::stroke::StrokeLog;

/// How a layer combines with what is below it, per channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[allow(missing_docs)] // The usual compositing modes; see the compositor section of the plan.
pub enum BlendMode {
    Normal,
    Replace,
    /// Folders only: children blend directly into the parent stack.
    Passthrough,
    Multiply,
    Divide,
    Screen,
    Overlay,
    SoftLight,
    HardLight,
    /// Linear dodge (add).
    LinearDodge,
    LinearBurn,
    Subtract,
    Difference,
    Exclusion,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    Hue,
    Saturation,
    Color,
    Luminosity,
    /// The channel is not affected.
    Disable,
    /// Normal channel: reoriented normal mapping (default for normals).
    NormalMapCombine,
    /// Normal channel: UDN detail blending.
    NormalMapDetail,
}

/// Blend mode and opacity of one channel of a layer.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelBlend {
    /// Blend mode.
    pub mode: BlendMode,
    /// Opacity, 0 to 1.
    pub opacity: f32,
}

impl ChannelBlend {
    /// Full-opacity blend with the usual mode for `channel`.
    pub fn standard(channel: Channel) -> Self {
        Self {
            mode: if channel == Channel::Normal {
                BlendMode::NormalMapCombine
            } else {
                BlendMode::Normal
            },
            opacity: 1.0,
        }
    }
}

/// A procedural material or pattern, referenced by its library identifier (M7).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProceduralRef {
    /// Identifier in the library.
    pub id: String,
    /// Version of its definition, to migrate parameters.
    pub version: u32,
    /// Parameter values.
    pub params: Params,
}

/// Content of a fill layer or fill mask effect.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FillSource {
    /// One value per channel (RGBA, scalars in the first component).
    Uniform(ChannelMap<[f32; 4]>),
    /// An imported image.
    Image {
        /// The image resource.
        resource: ResourceHash,
    },
    /// A procedural material or pattern.
    Procedural(ProceduralRef),
}

/// How a fill is mapped onto the mesh.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ProjectionKind {
    /// Mesh UVs.
    Uv,
    /// Three planar projections blended by the normal.
    Triplanar {
        /// Blend sharpness between the planes, 0 to 1.
        blend: f32,
    },
    /// One plane.
    Planar,
    /// Sphere around the mesh.
    Spherical,
    /// Cylinder around the Y axis.
    Cylindrical,
}

/// Projection with its transform.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Projection {
    /// Mapping.
    pub kind: ProjectionKind,
    /// Scale (tiling).
    pub scale: [f32; 2],
    /// Rotation, radians.
    pub rotation: f32,
    /// Offset.
    pub offset: [f32; 2],
}

impl Default for Projection {
    fn default() -> Self {
        Self {
            kind: ProjectionKind::Uv,
            scale: [1.0; 2],
            rotation: 0.0,
            offset: [0.0; 2],
        }
    }
}

/// What a layer contains.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LayerKind {
    /// Painted strokes; stored next to the manifest, not inside it.
    Paint {
        /// The strokes.
        #[serde(skip)]
        strokes: Arc<StrokeLog>,
    },
    /// Uniform value, image or procedural material.
    Fill {
        /// Content.
        source: FillSource,
        /// Mapping onto the mesh.
        projection: Projection,
    },
    /// Group of layers, bottom to top.
    Folder {
        /// Children, bottom to top.
        children: Vec<Layer>,
    },
    /// Replays another layer, possibly of another texture set.
    Instance {
        /// The instanced layer.
        source: LayerId,
    },
}

/// Exposes the composition right after a layer, for masks higher in the stack.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    /// Stable identifier.
    pub id: AnchorId,
    /// Name shown in the interface.
    pub name: String,
    /// Captured channels.
    pub channels: Vec<Channel>,
}

/// A layer of a texture set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// Stable identifier.
    pub id: LayerId,
    /// Name shown in the interface.
    pub name: String,
    /// Whether the layer is evaluated.
    pub visible: bool,
    /// Content.
    pub kind: LayerKind,
    /// Blend mode and opacity of each channel the layer affects.
    pub channels: ChannelMap<ChannelBlend>,
    /// Mask, if any.
    pub mask: Option<MaskStack>,
    /// Effects applied to the content, bottom to top.
    pub effects: Vec<EffectEntry>,
    /// Anchor point, if any.
    pub anchor: Option<Anchor>,
}

impl Layer {
    /// A visible layer without mask, effects or anchor, affecting `channels` with their
    /// standard blending.
    pub fn new(
        id: LayerId,
        name: impl Into<String>,
        kind: LayerKind,
        channels: &[Channel],
    ) -> Self {
        Self {
            id,
            name: name.into(),
            visible: true,
            kind,
            channels: channels
                .iter()
                .map(|&channel| (channel, ChannelBlend::standard(channel)))
                .collect(),
            mask: None,
            effects: Vec::new(),
            anchor: None,
        }
    }

    /// Children of a folder; empty for other layers.
    pub fn children(&self) -> &[Layer] {
        match &self.kind {
            LayerKind::Folder { children } => children,
            _ => &[],
        }
    }

    /// Calls `f` on this layer and every descendant, depth first.
    pub fn visit<'a>(&'a self, f: &mut impl FnMut(&'a Layer)) {
        f(self);
        for child in self.children() {
            child.visit(f);
        }
    }

    /// Calls `f` on this layer and every descendant, depth first, mutably.
    pub fn visit_mut(&mut self, f: &mut impl FnMut(&mut Layer)) {
        f(self);
        if let LayerKind::Folder { children } = &mut self.kind {
            for child in children {
                child.visit_mut(f);
            }
        }
    }

    /// Whether `id` is this layer or one of its descendants.
    pub fn contains(&self, id: LayerId) -> bool {
        let mut found = false;
        self.visit(&mut |layer| found |= layer.id == id);
        found
    }

    /// Approximate heap memory of the layer and its descendants, for history budgets.
    pub fn heap_size(&self) -> usize {
        let mut size = 0;
        self.visit(&mut |layer| {
            size += std::mem::size_of::<Layer>() + layer.name.len();
            if let LayerKind::Paint { strokes } = &layer.kind {
                size += strokes.heap_size();
            }
            if let Some(mask) = &layer.mask {
                size += mask.effects.len() * 64;
            }
        });
        size
    }
}

/// Where a layer sits: its parent folder (`None` at the top level) and its index there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayerPosition {
    /// Parent folder, `None` for the stack itself.
    pub parent: Option<LayerId>,
    /// Index among the parent's children, 0 = bottom.
    pub index: usize,
}

/// The layers of a texture set, bottom to top.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LayerStack {
    /// Top-level layers, bottom to top.
    pub layers: Vec<Layer>,
}

impl LayerStack {
    /// Calls `f` on every layer, depth first.
    pub fn visit<'a>(&'a self, f: &mut impl FnMut(&'a Layer)) {
        for layer in &self.layers {
            layer.visit(f);
        }
    }

    /// Calls `f` on every layer, depth first, mutably.
    pub fn visit_mut(&mut self, f: &mut impl FnMut(&mut Layer)) {
        for layer in &mut self.layers {
            layer.visit_mut(f);
        }
    }

    /// The layer `id`, if it is in this stack.
    pub fn find(&self, id: LayerId) -> Option<&Layer> {
        let mut found = None;
        self.visit(&mut |layer| {
            if layer.id == id {
                found = Some(layer);
            }
        });
        found
    }

    /// The layer `id`, mutably.
    pub fn find_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        fn search(layers: &mut [Layer], id: LayerId) -> Option<&mut Layer> {
            for layer in layers {
                if layer.id == id {
                    return Some(layer);
                }
                if let LayerKind::Folder { children } = &mut layer.kind
                    && let Some(found) = search(children, id)
                {
                    return Some(found);
                }
            }
            None
        }
        search(&mut self.layers, id)
    }

    /// Children of `parent` (`None` = top level), if `parent` is a folder of this stack.
    pub fn children(&self, parent: Option<LayerId>) -> Option<&Vec<Layer>> {
        match parent {
            None => Some(&self.layers),
            Some(id) => match &self.find(id)?.kind {
                LayerKind::Folder { children } => Some(children),
                _ => None,
            },
        }
    }

    /// Children of `parent`, mutably.
    pub fn children_mut(&mut self, parent: Option<LayerId>) -> Option<&mut Vec<Layer>> {
        match parent {
            None => Some(&mut self.layers),
            Some(id) => match &mut self.find_mut(id)?.kind {
                LayerKind::Folder { children } => Some(children),
                _ => None,
            },
        }
    }

    /// Position of the layer `id`, if it is in this stack.
    pub fn position(&self, id: LayerId) -> Option<LayerPosition> {
        fn search(layers: &[Layer], parent: Option<LayerId>, id: LayerId) -> Option<LayerPosition> {
            for (index, layer) in layers.iter().enumerate() {
                if layer.id == id {
                    return Some(LayerPosition { parent, index });
                }
                if let Some(found) = search(layer.children(), Some(layer.id), id) {
                    return Some(found);
                }
            }
            None
        }
        search(&self.layers, None, id)
    }
}
