//! Evaluation of a texture set at a resolution other than the working one, for export
//! (M10): strokes are replayed and fills evaluated again at that resolution, in a
//! document and a texture pool of their own, released afterwards. The working document
//! is not touched.

use patine_core::channel::ChannelDef;
use patine_core::project::{Project, Resolution, TextureSetId};
use patine_gpu::pool::TexturePool;
use patine_gpu::{GpuContext, GpuError, readback, wgpu};
use thiserror::Error;

use crate::compositor::Compositor;
use crate::gpu_document::GpuDocument;

/// One final channel of a texture set, read back.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelImage {
    /// The channel.
    pub def: ChannelDef,
    /// Side in pixels.
    pub size: u32,
    /// Texels row by row; a scalar channel repeats its value in the four components.
    pub texels: Vec<[f32; 4]>,
}

/// Why a texture set could not be evaluated.
#[derive(Debug, Error)]
pub enum OfflineError {
    /// The project has no such texture set.
    #[error("no texture set {0:?} in the project")]
    NoTextureSet(TextureSetId),
    /// A channel was not evaluated (it has no texture).
    #[error("channel {0} was not evaluated")]
    NotEvaluated(String),
    /// Reading back failed.
    #[error(transparent)]
    Gpu(#[from] GpuError),
}

/// The final channels of texture set `set` of `project`, evaluated at `resolution`.
///
/// Only that texture set is evaluated, in its own document and pool (peak memory: one
/// texture set at `resolution`). Blocks until the GPU has finished.
pub fn evaluate_set(
    ctx: &GpuContext,
    compositor: &Compositor,
    project: &Project,
    set: TextureSetId,
    resolution: Resolution,
) -> Result<Vec<ChannelImage>, OfflineError> {
    let mut alone = project.clone();
    alone.texture_sets.retain(|candidate| candidate.id == set);
    let Some(texture_set) = alone.texture_sets.first_mut() else {
        return Err(OfflineError::NoTextureSet(set));
    };
    texture_set.resolution = resolution;
    let defs = texture_set.channels.clone();

    let mut pool = TexturePool::new(u64::MAX);
    let mut document = GpuDocument::new();
    document.sync(ctx, &mut pool, compositor, &alone);
    let images = (|| {
        let textures = document
            .texture_set(set)
            .ok_or(OfflineError::NoTextureSet(set))?;
        defs.into_iter()
            .map(|def| {
                let texture = textures
                    .channel(def.channel)
                    .and_then(|channel| pool.peek(channel.texture))
                    .ok_or_else(|| OfflineError::NotEvaluated(def.channel.id()))?;
                let texels = match texture.format() {
                    wgpu::TextureFormat::R16Float => readback::read_r16f_level(ctx, texture, 0, 0)?
                        .into_iter()
                        .map(|v| [v; 4])
                        .collect(),
                    _ => readback::read_rgba16f(ctx, texture)?,
                };
                Ok(ChannelImage {
                    def,
                    size: resolution.pixels(),
                    texels,
                })
            })
            .collect::<Result<Vec<_>, OfflineError>>()
    })();
    document.clear(&mut pool);
    images
}
