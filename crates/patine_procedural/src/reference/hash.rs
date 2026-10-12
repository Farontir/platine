//! Mirror of `shaders/procedural/hash.wgsl`.

use super::{IVec2, Vec2};

/// PCG hash (one round).
pub fn pcg(v: u32) -> u32 {
    let state = v.wrapping_mul(747796405).wrapping_add(2891336453);
    let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277803737);
    (word >> 22) ^ word
}

/// Hash of a lattice cell and a seed.
pub fn hash_cell(cell: IVec2, seed: u32) -> u32 {
    pcg(pcg((cell.x as u32).wrapping_add(seed)).wrapping_add(cell.y as u32))
}

/// A number in [0, 1) from the 24 high bits of a hash.
pub fn unit_float(h: u32) -> f32 {
    (h >> 8) as f32 / 16777216.0
}

/// Two numbers in [0, 1) from a hash.
pub fn unit_float2(h: u32) -> Vec2 {
    Vec2::new(unit_float(h), unit_float(pcg(h)))
}

/// `cell` wrapped into [0, period) on each axis.
pub fn wrap_cell(cell: IVec2, period: IVec2) -> IVec2 {
    IVec2::new(
        (cell.x % period.x + period.x) % period.x,
        (cell.y % period.y + period.y) % period.y,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_spread_and_wrap() {
        assert_ne!(pcg(0), pcg(1));
        let values: Vec<f32> = (0..1000).map(|i| unit_float(pcg(i))).collect();
        assert!(values.iter().all(|v| (0.0..1.0).contains(v)));
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        assert!((mean - 0.5).abs() < 0.05, "{mean}");
        assert_eq!(
            wrap_cell(IVec2::new(-1, 9), IVec2::new(4, 4)),
            IVec2::new(3, 1)
        );
        assert_ne!(
            hash_cell(IVec2::new(1, 2), 0),
            hash_cell(IVec2::new(1, 2), 1)
        );
        assert_ne!(
            hash_cell(IVec2::new(1, 2), 0),
            hash_cell(IVec2::new(2, 1), 0)
        );
    }
}
