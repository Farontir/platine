//! Materials of the viewport: the channel textures of a texture set, bound for the mesh
//! shader (bind group 2, `shaders/render/material.wgsl`).
//!
//! The shader multiplies every texture by a factor, as glTF does. A document material
//! uses its channel textures with factors of 1; the constant material of
//! [`crate::settings::ViewSettings`] uses white textures and puts its values in the
//! factors. A channel a texture set lacks is replaced by a 1×1 texture holding its
//! neutral value.

use bytemuck::{Pod, Zeroable};
use patine_core::channel::Channel;
use patine_gpu::{GpuContext, wgpu};

use crate::settings::ConstantMaterial;

/// Texture slots read by the lit shader, in binding order (bindings 1 to 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    /// Linear base color (`Rgba16Float`).
    BaseColor,
    /// Metalness (red channel).
    Metallic,
    /// Perceptual roughness (red channel).
    Roughness,
    /// Tangent-space normal, not encoded (`Rgba16Float`).
    Normal,
    /// Ambient occlusion (red channel), applied to indirect light.
    AmbientOcclusion,
    /// Emitted radiance (`Rgba16Float`).
    Emissive,
}

impl Slot {
    /// Every slot, in binding order.
    pub const ALL: [Slot; 6] = [
        Slot::BaseColor,
        Slot::Metallic,
        Slot::Roughness,
        Slot::Normal,
        Slot::AmbientOcclusion,
        Slot::Emissive,
    ];

    /// The slot showing `channel` in the lit shader, if any.
    pub fn of(channel: Channel) -> Option<Slot> {
        match channel {
            Channel::BaseColor => Some(Slot::BaseColor),
            Channel::Metallic => Some(Slot::Metallic),
            Channel::Roughness => Some(Slot::Roughness),
            Channel::Normal => Some(Slot::Normal),
            Channel::AmbientOcclusion => Some(Slot::AmbientOcclusion),
            Channel::Emissive => Some(Slot::Emissive),
            Channel::Height | Channel::Opacity | Channel::User(_) => None,
        }
    }

    fn index(self) -> usize {
        Slot::ALL
            .iter()
            .position(|slot| *slot == self)
            .expect("ALL lists every slot")
    }
}

/// How the inspected channel is shown in the channel display mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum InspectKind {
    /// Linear color, sRGB-encoded for display.
    #[default]
    Color,
    /// Data in the red channel, shown as is (0.5 = middle gray).
    Scalar,
    /// Tangent-space normal, shown as `n × 0.5 + 0.5`.
    Normal,
}

impl InspectKind {
    /// How `channel` is shown.
    pub fn of(channel: Channel) -> Self {
        match channel {
            Channel::BaseColor | Channel::Emissive => InspectKind::Color,
            Channel::Normal => InspectKind::Normal,
            _ => InspectKind::Scalar,
        }
    }

    fn index(self) -> f32 {
        match self {
            InspectKind::Color => 0.0,
            InspectKind::Scalar => 1.0,
            InspectKind::Normal => 2.0,
        }
    }
}

/// Uniforms of a material. Must match `Material` in `shaders/render/material.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct MaterialUniforms {
    /// rgb: base color factor.
    base_color: [f32; 4],
    /// x: metallic, y: roughness, z: occlusion strength, w: emissive factor.
    factors: [f32; 4],
    /// x: inspected channel kind, y: 1 when the inspected channel exists.
    inspect: [f32; 4],
}

/// Channel textures of a texture set, by slot, plus the channel shown in the channel
/// display mode. Missing slots use neutral values.
#[derive(Clone, Copy, Debug, Default)]
pub struct MaterialTextures<'a> {
    /// Texture of each slot, in [`Slot::ALL`] order.
    pub slots: [Option<&'a wgpu::TextureView>; 6],
    /// Channel shown in the channel display mode, and how.
    pub inspect: Option<(&'a wgpu::TextureView, InspectKind)>,
}

impl<'a> MaterialTextures<'a> {
    /// Sets the texture of `slot`.
    pub fn with(mut self, slot: Slot, view: &'a wgpu::TextureView) -> Self {
        self.slots[slot.index()] = Some(view);
        self
    }
}

/// A material bound for the mesh shader.
#[derive(Debug)]
pub struct GpuMaterial {
    pub(crate) bind_group: wgpu::BindGroup,
    buffer: wgpu::Buffer,
}

/// 1×1 texture holding `value`.
fn pixel(
    ctx: &GpuContext,
    label: &str,
    format: wgpu::TextureFormat,
    value: [f32; 4],
) -> wgpu::TextureView {
    let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let components = if format == wgpu::TextureFormat::R16Float {
        1
    } else {
        4
    };
    let bytes: Vec<u8> = value[..components]
        .iter()
        .flat_map(|v| half::f16::from_f32(*v).to_le_bytes())
        .collect();
    ctx.queue.write_texture(
        texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: None,
            rows_per_image: None,
        },
        texture.size(),
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// The material bind group layout, its sampler and the 1×1 fallback textures.
#[derive(Debug)]
pub(crate) struct MaterialResources {
    pub(crate) layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Neutral value of each slot for document materials.
    neutral: [wgpu::TextureView; 6],
    /// White textures (and a flat normal, no emission) for the constant material.
    white: [wgpu::TextureView; 6],
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

impl MaterialResources {
    pub(crate) fn new(ctx: &GpuContext) -> Self {
        use wgpu::TextureFormat::{R16Float, Rgba16Float};
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(size_of::<MaterialUniforms>() as u64),
            },
            count: None,
        }];
        // Six slots and the inspected channel.
        entries.extend((1..=7).map(texture_entry));
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 8,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        });
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("viewport material"),
                entries: &entries,
            });
        let sampler = ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("viewport material"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let flat = [0.0, 0.0, 1.0, 0.0];
        let neutral = [
            pixel(ctx, "neutral base color", Rgba16Float, [0.5, 0.5, 0.5, 1.0]),
            pixel(ctx, "neutral metallic", R16Float, [0.0; 4]),
            pixel(ctx, "neutral roughness", R16Float, [0.5; 4]),
            pixel(ctx, "flat normal", Rgba16Float, flat),
            pixel(ctx, "neutral occlusion", R16Float, [1.0; 4]),
            pixel(ctx, "neutral emission", Rgba16Float, [0.0, 0.0, 0.0, 1.0]),
        ];
        let white = [
            pixel(ctx, "white base color", Rgba16Float, [1.0; 4]),
            pixel(ctx, "white metallic", R16Float, [1.0; 4]),
            pixel(ctx, "white roughness", R16Float, [1.0; 4]),
            pixel(ctx, "flat normal", Rgba16Float, flat),
            pixel(ctx, "white occlusion", R16Float, [1.0; 4]),
            pixel(ctx, "no emission", Rgba16Float, [0.0, 0.0, 0.0, 1.0]),
        ];
        Self {
            layout,
            sampler,
            neutral,
            white,
        }
    }

    fn bind(
        &self,
        ctx: &GpuContext,
        slots: [&wgpu::TextureView; 6],
        inspect: &wgpu::TextureView,
        uniforms: MaterialUniforms,
    ) -> GpuMaterial {
        let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport material"),
            size: size_of::<MaterialUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ctx.queue
            .write_buffer(&buffer, 0, bytemuck::bytes_of(&uniforms));
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }];
        for (binding, view) in (1..).zip(slots.iter().chain([&inspect])) {
            entries.push(wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        entries.push(wgpu::BindGroupEntry {
            binding: 8,
            resource: wgpu::BindingResource::Sampler(&self.sampler),
        });
        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport material"),
            layout: &self.layout,
            entries: &entries,
        });
        GpuMaterial { bind_group, buffer }
    }

    /// A document material: its textures with factors of 1.
    pub(crate) fn document(
        &self,
        ctx: &GpuContext,
        textures: &MaterialTextures<'_>,
    ) -> GpuMaterial {
        let slots: [&wgpu::TextureView; 6] =
            std::array::from_fn(|i| textures.slots[i].unwrap_or(&self.neutral[i]));
        let (inspect, kind, present) = match textures.inspect {
            Some((view, kind)) => (view, kind, 1.0),
            None => (
                &self.neutral[Slot::Metallic.index()],
                InspectKind::Scalar,
                0.0,
            ),
        };
        let uniforms = MaterialUniforms {
            base_color: [1.0; 4],
            factors: [1.0; 4],
            inspect: [kind.index(), present, 0.0, 0.0],
        };
        self.bind(ctx, slots, inspect, uniforms)
    }

    /// The constant material: white textures, values in the factors.
    pub(crate) fn constant(&self, ctx: &GpuContext) -> GpuMaterial {
        let slots: [&wgpu::TextureView; 6] = std::array::from_fn(|i| &self.white[i]);
        let base = &self.white[Slot::BaseColor.index()];
        self.bind(
            ctx,
            slots,
            base,
            constant_uniforms(&ConstantMaterial::default()),
        )
    }
}

fn constant_uniforms(material: &ConstantMaterial) -> MaterialUniforms {
    let [r, g, b] = material.base_color;
    MaterialUniforms {
        base_color: [r, g, b, 1.0],
        factors: [material.metallic, material.roughness, 1.0, 1.0],
        // The constant material shows its base color in the channel mode.
        inspect: [InspectKind::Color.index(), 1.0, 0.0, 0.0],
    }
}

impl GpuMaterial {
    /// Writes the values of the constant material into its factors.
    pub(crate) fn set_constant(&self, ctx: &GpuContext, material: &ConstantMaterial) {
        ctx.queue.write_buffer(
            &self.buffer,
            0,
            bytemuck::bytes_of(&constant_uniforms(material)),
        );
    }
}
