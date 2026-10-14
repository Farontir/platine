//! Texture pool: every large texture of the document goes through it, so that GPU memory
//! is counted and kept under a budget.
//!
//! Textures are either *pinned* (never evicted: the final channels of the document) or
//! *discardable* (caches that can be computed again). When the discardable textures
//! push the total over the budget, the least recently used ones are destroyed; their
//! owner finds out through [`TexturePool::get`] returning `None`, and recomputes them.
//! Offloading textures to compressed CPU memory comes with the painted layers (M8).
//!
//! One command encoder records many passes before its submit, allocating along the way.
//! So that a pass never loses a texture recorded earlier in the same encoder:
//! - a texture allocated or used ([`TexturePool::get`]) since the last
//!   [`TexturePool::flush`] is never evicted: the pool may go over budget while
//!   recording, and gets back under it at the flush;
//! - released textures leave the accounting at once, but are only destroyed by the flush.
//!
//! The owner of the encoder calls the flush right after its submit.

use std::collections::HashMap;

use crate::GpuContext;

/// Identifier of a texture in a [`TexturePool`], never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureId(u64);

/// Whether a texture may be evicted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Residency {
    /// Never evicted: its content exists nowhere else.
    Pinned,
    /// May be destroyed when over budget: its owner can compute it again.
    Discardable,
}

#[derive(Debug)]
struct Entry {
    texture: wgpu::Texture,
    bytes: u64,
    residency: Residency,
    /// Logical time of the last use, for the LRU order.
    last_use: u64,
    /// Flush count at the last use: the texture is busy while it equals the pool's.
    epoch: u64,
}

/// Memory taken by a texture described by `desc`, mips and layers included, in bytes.
/// Compressed and depth-stencil formats are counted by their copy size when it exists,
/// otherwise as 4 bytes per texel.
pub fn texture_bytes(desc: &wgpu::TextureDescriptor<'_>) -> u64 {
    let texel = u64::from(desc.format.block_copy_size(None).unwrap_or(4));
    let layers = u64::from(desc.size.depth_or_array_layers.max(1));
    let samples = u64::from(desc.sample_count.max(1));
    (0..desc.mip_level_count)
        .map(|level| {
            let width = u64::from((desc.size.width >> level).max(1));
            let height = u64::from((desc.size.height >> level).max(1));
            width * height
        })
        .sum::<u64>()
        * texel
        * layers
        * samples
}

/// Number of mip levels of a full chain for a `size`² texture.
pub fn full_mip_count(size: u32) -> u32 {
    u32::BITS - size.max(1).leading_zeros()
}

/// Memory actually allocated by wgpu for every resource of the device, when the backend
/// reports it (DX12 and Vulkan do).
pub fn allocated_bytes(ctx: &GpuContext) -> Option<u64> {
    ctx.device
        .generate_allocator_report()
        .map(|report| report.total_allocated_bytes)
}

/// The textures of the document, with their memory accounting.
#[derive(Debug)]
pub struct TexturePool {
    entries: HashMap<TextureId, Entry>,
    next_id: u64,
    clock: u64,
    budget: u64,
    memory: u64,
    evicted: u64,
    /// Number of flushes so far.
    epoch: u64,
    /// Released or evicted, not yet destroyed: commands not submitted yet may use them.
    pending: Vec<wgpu::Texture>,
}

impl TexturePool {
    /// An empty pool. Discardable textures are evicted to stay under `budget` bytes.
    pub fn new(budget: u64) -> Self {
        Self {
            entries: HashMap::new(),
            next_id: 0,
            clock: 0,
            budget,
            memory: 0,
            evicted: 0,
            epoch: 0,
            pending: Vec::new(),
        }
    }

    /// Creates a texture and counts it. Allocating may evict discardable textures that
    /// were not used since the last flush.
    pub fn allocate(
        &mut self,
        ctx: &GpuContext,
        desc: &wgpu::TextureDescriptor<'_>,
        residency: Residency,
    ) -> TextureId {
        let id = TextureId(self.next_id);
        self.next_id += 1;
        let bytes = texture_bytes(desc);
        self.clock += 1;
        self.entries.insert(
            id,
            Entry {
                texture: ctx.device.create_texture(desc),
                bytes,
                residency,
                last_use: self.clock,
                epoch: self.epoch,
            },
        );
        self.memory += bytes;
        self.evict();
        id
    }

    /// The texture `id`, marked as just used; `None` if it was released or evicted.
    pub fn get(&mut self, id: TextureId) -> Option<&wgpu::Texture> {
        self.clock += 1;
        let entry = self.entries.get_mut(&id)?;
        entry.last_use = self.clock;
        entry.epoch = self.epoch;
        Some(&entry.texture)
    }

    /// The texture `id`, without marking it as used.
    pub fn peek(&self, id: TextureId) -> Option<&wgpu::Texture> {
        self.entries.get(&id).map(|entry| &entry.texture)
    }

    /// Removes the texture `id` from the pool and its memory from the accounting; it is
    /// destroyed by the next [`TexturePool::flush`]. Releasing an unknown texture does
    /// nothing.
    pub fn release(&mut self, id: TextureId) {
        if let Some(entry) = self.entries.remove(&id) {
            self.memory -= entry.bytes;
            self.pending.push(entry.texture);
        }
    }

    /// Ends a recording: call it once the commands recorded since the last flush have
    /// been submitted. Every texture becomes evictable again, the pool evicts down to its
    /// budget, and the released and evicted textures are destroyed (wgpu frees them when
    /// the GPU is done with them).
    pub fn flush(&mut self) {
        self.epoch += 1;
        self.evict();
        for texture in self.pending.drain(..) {
            texture.destroy();
        }
    }

    /// Number of released or evicted textures waiting for [`TexturePool::flush`].
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Memory of the live textures, in bytes.
    pub fn memory(&self) -> u64 {
        self.memory
    }

    /// Number of live textures.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the pool holds no texture.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Budget in bytes.
    pub fn budget(&self) -> u64 {
        self.budget
    }

    /// Number of textures evicted since the pool was created.
    pub fn evicted(&self) -> u64 {
        self.evicted
    }

    /// Changes the budget, evicting the textures not used since the last flush at once
    /// if needed (the others at the next flush).
    pub fn set_budget(&mut self, budget: u64) {
        self.budget = budget;
        self.evict();
    }

    /// Releases the least recently used discardable textures not used since the last
    /// flush, while the pool is over budget.
    fn evict(&mut self) {
        while self.memory > self.budget {
            let victim = self
                .entries
                .iter()
                .filter(|(_, entry)| {
                    entry.residency == Residency::Discardable && entry.epoch != self.epoch
                })
                .min_by_key(|(_, entry)| entry.last_use)
                .map(|(id, _)| *id);
            let Some(victim) = victim else {
                // Only pinned or busy textures are left: the budget cannot be met now.
                break;
            };
            tracing::debug!(?victim, "texture evicted");
            self.release(victim);
            self.evicted += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(size: u32, format: wgpu::TextureFormat, mips: u32) -> wgpu::TextureDescriptor<'static> {
        wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        }
    }

    #[test]
    fn sizes_count_texels_mips_and_formats() {
        use wgpu::TextureFormat::{R16Float, Rgba16Float};
        assert_eq!(texture_bytes(&desc(4096, Rgba16Float, 1)), 4096 * 4096 * 8);
        assert_eq!(texture_bytes(&desc(4096, R16Float, 1)), 4096 * 4096 * 2);
        // 4 + 1 texels for a 2² chain.
        assert_eq!(texture_bytes(&desc(2, R16Float, 2)), 5 * 2);
        assert_eq!(full_mip_count(1), 1);
        assert_eq!(full_mip_count(2048), 12);
        assert_eq!(full_mip_count(4096), 13);
        // The full chain adds a third, minus the geometric series' remainder.
        let chain = texture_bytes(&desc(2048, Rgba16Float, full_mip_count(2048)));
        let base = texture_bytes(&desc(2048, Rgba16Float, 1));
        assert!(chain < base * 4 / 3 && base * 4 / 3 - chain < 64, "{chain}");
    }
}
