//! CPU reference of the compositor: every blend mode, the composition of a stack of fill
//! layers, and the mip reduction. The GPU passes are tested against these functions.
//!
//! Values are linear and unclamped (height may be negative, emission above 1), except
//! where a mode's formula clamps. Normals are tangent-space unit vectors, not encoded.

use patine_core::channel::Channel;
use patine_core::layer::{BlendMode, FillSource, Layer, LayerKind};

/// How a channel's values are blended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChannelKind {
    /// RGB color (base color, emission).
    Color,
    /// One value in the red component (metallic, roughness, height...).
    Scalar,
    /// Tangent-space unit normal in RGB.
    Normal,
}

impl ChannelKind {
    /// The kind of `channel`.
    pub fn of(channel: Channel) -> Self {
        match channel {
            Channel::BaseColor | Channel::Emissive => ChannelKind::Color,
            Channel::Normal => ChannelKind::Normal,
            _ => ChannelKind::Scalar,
        }
    }

    /// Index passed to the shaders.
    pub fn index(self) -> u32 {
        match self {
            ChannelKind::Color => 0,
            ChannelKind::Scalar => 1,
            ChannelKind::Normal => 2,
        }
    }
}

/// Index of a blend mode in the shaders (`shaders/compositor/blend_ops.wgsl`).
pub fn mode_index(mode: BlendMode) -> u32 {
    match mode {
        BlendMode::Normal => 0,
        BlendMode::Replace => 1,
        BlendMode::Passthrough => 2,
        BlendMode::Multiply => 3,
        BlendMode::Divide => 4,
        BlendMode::Screen => 5,
        BlendMode::Overlay => 6,
        BlendMode::SoftLight => 7,
        BlendMode::HardLight => 8,
        BlendMode::LinearDodge => 9,
        BlendMode::LinearBurn => 10,
        BlendMode::Subtract => 11,
        BlendMode::Difference => 12,
        BlendMode::Exclusion => 13,
        BlendMode::Darken => 14,
        BlendMode::Lighten => 15,
        BlendMode::ColorDodge => 16,
        BlendMode::ColorBurn => 17,
        BlendMode::Hue => 18,
        BlendMode::Saturation => 19,
        BlendMode::Color => 20,
        BlendMode::Luminosity => 21,
        BlendMode::Disable => 22,
        BlendMode::NormalMapCombine => 23,
        BlendMode::NormalMapDetail => 24,
    }
}

/// Every blend mode, in shader index order.
pub const ALL_MODES: [BlendMode; 25] = [
    BlendMode::Normal,
    BlendMode::Replace,
    BlendMode::Passthrough,
    BlendMode::Multiply,
    BlendMode::Divide,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::SoftLight,
    BlendMode::HardLight,
    BlendMode::LinearDodge,
    BlendMode::LinearBurn,
    BlendMode::Subtract,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::ColorDodge,
    BlendMode::ColorBurn,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Color,
    BlendMode::Luminosity,
    BlendMode::Disable,
    BlendMode::NormalMapCombine,
    BlendMode::NormalMapDetail,
];

/// Below this, a divisor counts as zero (Divide, Color Dodge, Color Burn).
pub const EPSILON: f32 = 1e-4;

type Rgb = [f32; 3];

fn map(a: Rgb, b: Rgb, f: impl Fn(f32, f32) -> f32) -> Rgb {
    [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2])]
}

fn hard_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        b * 2.0 * s
    } else {
        let s = 2.0 * s - 1.0;
        b + s - b * s
    }
}

fn soft_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        b - (1.0 - 2.0 * s) * b * (1.0 - b)
    } else {
        let d = if b <= 0.25 {
            ((16.0 * b - 12.0) * b + 4.0) * b
        } else {
            b.max(0.0).sqrt()
        };
        b + (2.0 * s - 1.0) * (d - b)
    }
}

fn color_dodge(b: f32, s: f32) -> f32 {
    if b <= 0.0 {
        0.0
    } else if s >= 1.0 - EPSILON {
        1.0
    } else {
        (b / (1.0 - s)).min(1.0)
    }
}

fn color_burn(b: f32, s: f32) -> f32 {
    if b >= 1.0 {
        1.0
    } else if s <= EPSILON {
        0.0
    } else {
        1.0 - ((1.0 - b) / s).min(1.0)
    }
}

fn divide(b: f32, s: f32) -> f32 {
    if s <= EPSILON {
        if b > 0.0 { 1.0 } else { 0.0 }
    } else {
        b / s
    }
}

// Non-separable modes of the W3C Compositing and Blending specification.
fn lum(c: Rgb) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(c: Rgb) -> Rgb {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut out = c;
    if n < 0.0 {
        out = out.map(|v| l + (v - l) * l / (l - n));
    }
    if x > 1.0 {
        out = out.map(|v| l + (v - l) * (1.0 - l) / (x - l));
    }
    out
}

fn set_lum(c: Rgb, l: f32) -> Rgb {
    let d = l - lum(c);
    clip_color(c.map(|v| v + d))
}

fn sat(c: Rgb) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(c: Rgb, s: f32) -> Rgb {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    if max > min {
        c.map(|v| (v - min) * s / (max - min))
    } else {
        [0.0; 3]
    }
}

/// `blend(dst, src)` of a separable or non-separable mode on RGB values.
fn blend_rgb(mode: BlendMode, b: Rgb, s: Rgb) -> Rgb {
    match mode {
        BlendMode::Normal | BlendMode::Replace | BlendMode::Passthrough => s,
        BlendMode::Multiply => map(b, s, |b, s| b * s),
        BlendMode::Divide => map(b, s, divide),
        BlendMode::Screen => map(b, s, |b, s| b + s - b * s),
        BlendMode::Overlay => map(b, s, |b, s| hard_light(s, b)),
        BlendMode::SoftLight => map(b, s, soft_light),
        BlendMode::HardLight => map(b, s, hard_light),
        BlendMode::LinearDodge => map(b, s, |b, s| b + s),
        BlendMode::LinearBurn => map(b, s, |b, s| b + s - 1.0),
        BlendMode::Subtract => map(b, s, |b, s| b - s),
        BlendMode::Difference => map(b, s, |b, s| (b - s).abs()),
        BlendMode::Exclusion => map(b, s, |b, s| b + s - 2.0 * b * s),
        BlendMode::Darken => map(b, s, f32::min),
        BlendMode::Lighten => map(b, s, f32::max),
        BlendMode::ColorDodge => map(b, s, color_dodge),
        BlendMode::ColorBurn => map(b, s, color_burn),
        BlendMode::Hue => set_lum(set_sat(s, sat(b)), lum(b)),
        BlendMode::Saturation => set_lum(set_sat(b, sat(s)), lum(b)),
        BlendMode::Color => set_lum(s, lum(b)),
        BlendMode::Luminosity => set_lum(b, lum(s)),
        // Normal-only modes act as Normal on colors; Disable is handled by `apply`.
        BlendMode::Disable | BlendMode::NormalMapCombine | BlendMode::NormalMapDetail => s,
    }
}

fn normalize(v: Rgb) -> Rgb {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length > 0.0 {
        v.map(|c| c / length)
    } else {
        [0.0, 0.0, 1.0]
    }
}

/// Reoriented normal mapping: `detail` applied on top of `base` (Barré-Brisebois, Hill).
pub fn reoriented_normal(base: Rgb, detail: Rgb) -> Rgb {
    let t = [base[0], base[1], base[2] + 1.0];
    let u = [-detail[0], -detail[1], detail[2]];
    let dot = t[0] * u[0] + t[1] * u[1] + t[2] * u[2];
    normalize([
        t[0] * dot / t[2] - u[0],
        t[1] * dot / t[2] - u[1],
        t[2] * dot / t[2] - u[2],
    ])
}

/// UDN detail blending: tilts add up, the base keeps its z.
pub fn udn_normal(base: Rgb, detail: Rgb) -> Rgb {
    normalize([base[0] + detail[0], base[1] + detail[1], base[2]])
}

/// `dst` after a layer of content `src` with `mode` and `weight` (mask × opacity):
/// `mix(dst, blend(dst, src), weight)`, renormalized for normals. The alpha component of
/// colors follows the Normal mode; scalars use the red component only.
pub fn apply(
    mode: BlendMode,
    kind: ChannelKind,
    dst: [f32; 4],
    src: [f32; 4],
    weight: f32,
) -> [f32; 4] {
    if mode == BlendMode::Disable {
        return match kind {
            ChannelKind::Scalar => [dst[0], 0.0, 0.0, 0.0],
            _ => dst,
        };
    }
    let w = weight.clamp(0.0, 1.0);
    let mix = |a: f32, b: f32| a + (b - a) * w;
    let b = [dst[0], dst[1], dst[2]];
    let s = [src[0], src[1], src[2]];
    match kind {
        ChannelKind::Normal => {
            let blended = match mode {
                BlendMode::NormalMapCombine => reoriented_normal(b, s),
                BlendMode::NormalMapDetail => udn_normal(b, s),
                _ => s,
            };
            let n = normalize([
                mix(b[0], blended[0]),
                mix(b[1], blended[1]),
                mix(b[2], blended[2]),
            ]);
            [n[0], n[1], n[2], dst[3]]
        }
        ChannelKind::Color => {
            let c = blend_rgb(mode, b, s);
            [
                mix(b[0], c[0]),
                mix(b[1], c[1]),
                mix(b[2], c[2]),
                mix(dst[3], src[3]),
            ]
        }
        ChannelKind::Scalar => {
            let c = blend_rgb(mode, [dst[0]; 3], [src[0]; 3]);
            [mix(dst[0], c[0]), 0.0, 0.0, 0.0]
        }
    }
}

/// Value of `channel` after the fill layers of `layers` (bottom to top), starting from
/// `start`. A folder composes its children onto the value below, then mixes the result
/// in with its own opacity: without per-pixel coverage (M8), Normal and Passthrough are
/// equivalent for folders, Disable skips the folder, and other modes act as Normal.
/// Layers that do not affect `channel`, hidden layers and non-uniform content are skipped.
pub fn composite(layers: &[Layer], channel: Channel, start: [f32; 4]) -> [f32; 4] {
    let kind = ChannelKind::of(channel);
    let mut dst = start;
    for layer in layers.iter().filter(|layer| layer.visible) {
        let Some(blend) = layer.channels.get(&channel) else {
            continue;
        };
        match &layer.kind {
            LayerKind::Fill {
                source: FillSource::Uniform(values),
                ..
            } => {
                if let Some(src) = values.get(&channel) {
                    dst = apply(blend.mode, kind, dst, *src, blend.opacity);
                }
            }
            LayerKind::Folder { children } if blend.mode != BlendMode::Disable => {
                let inner = composite(children, channel, dst);
                dst = apply(BlendMode::Normal, kind, dst, inner, blend.opacity);
            }
            // Paint, procedural and image content, instances: later milestones.
            _ => {}
        }
    }
    dst
}

/// One mip level from the previous one (`width` × `height`, even sizes): the average of
/// each 2 × 2 block, renormalized for normals.
pub fn downsample(
    texels: &[[f32; 4]],
    width: usize,
    height: usize,
    kind: ChannelKind,
) -> Vec<[f32; 4]> {
    let (w, h) = ((width / 2).max(1), (height / 2).max(1));
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0.0f32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let sx = (2 * x + dx).min(width - 1);
                let sy = (2 * y + dy).min(height - 1);
                let t = texels[sy * width + sx];
                for i in 0..4 {
                    sum[i] += t[i];
                }
            }
            let mut average = sum.map(|v| v * 0.25);
            if kind == ChannelKind::Normal {
                let n = normalize([average[0], average[1], average[2]]);
                average = [n[0], n[1], n[2], average[3]];
            }
            out.push(average);
        }
    }
    out
}

/// `value` rounded to the nearest f16, as stored in a channel texture.
pub fn round_f16(value: f32) -> f32 {
    half::f16::from_f32(value).to_f32()
}

#[cfg(test)]
mod tests {
    use super::*;
    use patine_core::ids::LayerId;
    use patine_core::layer::{ChannelBlend, Projection};

    const KINDS: [ChannelKind; 3] = [ChannelKind::Color, ChannelKind::Scalar, ChannelKind::Normal];

    fn close(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn known_values() {
        let b = [0.25, 0.5, 0.75];
        let s = [0.5, 0.5, 0.5];
        let check = |mode, expected: Rgb| {
            let got = blend_rgb(mode, b, s);
            assert!(
                close(
                    [got[0], got[1], got[2], 0.0],
                    [expected[0], expected[1], expected[2], 0.0]
                ),
                "{mode:?}: {got:?}"
            );
        };
        check(BlendMode::Multiply, [0.125, 0.25, 0.375]);
        check(BlendMode::Screen, [0.625, 0.75, 0.875]);
        check(BlendMode::LinearDodge, [0.75, 1.0, 1.25]);
        check(BlendMode::LinearBurn, [-0.25, 0.0, 0.25]);
        check(BlendMode::Subtract, [-0.25, 0.0, 0.25]);
        check(BlendMode::Difference, [0.25, 0.0, 0.25]);
        check(BlendMode::Darken, [0.25, 0.5, 0.5]);
        check(BlendMode::Lighten, [0.5, 0.5, 0.75]);
        check(BlendMode::Divide, [0.5, 1.0, 1.5]);
        check(BlendMode::Exclusion, [0.5, 0.5, 0.5]);
        // Hard light with a mid-gray source is the identity, overlay too for this base.
        check(BlendMode::HardLight, b);
        check(BlendMode::SoftLight, b);
        check(BlendMode::Overlay, [0.25, 0.5, 0.75]);
        check(BlendMode::ColorDodge, [0.5, 1.0, 1.0]);
        check(BlendMode::ColorBurn, [0.0, 0.0, 0.5]);
        // A gray source has no hue or saturation: Color and Hue give a gray of b's luminosity.
        let l = lum(b);
        check(BlendMode::Color, [l, l, l]);
        check(BlendMode::Luminosity, set_lum(b, 0.5));
    }

    #[test]
    fn non_separable_modes_keep_what_they_should() {
        let b = [0.8, 0.2, 0.1];
        let s = [0.1, 0.3, 0.9];
        let luminosity = blend_rgb(BlendMode::Luminosity, b, s);
        assert!((lum(luminosity) - lum(s)).abs() < 1e-4);
        let color = blend_rgb(BlendMode::Color, b, s);
        assert!((lum(color) - lum(b)).abs() < 1e-4);
        let saturation = blend_rgb(BlendMode::Saturation, b, s);
        assert!((sat(saturation) - sat(s)).abs() < 1e-3);
        for mode in [
            BlendMode::Hue,
            BlendMode::Saturation,
            BlendMode::Color,
            BlendMode::Luminosity,
        ] {
            let c = blend_rgb(mode, b, s);
            assert!(
                c.iter().all(|v| (-1e-5..=1.0 + 1e-5).contains(v)),
                "{mode:?} {c:?}"
            );
        }
    }

    #[test]
    fn opacity_interpolates_between_destination_and_blend() {
        let dst = [0.2, 0.4, 0.6, 1.0];
        let src = [0.9, 0.1, 0.5, 1.0];
        for kind in KINDS {
            for mode in ALL_MODES {
                let (dst, src) = if kind == ChannelKind::Normal {
                    let n = normalize([dst[0], dst[1], dst[2] + 1.0]);
                    let m = normalize([src[0] - 0.5, src[1], src[2] + 0.5]);
                    ([n[0], n[1], n[2], 0.0], [m[0], m[1], m[2], 0.0])
                } else {
                    (dst, src)
                };
                let none = apply(mode, kind, dst, src, 0.0);
                let expected_none = if kind == ChannelKind::Scalar {
                    [dst[0], 0.0, 0.0, 0.0]
                } else {
                    dst
                };
                assert!(
                    close(none, expected_none),
                    "{mode:?} {kind:?} at 0: {none:?}"
                );
                if kind == ChannelKind::Normal {
                    let full = apply(mode, kind, dst, src, 1.0);
                    let length = (full[0] * full[0] + full[1] * full[1] + full[2] * full[2]).sqrt();
                    assert!((length - 1.0).abs() < 1e-5, "{mode:?}: {full:?}");
                }
            }
            if kind != ChannelKind::Normal {
                let full = apply(BlendMode::Normal, kind, dst, src, 1.0);
                assert!((full[0] - src[0]).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn normal_blending() {
        let flat = [0.0, 0.0, 1.0];
        let tilted = normalize([0.3, -0.2, 0.9]);
        // Combining with a flat base or a flat detail changes nothing.
        let a = reoriented_normal(flat, tilted);
        let b = reoriented_normal(tilted, flat);
        for i in 0..3 {
            assert!((a[i] - tilted[i]).abs() < 1e-5 && (b[i] - tilted[i]).abs() < 1e-5);
        }
        // Two equal tilts compose into a stronger tilt in the same direction.
        let twice = reoriented_normal(tilted, tilted);
        assert!(twice[0] > tilted[0] && twice[2] < tilted[2]);
        let detail = udn_normal(tilted, tilted);
        assert!(detail[0] > tilted[0]);
        // Disable leaves the value alone.
        let dst = [0.1, 0.2, 0.97, 0.0];
        assert_eq!(
            apply(BlendMode::Disable, ChannelKind::Normal, dst, [0.0; 4], 1.0),
            dst
        );
    }

    fn fill(id: u64, value: [f32; 4], mode: BlendMode, opacity: f32) -> Layer {
        let mut layer = Layer::new(
            LayerId(id),
            "fill",
            LayerKind::Fill {
                source: FillSource::Uniform([(Channel::BaseColor, value)].into()),
                projection: Projection::default(),
            },
            &[Channel::BaseColor],
        );
        layer
            .channels
            .insert(Channel::BaseColor, ChannelBlend { mode, opacity });
        layer
    }

    #[test]
    fn stacks_with_folders_hidden_layers_and_absent_channels() {
        let start = [0.5, 0.5, 0.5, 1.0];
        let red = fill(1, [1.0, 0.0, 0.0, 1.0], BlendMode::Normal, 1.0);
        let half_blue = fill(2, [0.0, 0.0, 1.0, 1.0], BlendMode::Normal, 0.5);
        let result = composite(&[red.clone(), half_blue.clone()], Channel::BaseColor, start);
        assert!(close(result, [0.5, 0.0, 0.5, 1.0]), "{result:?}");
        // Roughness is affected by neither.
        assert_eq!(
            composite(std::slice::from_ref(&red), Channel::Roughness, start),
            start
        );

        let mut hidden = red.clone();
        hidden.visible = false;
        assert_eq!(composite(&[hidden], Channel::BaseColor, start), start);

        // A folder at 50 % fades its whole content; at 100 % it equals its children.
        let mut folder = Layer::new(
            LayerId(3),
            "folder",
            LayerKind::Folder {
                children: vec![red.clone(), half_blue.clone()],
            },
            &[Channel::BaseColor],
        );
        let direct = composite(&[red.clone(), half_blue.clone()], Channel::BaseColor, start);
        assert!(close(
            composite(&[folder.clone()], Channel::BaseColor, start),
            direct
        ));
        folder
            .channels
            .get_mut(&Channel::BaseColor)
            .unwrap()
            .opacity = 0.5;
        let faded = composite(&[folder.clone()], Channel::BaseColor, start);
        let expected = [0.5, 0.25, 0.5, 1.0];
        assert!(close(faded, expected), "{faded:?}");
        folder.channels.get_mut(&Channel::BaseColor).unwrap().mode = BlendMode::Disable;
        assert_eq!(composite(&[folder], Channel::BaseColor, start), start);
    }

    #[test]
    fn downsampling_averages_blocks() {
        let texels = vec![[0.0; 4], [1.0; 4], [0.5; 4], [0.5; 4]];
        let out = downsample(&texels, 2, 2, ChannelKind::Color);
        assert_eq!(out, vec![[0.5; 4]]);
        let normals = vec![
            [0.6, 0.0, 0.8, 0.0],
            [-0.6, 0.0, 0.8, 0.0],
            [0.0, 0.6, 0.8, 0.0],
            [0.0, -0.6, 0.8, 0.0],
        ];
        let out = downsample(&normals, 2, 2, ChannelKind::Normal);
        assert!(close(out[0], [0.0, 0.0, 1.0, 0.0]), "{:?}", out[0]);
    }
}
