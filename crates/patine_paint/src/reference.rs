//! CPU mirror of the paint shaders (`shaders/paint/`), texel by texel. The GPU passes are
//! tested against these functions, written with the same formulas in the same order.

use glam::{Vec2, Vec3, Vec4};
use patine_procedural::reference::surface::Surface;

use crate::alphas::AlphaImage;
use crate::projection::{GpuDab, StrokeProjection};
use crate::{DEPTH_BIAS, DILATION, GRAZING};

/// The depth seen from a stroke's camera: for every viewport pixel, the distance of the
/// nearest surface ([`StrokeProjection::depth`]), `f32::MAX` over the background.
#[derive(Clone, Debug, PartialEq)]
pub struct DepthImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Depths, row by row from the top.
    pub values: Vec<f32>,
}

impl DepthImage {
    fn at(&self, x: i32, y: i32) -> f32 {
        let x = x.clamp(0, self.width as i32 - 1) as u32;
        let y = y.clamp(0, self.height as i32 - 1) as u32;
        self.values[(y * self.width + x) as usize]
    }

    /// The farthest depth among the four pixels around `pixel`.
    pub fn around(&self, pixel: Vec2) -> f32 {
        let p = pixel - 0.5;
        let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
        self.at(x, y)
            .max(self.at(x + 1, y))
            .max(self.at(x, y + 1))
            .max(self.at(x + 1, y + 1))
    }
}

/// GLSL/WGSL `smoothstep`.
pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Value of an alpha at `uv` (0 to 1 over the image), bilinear between pixel centers,
/// 0 outside.
pub fn sample_alpha(alpha: &AlphaImage, uv: Vec2) -> f32 {
    let (w, h) = (alpha.width as i32, alpha.height as i32);
    let p = Vec2::new(uv.x * w as f32 - 0.5, uv.y * h as f32 - 0.5);
    let base = p.floor();
    let f = p - base;
    let texel = |dx: i32, dy: i32| {
        let x = base.x as i32 + dx;
        let y = base.y as i32 + dy;
        if x < 0 || y < 0 || x >= w || y >= h {
            0.0
        } else {
            f32::from(alpha.pixels[(y * w + x) as usize]) / 255.0
        }
    };
    let top = texel(0, 0) * (1.0 - f.x) + texel(1, 0) * f.x;
    let bottom = texel(0, 1) * (1.0 - f.x) + texel(1, 1) * f.x;
    top * (1.0 - f.y) + bottom * f.y
}

/// How much of `dab` covers the viewport pixel position `pixel`, 0 to 1 (before flow):
/// its alpha there, or a disk softened by its hardness.
pub fn dab_coverage(dab: &GpuDab, pixel: Vec2, alpha: Option<&AlphaImage>) -> f32 {
    let d = pixel - Vec2::from(dab.center);
    let [c, s] = dab.rotation;
    // Into the dab's frame (inverse rotation), in units of its radius.
    let local = Vec2::new(d.x * c + d.y * s, -d.x * s + d.y * c) / dab.radius;
    match alpha {
        Some(alpha) => {
            if local.x.abs() > 1.0 || local.y.abs() > 1.0 {
                0.0
            } else {
                sample_alpha(alpha, local * 0.5 + 0.5)
            }
        }
        None => {
            let distance = local.length() * dab.radius;
            let soft = ((1.0 - dab.hardness) * dab.radius).max(1.0);
            smoothstep(0.0, 1.0, (dab.radius - distance) / soft)
        }
    }
}

/// What a texel shows to a stroke's camera: where it lands on the viewport, and how much
/// paint it takes there (angle to the camera), or `None` when it takes none (no
/// surface, too far from its island, behind the camera, seen edge-on, or hidden).
pub fn texel_view(
    projection: &StrokeProjection,
    depth: &DepthImage,
    extent: f32,
    surface: &Surface,
    texel: [u32; 2],
    size: u32,
) -> Option<(Vec2, f32)> {
    if surface.normal == Vec3::ZERO {
        return None;
    }
    let p = (Vec2::new(texel[0] as f32, texel[1] as f32) + 0.5) / size as f32;
    if (p - surface.uv).length() * size as f32 > DILATION {
        return None;
    }
    let clip: Vec4 = projection.clip(surface.position);
    let pixel = projection.pixel(clip)?;
    let facing = surface.normal.dot(projection.toward_camera(clip));
    let angle = smoothstep(GRAZING[0], GRAZING[1], facing);
    if angle <= 0.0 {
        return None;
    }
    if projection.depth(clip) > depth.around(pixel) + DEPTH_BIAS * extent {
        return None;
    }
    Some((pixel, angle))
}

/// What a texel of a stroke painted in UV space shows to its camera: where its surface
/// point's UV lands on the 2D view (no occlusion, no angle), or `None` when it takes no
/// paint (no surface, too far from its island, outside the camera).
pub fn texel_view_uv(
    projection: &StrokeProjection,
    surface: &Surface,
    texel: [u32; 2],
    size: u32,
) -> Option<(Vec2, f32)> {
    if surface.normal == Vec3::ZERO {
        return None;
    }
    let p = (Vec2::new(texel[0] as f32, texel[1] as f32) + 0.5) / size as f32;
    if (p - surface.uv).length() * size as f32 > DILATION {
        return None;
    }
    let pixel = projection.pixel(projection.clip(surface.uv.extend(0.0)))?;
    Some((pixel, 1.0))
}

/// Paint accumulated by the stroke buffer after one more dab of weight `a` whose
/// opacity is `opacity`: the paint grows toward 1 but never past the dab's opacity, and
/// never decreases. With an opacity of 1, `s + (1 − s) × a` exactly.
pub fn accumulate(s: f32, a: f32, opacity: f32) -> f32 {
    s.max(opacity.min(s + (1.0 - s) * a))
}

/// The stroke buffer value of a texel after `dabs`, from `s`, seen through `views`
/// (one per symmetry copy): dabs in order, and for each dab the copies in order.
pub fn stroke_texel(
    s: f32,
    views: &[Option<(Vec2, f32)>],
    dabs: &[GpuDab],
    alpha: Option<&AlphaImage>,
) -> f32 {
    dabs.iter().fold(s, |s, dab| {
        views.iter().flatten().fold(s, |s, (pixel, angle)| {
            accumulate(
                s,
                dab.flow * dab_coverage(dab, *pixel, alpha) * angle,
                dab.opacity,
            )
        })
    })
}

/// Layout of a paint content texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentLayout {
    /// `Rgba16Float`: premultiplied rgb (or xyz), coverage in `a`.
    Vector,
    /// `Rg16Float`: premultiplied value, coverage in `g`.
    Scalar,
}

impl ContentLayout {
    /// Index of the coverage component.
    pub fn coverage(self) -> usize {
        match self {
            Self::Vector => 3,
            Self::Scalar => 1,
        }
    }
}

/// A content texel after the stroke deposits `value` with weight `a` (`a` is the stroke
/// buffer times the stroke's opacity) over `before`.
pub fn deposit(layout: ContentLayout, before: [f32; 4], value: [f32; 4], a: f32) -> [f32; 4] {
    let k = 1.0 - a;
    match layout {
        ContentLayout::Vector => [
            before[0] * k + value[0] * a,
            before[1] * k + value[1] * a,
            before[2] * k + value[2] * a,
            before[3] * k + a,
        ],
        ContentLayout::Scalar => [before[0] * k + value[0] * a, before[1] * k + a, 0.0, 0.0],
    }
}

/// A content texel after the eraser removes coverage `a` from `before`.
pub fn erase(layout: ContentLayout, before: [f32; 4], a: f32) -> [f32; 4] {
    let k = 1.0 - a;
    match layout {
        ContentLayout::Vector => before.map(|v| v * k),
        ContentLayout::Scalar => [before[0] * k, before[1] * k, 0.0, 0.0],
    }
}

/// The value and coverage of a content texel: premultiplied values divided by the
/// coverage (0 where there is none).
pub fn unpremultiply(layout: ContentLayout, texel: [f32; 4]) -> ([f32; 4], f32) {
    let c = texel[layout.coverage()];
    let inverse = if c > COVERAGE_EPSILON { 1.0 / c } else { 0.0 };
    match layout {
        ContentLayout::Vector => (
            [
                texel[0] * inverse,
                texel[1] * inverse,
                texel[2] * inverse,
                1.0,
            ],
            c,
        ),
        ContentLayout::Scalar => ([texel[0] * inverse, 0.0, 0.0, 0.0], c),
    }
}

/// Coverage under which a content texel counts as empty.
pub const COVERAGE_EPSILON: f32 = 1e-4;

#[cfg(test)]
mod tests {
    use super::*;

    fn dab(hardness: f32) -> GpuDab {
        GpuDab {
            center: [50.0, 50.0],
            radius: 10.0,
            flow: 1.0,
            rotation: [1.0, 0.0],
            hardness,
            opacity: 1.0,
        }
    }

    #[test]
    fn round_dabs_fade_with_their_hardness() {
        let soft = dab(0.0);
        assert!((dab_coverage(&soft, Vec2::new(50.0, 50.0), None) - 1.0).abs() < 1e-6);
        assert_eq!(dab_coverage(&soft, Vec2::new(60.0, 50.0), None), 0.0);
        let half = dab_coverage(&soft, Vec2::new(55.0, 50.0), None);
        assert!((half - 0.5).abs() < 1e-6, "{half}");
        // A hard dab is full up to one pixel from its edge.
        let hard = dab(1.0);
        assert_eq!(dab_coverage(&hard, Vec2::new(58.9, 50.0), None), 1.0);
        assert_eq!(dab_coverage(&hard, Vec2::new(60.0, 50.0), None), 0.0);
    }

    #[test]
    fn alphas_turn_with_the_dab() {
        // An alpha painted only on its right half.
        let alpha = AlphaImage {
            width: 2,
            height: 2,
            pixels: vec![0, 255, 0, 255],
        };
        let mut d = dab(0.5);
        // At the center of the painted pixel (local x = 0.5).
        let right = Vec2::new(55.0, 50.0);
        assert!(dab_coverage(&d, right, Some(&alpha)) > 0.9);
        // Turned half a turn: the painted half is on the left.
        d.rotation = [-1.0, 0.0];
        assert!(dab_coverage(&d, right, Some(&alpha)) < 0.1);
        assert_eq!(dab_coverage(&d, Vec2::new(70.0, 50.0), Some(&alpha)), 0.0);
    }

    #[test]
    fn deposits_compose_like_over_and_erase_scales() {
        let empty = [0.0; 4];
        let red = [1.0, 0.0, 0.0, 1.0];
        let once = deposit(ContentLayout::Vector, empty, red, 0.5);
        assert_eq!(once, [0.5, 0.0, 0.0, 0.5]);
        assert_eq!(
            unpremultiply(ContentLayout::Vector, once),
            ([1.0, 0.0, 0.0, 1.0], 0.5)
        );
        let erased = erase(ContentLayout::Vector, once, 0.5);
        assert_eq!(erased, [0.25, 0.0, 0.0, 0.25]);
        let scalar = deposit(ContentLayout::Scalar, [0.2, 0.4, 0.0, 0.0], [0.9; 4], 0.5);
        assert!((scalar[0] - 0.55).abs() < 1e-6 && (scalar[1] - 0.7).abs() < 1e-6);
        assert_eq!(
            unpremultiply(ContentLayout::Scalar, [0.0; 4]),
            ([0.0; 4], 0.0)
        );
        // The stroke buffer saturates without exceeding 1.
        let s = (0..100).fold(0.0, |s, _| accumulate(s, 0.3, 1.0));
        assert!(s <= 1.0 && s > 0.999);
        // A dab's opacity caps what it adds, and never removes paint.
        let capped = (0..100).fold(0.0, |s, _| accumulate(s, 0.3, 0.4));
        assert_eq!(capped, 0.4);
        assert_eq!(accumulate(0.7, 0.9, 0.2), 0.7);
        assert_eq!(accumulate(0.25, 0.5, 1.0), 0.25 + 0.75 * 0.5);
    }
}
