//! Material channels of a texture set.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A material channel. Ordered so that maps keyed by channel serialize in a stable order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Channel {
    /// Albedo, linear RGB.
    BaseColor,
    /// Metalness, 0 to 1.
    Metallic,
    /// Perceptual roughness, 0 to 1.
    Roughness,
    /// Tangent-space normal.
    Normal,
    /// Height, converted to a normal at composition.
    Height,
    /// Ambient occlusion, 1 = unoccluded.
    AmbientOcclusion,
    /// Emitted radiance, linear RGB.
    Emissive,
    /// Opacity, 1 = opaque.
    Opacity,
    /// Free channel for the user's own data.
    User(u8),
}

impl Channel {
    /// Every channel of a texture set except the user channels, in order.
    pub const STANDARD: [Channel; 8] = [
        Channel::BaseColor,
        Channel::Metallic,
        Channel::Roughness,
        Channel::Normal,
        Channel::Height,
        Channel::AmbientOcclusion,
        Channel::Emissive,
        Channel::Opacity,
    ];

    /// Stable textual identifier, e.g. `base-color` or `user-3`, used in history labels
    /// and translation keys.
    pub fn id(self) -> String {
        match self {
            Channel::BaseColor => "base-color".into(),
            Channel::Metallic => "metallic".into(),
            Channel::Roughness => "roughness".into(),
            Channel::Normal => "normal".into(),
            Channel::Height => "height".into(),
            Channel::AmbientOcclusion => "ambient-occlusion".into(),
            Channel::Emissive => "emissive".into(),
            Channel::Opacity => "opacity".into(),
            Channel::User(index) => format!("user-{index}"),
        }
    }

    /// The channel with identifier `id`.
    pub fn from_id(id: &str) -> Option<Self> {
        if let Some(index) = id.strip_prefix("user-") {
            return index.parse().ok().map(Channel::User);
        }
        Channel::STANDARD
            .into_iter()
            .find(|channel| channel.id() == id)
    }

    /// Number of meaningful components: 3 for colors and normals, 1 otherwise.
    pub fn components(self) -> usize {
        match self {
            Channel::BaseColor | Channel::Normal | Channel::Emissive => 3,
            _ => 1,
        }
    }
}

/// GPU storage format of a channel (computations are done in f32).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChannelFormat {
    /// Four 16-bit floats, for colors and normals.
    Rgba16Float,
    /// One 16-bit float, for scalar channels and masks.
    R16Float,
}

/// A channel active in a texture set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelDef {
    /// The channel.
    pub channel: Channel,
    /// Storage format.
    pub format: ChannelFormat,
    /// Value before any layer: RGBA, scalars in the first component. Normals are
    /// tangent-space vectors, not encoded colors.
    pub default: [f32; 4],
}

impl ChannelDef {
    /// The channel with its usual format and default value.
    pub fn standard(channel: Channel) -> Self {
        let (format, default) = match channel {
            Channel::BaseColor => (ChannelFormat::Rgba16Float, [0.5, 0.5, 0.5, 1.0]),
            Channel::Normal => (ChannelFormat::Rgba16Float, [0.0, 0.0, 1.0, 0.0]),
            Channel::Emissive => (ChannelFormat::Rgba16Float, [0.0, 0.0, 0.0, 1.0]),
            Channel::Roughness => (ChannelFormat::R16Float, [0.5, 0.0, 0.0, 0.0]),
            Channel::AmbientOcclusion | Channel::Opacity => {
                (ChannelFormat::R16Float, [1.0, 0.0, 0.0, 0.0])
            }
            Channel::Metallic | Channel::Height | Channel::User(_) => {
                (ChannelFormat::R16Float, [0.0; 4])
            }
        };
        Self {
            channel,
            format,
            default,
        }
    }
}

/// Channels of a new texture set: the usual metal/roughness material with height.
pub fn default_channels() -> Vec<ChannelDef> {
    [
        Channel::BaseColor,
        Channel::Metallic,
        Channel::Roughness,
        Channel::Normal,
        Channel::Height,
    ]
    .into_iter()
    .map(ChannelDef::standard)
    .collect()
}

/// A value per channel, in a stable order.
pub type ChannelMap<T> = BTreeMap<Channel, T>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_channels_have_their_usual_formats() {
        let channels = default_channels();
        assert_eq!(channels.len(), 5);
        for def in &channels {
            let expected = if def.channel.components() == 3 {
                ChannelFormat::Rgba16Float
            } else {
                ChannelFormat::R16Float
            };
            assert_eq!(def.format, expected, "{:?}", def.channel);
        }
        assert_eq!(
            ChannelDef::standard(Channel::Normal).default,
            [0.0, 0.0, 1.0, 0.0]
        );
    }

    #[test]
    fn identifiers_round_trip() {
        for channel in Channel::STANDARD
            .into_iter()
            .chain([Channel::User(0), Channel::User(7)])
        {
            assert_eq!(Channel::from_id(&channel.id()), Some(channel));
        }
        assert_eq!(Channel::from_id("user-x"), None);
        assert_eq!(Channel::from_id("albedo"), None);
    }
}
