//! Symmetry: the copies of a stroke. Each copy is a transform `T` of object space (mirrors
//! across the planes through the origin, rotations around the Y axis); the copy paints as
//! if the stroke had been painted with the camera `view_proj × T`, with that camera's own
//! occlusion. On a symmetric mesh, it paints exactly the mirror image of the stroke.

use glam::{Mat4, Vec3};
use patine_core::stroke::Symmetry;

/// Most copies of a stroke, the identity included.
pub const MAX_COPIES: usize = 16;

/// The transforms of the copies of a stroke with `symmetry`, the identity first: every
/// combination of the mirrors, each turned around Y by the radial steps. Beyond
/// [`MAX_COPIES`], the last ones are dropped.
pub fn copies(symmetry: &Symmetry) -> Vec<Mat4> {
    let mut mirrors = vec![Mat4::IDENTITY];
    for (axis, enabled) in symmetry.mirror.iter().enumerate() {
        if *enabled {
            let mut scale = Vec3::ONE;
            scale[axis] = -1.0;
            let flipped: Vec<Mat4> = mirrors
                .iter()
                .map(|m| Mat4::from_scale(scale) * *m)
                .collect();
            mirrors.extend(flipped);
        }
    }
    let steps = symmetry.radial.max(1);
    let mut copies = Vec::new();
    for step in 0..steps {
        let turn = Mat4::from_rotation_y(step as f32 / steps as f32 * std::f32::consts::TAU);
        for mirror in &mirrors {
            copies.push(turn * *mirror);
        }
    }
    copies.truncate(MAX_COPIES);
    copies
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_combine_mirrors_and_turns() {
        assert_eq!(copies(&Symmetry::default()), [Mat4::IDENTITY]);
        let x = copies(&Symmetry {
            mirror: [true, false, false],
            radial: 0,
        });
        assert_eq!(x.len(), 2);
        assert_eq!(
            x[1].transform_point3(Vec3::new(1.0, 2.0, 3.0)),
            Vec3::new(-1.0, 2.0, 3.0)
        );
        let xz = copies(&Symmetry {
            mirror: [true, false, true],
            radial: 0,
        });
        assert_eq!(xz.len(), 4);
        assert_eq!(
            xz[3].transform_point3(Vec3::ONE),
            Vec3::new(-1.0, 1.0, -1.0)
        );
        let radial = copies(&Symmetry {
            mirror: [false; 3],
            radial: 4,
        });
        assert_eq!(radial.len(), 4);
        let p = radial[1].transform_point3(Vec3::X);
        assert!(
            (p - Vec3::NEG_Z).length() < 1e-6,
            "a quarter turn around Y: {p}"
        );
        // Capped.
        let many = copies(&Symmetry {
            mirror: [true; 3],
            radial: 8,
        });
        assert_eq!(many.len(), MAX_COPIES);
        assert_eq!(many[0], Mat4::IDENTITY);
    }
}
