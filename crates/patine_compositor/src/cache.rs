//! Raster caches (M10): the painted tiles of a paint layer or a paint mask effect, saved
//! with the project so that opening it does not replay every stroke.
//!
//! A cache holds, for each content channel, the tiles its strokes touched and their
//! texels as stored on the GPU (half floats, exact). It is only used when its key
//! matches: the cache version, the texture set's resolution and material, the channels
//! and the fingerprint of the stroke log. Otherwise the strokes are replayed, as they
//! would be without a cache.

use patine_core::channel::Channel;
use patine_core::project::TextureSet;
use patine_core::resource::ResourceHash;
use patine_core::stroke::StrokeLog;
use patine_gpu::{GpuContext, GpuError, wgpu};
use patine_paint::projection::TILE;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use wgpu::util::DeviceExt as _;

/// Version of the rasterization the caches come from. Increment it whenever painted
/// contents may change for the same strokes (dab shaders, composition, procedural
/// shaders used by brushes): older caches are then replayed instead. The application
/// version is part of the key too.
pub const CACHE_VERSION: u32 = 1;

/// zstd level of caches: fast, and painted tiles are mostly flat.
const ZSTD_LEVEL: i32 = 3;

/// The painted tiles of one content channel.
#[derive(Clone, Debug, PartialEq)]
pub struct CachedChannel {
    /// The channel.
    pub channel: Channel,
    /// `Rgba16Float` content (color and coverage) rather than `Rg16Float` (value and
    /// coverage).
    pub vector: bool,
    /// Tiles, `[x, y]` in tile units.
    pub tiles: Vec<[u32; 2]>,
    /// The texels of each tile in turn, rows of [`TILE`] texels, as on the GPU.
    pub texels: Vec<u8>,
}

impl CachedChannel {
    /// Bytes of one texel.
    pub fn texel_size(&self) -> usize {
        if self.vector { 8 } else { 4 }
    }

    /// Bytes of one tile.
    pub fn tile_bytes(&self) -> usize {
        (TILE * TILE) as usize * self.texel_size()
    }
}

/// The raster cache of one paint layer or paint mask effect.
#[derive(Clone, Debug, PartialEq)]
pub struct RasterCache {
    /// [`cache_key`] of what the cache was made from.
    pub key: [u8; 32],
    /// Side of the contents in texels.
    pub size: u32,
    /// The painted channels.
    pub channels: Vec<CachedChannel>,
}

/// What the stored cache holds before the texels (postcard).
#[derive(Serialize, Deserialize)]
struct Header {
    key: [u8; 32],
    size: u32,
    /// Channel, layout (`vector`) and tiles of each channel; its texels follow, in order.
    channels: Vec<(Channel, bool, Vec<[u32; 2]>)>,
}

/// Why a cache cannot be read.
#[derive(Debug, Error)]
pub enum CacheError {
    /// Not zstd data.
    #[error("raster cache not compressed as expected: {0}")]
    Compression(#[from] std::io::Error),
    /// Not a raster cache.
    #[error("raster cache unreadable: {0}")]
    Format(#[from] postcard::Error),
    /// The texels do not match the header.
    #[error("raster cache truncated")]
    Truncated,
}

/// The key of a cache of `channels` of a paint node of `set` holding `log`.
pub fn cache_key(set: &TextureSet, channels: &[Channel], log: &StrokeLog) -> [u8; 32] {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"patine raster cache");
    bytes.extend_from_slice(&CACHE_VERSION.to_le_bytes());
    bytes.extend_from_slice(env!("CARGO_PKG_VERSION").as_bytes());
    bytes.extend_from_slice(&set.resolution.pixels().to_le_bytes());
    bytes.extend_from_slice(&set.material_index.to_le_bytes());
    for channel in channels {
        bytes.extend_from_slice(channel.id().as_bytes());
        bytes.push(0);
    }
    bytes.extend_from_slice(&log.fingerprint().0);
    ResourceHash::of(&bytes).0
}

impl RasterCache {
    /// Whether the cache was made from `log` in `set`, with consistent data.
    pub fn matches(&self, set: &TextureSet, log: &StrokeLog) -> bool {
        let channels: Vec<Channel> = self.channels.iter().map(|c| c.channel).collect();
        let tiles = set.resolution.pixels().div_ceil(TILE);
        self.size == set.resolution.pixels()
            && self.key == cache_key(set, &channels, log)
            && self.channels.iter().all(|channel| {
                channel.texels.len() == channel.tiles.len() * channel.tile_bytes()
                    && channel.tiles.iter().all(|[x, y]| *x < tiles && *y < tiles)
            })
    }

    /// The cache as stored in a project file, compressed by zstd: the length of the
    /// header (u32, little endian), the header (postcard), then the texels of each
    /// channel as they are (a postcard sequence of bytes would be read byte by byte).
    pub fn encode(&self) -> Vec<u8> {
        let header = Header {
            key: self.key,
            size: self.size,
            channels: self
                .channels
                .iter()
                .map(|c| (c.channel, c.vector, c.tiles.clone()))
                .collect(),
        };
        let header = postcard::to_allocvec(&header).expect("a cache header always serializes");
        let texels: usize = self.channels.iter().map(|c| c.texels.len()).sum();
        let mut raw = Vec::with_capacity(4 + header.len() + texels);
        raw.extend_from_slice(&(header.len() as u32).to_le_bytes());
        raw.extend_from_slice(&header);
        for channel in &self.channels {
            raw.extend_from_slice(&channel.texels);
        }
        zstd::encode_all(raw.as_slice(), ZSTD_LEVEL).expect("compressing into memory does not fail")
    }

    /// A cache read from a project file.
    pub fn decode(bytes: &[u8]) -> Result<Self, CacheError> {
        let raw = zstd::decode_all(bytes)?;
        let (length, rest) = raw.split_at_checked(4).ok_or(CacheError::Truncated)?;
        let length = u32::from_le_bytes(length.try_into().expect("4 bytes")) as usize;
        let (header, mut texels) = rest.split_at_checked(length).ok_or(CacheError::Truncated)?;
        let header: Header = postcard::from_bytes(header)?;
        let mut channels = Vec::with_capacity(header.channels.len());
        for (channel, vector, tiles) in header.channels {
            let mut cached = CachedChannel {
                channel,
                vector,
                tiles,
                texels: Vec::new(),
            };
            let (own, rest) = texels
                .split_at_checked(cached.tiles.len() * cached.tile_bytes())
                .ok_or(CacheError::Truncated)?;
            cached.texels = own.to_vec();
            texels = rest;
            channels.push(cached);
        }
        if !texels.is_empty() {
            return Err(CacheError::Truncated);
        }
        Ok(Self {
            key: header.key,
            size: header.size,
            channels,
        })
    }
}

/// Bytes per texel of a content texture format.
fn texel_size(format: wgpu::TextureFormat) -> u32 {
    match format {
        wgpu::TextureFormat::Rgba16Float => 8,
        _ => 4,
    }
}

/// Reads `tiles` of `texture` back, tile after tile. Blocks until the GPU is done.
pub(crate) fn read_tiles(
    ctx: &GpuContext,
    texture: &wgpu::Texture,
    tiles: &[[u32; 2]],
) -> Result<Vec<u8>, GpuError> {
    let texel = texel_size(texture.format());
    let row = TILE * texel;
    let tile_bytes = u64::from(row * TILE);
    let size = (tile_bytes * tiles.len() as u64).max(4);
    let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("raster cache readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("raster cache readback"),
        });
    for (index, [x, y]) in tiles.iter().enumerate() {
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: x * TILE,
                    y: y * TILE,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: index as u64 * tile_bytes,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(TILE),
                },
            },
            wgpu::Extent3d {
                width: TILE,
                height: TILE,
                depth_or_array_layers: 1,
            },
        );
    }
    ctx.queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    staging.map_async(wgpu::MapMode::Read, .., move |result| {
        // The receiver outlives the poll below; a send failure cannot happen.
        let _ = sender.send(result);
    });
    ctx.device.poll(wgpu::PollType::wait_indefinitely())?;
    receiver
        .recv()
        .map_err(|error| GpuError::BufferMap(error.to_string()))?
        .map_err(|error| GpuError::BufferMap(error.to_string()))?;
    let bytes = staging
        .get_mapped_range(..)
        .map_err(|error| GpuError::BufferMap(error.to_string()))?
        .to_vec();
    staging.unmap();
    Ok(bytes[..(tile_bytes * tiles.len() as u64) as usize].to_vec())
}

/// Encodes copies of the tiles of `cached` into `texture`.
pub(crate) fn upload_tiles(
    ctx: &GpuContext,
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    cached: &CachedChannel,
) {
    if cached.tiles.is_empty() {
        return;
    }
    let row = TILE * texel_size(texture.format());
    let tile_bytes = u64::from(row * TILE);
    let buffer = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("raster cache upload"),
            contents: &cached.texels,
            usage: wgpu::BufferUsages::COPY_SRC,
        });
    for (index, [x, y]) in cached.tiles.iter().enumerate() {
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: index as u64 * tile_bytes,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(TILE),
                },
            },
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: x * TILE,
                    y: y * TILE,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: TILE,
                height: TILE,
                depth_or_array_layers: 1,
            },
        );
    }
}

/// Applies `f` to every item on all the cores, keeping the order.
fn parallel<T: Sync, U: Send>(items: &[T], f: impl Fn(&T) -> U + Sync) -> Vec<U> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let chunk = items.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|part| scope.spawn(|| part.iter().map(&f).collect::<Vec<U>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("a cache thread does not panic"))
            .collect()
    })
}

/// [`RasterCache::encode`] of every cache, on all the cores.
pub fn encode_all(caches: &[(u64, RasterCache)]) -> Vec<(u64, Vec<u8>)> {
    parallel(caches, |(id, cache)| (*id, cache.encode()))
}

/// [`RasterCache::decode`] of every stored cache, on all the cores, in order.
pub fn decode_all(stored: &[(u64, Vec<u8>)]) -> Vec<(u64, Result<RasterCache, CacheError>)> {
    parallel(stored, |(id, bytes)| (*id, RasterCache::decode(bytes)))
}
