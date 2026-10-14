//! Bounding volume hierarchy over the triangles of a mesh, built with the binned SAH.
//!
//! The flattened layout is shared with the GPU (baker, M12): nodes are stored depth
//! first, an interior node's left child immediately follows it, and [`BvhNode`] is 32
//! bytes with std430-compatible alignment.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

/// Largest number of triangles in a leaf, unless they cannot be separated.
pub const MAX_LEAF_TRIANGLES: usize = 4;
const BINS: usize = 16;
/// Below this depth the SAH chooses splits; deeper nodes are split at the median, which
/// bounds the depth to about `SAH_DEPTH + log2(n)` and keeps traversal stacks small.
const SAH_DEPTH: usize = 64;
/// Traversal stack size: enough for `SAH_DEPTH` plus a median tree of 2^32 triangles.
const STACK_SIZE: usize = SAH_DEPTH + 32;

/// Outcome of the split search for a node.
enum Split {
    /// Partitioned at this index.
    At(usize),
    /// Keeping a small leaf is cheaper than any split.
    Leaf,
    /// No plane separates the triangles: split by count.
    Median,
}

/// A node of the flattened hierarchy.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct BvhNode {
    /// Minimum corner of the bounds.
    pub min: [f32; 3],
    /// Interior node: index of the right child (the left child is the next node).
    /// Leaf: index of the first triangle in [`Bvh::triangle_order`].
    pub left_or_first: u32,
    /// Maximum corner of the bounds.
    pub max: [f32; 3],
    /// Number of triangles of a leaf; 0 for an interior node.
    pub count: u32,
}

impl BvhNode {
    /// Whether the node is a leaf.
    pub fn is_leaf(&self) -> bool {
        self.count > 0
    }
}

/// A ray. `direction` does not need to be normalized; hit distances are in its units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    /// Origin.
    pub origin: Vec3,
    /// Direction.
    pub direction: Vec3,
}

/// A ray-triangle intersection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// Triangle index in the mesh.
    pub triangle: u32,
    /// Distance along the ray, in units of its direction.
    pub t: f32,
    /// Barycentric weights of the second and third corners (the first is 1 − u − v).
    pub barycentric: [f32; 2],
}

/// The hierarchy. Queries take the same positions and triangles it was built from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bvh {
    /// Nodes, depth first; node 0 is the root.
    pub nodes: Vec<BvhNode>,
    /// Triangle indices, grouped by leaf.
    pub triangle_order: Vec<u32>,
}

#[derive(Clone, Copy)]
struct Aabb {
    min: Vec3,
    max: Vec3,
}

impl Aabb {
    const EMPTY: Self = Self {
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };

    fn grow(&mut self, point: Vec3) {
        self.min = self.min.min(point);
        self.max = self.max.max(point);
    }

    fn merge(&mut self, other: &Aabb) {
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
    }

    fn half_area(&self) -> f32 {
        let size = (self.max - self.min).max(Vec3::ZERO);
        size.x * size.y + size.y * size.z + size.z * size.x
    }
}

fn triangle_corners(positions: &[[f32; 3]], triangle: [u32; 3]) -> [Vec3; 3] {
    triangle.map(|v| Vec3::from(positions[v as usize]))
}

/// Builder state shared by the recursion.
struct Builder<'a> {
    bounds: &'a [Aabb],
    centroids: &'a [Vec3],
    order: Vec<u32>,
    nodes: Vec<BvhNode>,
}

impl Builder<'_> {
    /// Builds the subtree over `order[start..end]` at `depth` and returns its node index.
    fn build(&mut self, start: usize, end: usize, depth: usize) -> u32 {
        let index = self.nodes.len();
        let mut bounds = Aabb::EMPTY;
        let mut centroid_bounds = Aabb::EMPTY;
        for &triangle in &self.order[start..end] {
            bounds.merge(&self.bounds[triangle as usize]);
            centroid_bounds.grow(self.centroids[triangle as usize]);
        }
        self.nodes.push(BvhNode {
            min: bounds.min.to_array(),
            left_or_first: start as u32,
            max: bounds.max.to_array(),
            count: (end - start) as u32,
        });
        let count = end - start;
        if count <= MAX_LEAF_TRIANGLES {
            return index as u32;
        }

        let split = if depth < SAH_DEPTH {
            self.sah_split(start, end, &bounds, &centroid_bounds)
        } else {
            Split::Median
        };
        let mid = match split {
            Split::At(mid) => mid,
            Split::Leaf => return index as u32,
            Split::Median => {
                let axis = (centroid_bounds.max - centroid_bounds.min).max_position();
                let centroids = self.centroids;
                self.order[start..end].select_nth_unstable_by(count / 2, |a, b| {
                    centroids[*a as usize][axis].total_cmp(&centroids[*b as usize][axis])
                });
                start + count / 2
            }
        };

        self.build(start, mid, depth + 1);
        let right = self.build(mid, end, depth + 1);
        let node = &mut self.nodes[index];
        node.left_or_first = right;
        node.count = 0;
        index as u32
    }

    /// Partitions `order[start..end]` at the best SAH plane, if any.
    fn sah_split(
        &mut self,
        start: usize,
        end: usize,
        bounds: &Aabb,
        centroid_bounds: &Aabb,
    ) -> Split {
        let extent = centroid_bounds.max - centroid_bounds.min;
        let mut best: Option<(f32, usize, usize)> = None; // (cost, axis, split bin)
        for axis in 0..3 {
            if extent[axis] <= 0.0 {
                continue;
            }
            let scale = BINS as f32 / extent[axis];
            let bin_of = |centroid: Vec3| {
                (((centroid[axis] - centroid_bounds.min[axis]) * scale) as usize).min(BINS - 1)
            };
            let mut bin_bounds = [Aabb::EMPTY; BINS];
            let mut bin_counts = [0usize; BINS];
            for &triangle in &self.order[start..end] {
                let bin = bin_of(self.centroids[triangle as usize]);
                bin_bounds[bin].merge(&self.bounds[triangle as usize]);
                bin_counts[bin] += 1;
            }
            // Sweep from the right, then evaluate each plane from the left.
            let mut right_area = [0.0f32; BINS];
            let mut right_count = [0usize; BINS];
            let mut accumulated = Aabb::EMPTY;
            let mut count = 0;
            for bin in (1..BINS).rev() {
                accumulated.merge(&bin_bounds[bin]);
                count += bin_counts[bin];
                right_area[bin] = accumulated.half_area();
                right_count[bin] = count;
            }
            let mut left = Aabb::EMPTY;
            let mut left_count = 0;
            for split in 1..BINS {
                left.merge(&bin_bounds[split - 1]);
                left_count += bin_counts[split - 1];
                if left_count == 0 || right_count[split] == 0 {
                    continue;
                }
                let cost = left.half_area() * left_count as f32
                    + right_area[split] * right_count[split] as f32;
                if best.is_none_or(|(best_cost, _, _)| cost < best_cost) {
                    best = Some((cost, axis, split));
                }
            }
        }

        let Some((cost, axis, split)) = best else {
            return Split::Median;
        };
        // Traversal cost 1, intersection cost 1, relative to the parent's area.
        let split_cost = 1.0 + cost / bounds.half_area().max(f32::MIN_POSITIVE);
        let leaf_cost = (end - start) as f32;
        if split_cost >= leaf_cost && end - start <= 2 * MAX_LEAF_TRIANGLES {
            return Split::Leaf;
        }

        let scale = BINS as f32 / extent[axis];
        let centroids = self.centroids;
        let min = centroid_bounds.min[axis];
        let slice = &mut self.order[start..end];
        let mut left = 0;
        for i in 0..slice.len() {
            let bin = (((centroids[slice[i] as usize][axis] - min) * scale) as usize).min(BINS - 1);
            if bin < split {
                slice.swap(i, left);
                left += 1;
            }
        }
        if left > 0 && left < slice.len() {
            Split::At(start + left)
        } else {
            Split::Median
        }
    }
}

/// Slab test: entry distance of the ray into the node, if it hits within `t_max`.
fn ray_box(node: &BvhNode, origin: Vec3, inv_direction: Vec3, t_max: f32) -> Option<f32> {
    let t0 = (Vec3::from(node.min) - origin) * inv_direction;
    let t1 = (Vec3::from(node.max) - origin) * inv_direction;
    // NaN (0 × ∞ on a slab boundary) is discarded by min/max.
    let near = t0.min(t1).max_element().max(0.0);
    let far = t0.max(t1).min_element().min(t_max);
    (near <= far).then_some(near)
}

/// Möller–Trumbore intersection, two-sided.
fn ray_triangle(ray: &Ray, corners: [Vec3; 3], t_max: f32) -> Option<(f32, f32, f32)> {
    let edge1 = corners[1] - corners[0];
    let edge2 = corners[2] - corners[0];
    let p = ray.direction.cross(edge2);
    let determinant = edge1.dot(p);
    if determinant.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / determinant;
    let s = ray.origin - corners[0];
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(edge1);
    let v = ray.direction.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = edge2.dot(q) * inv;
    (t >= 0.0 && t <= t_max).then_some((t, u, v))
}

impl Bvh {
    /// Builds the hierarchy over `triangles`.
    pub fn build(positions: &[[f32; 3]], triangles: &[[u32; 3]]) -> Self {
        if triangles.is_empty() {
            return Self::default();
        }
        let bounds: Vec<Aabb> = triangles
            .iter()
            .map(|&triangle| {
                let mut aabb = Aabb::EMPTY;
                for corner in triangle_corners(positions, triangle) {
                    aabb.grow(corner);
                }
                aabb
            })
            .collect();
        let centroids: Vec<Vec3> = bounds.iter().map(|b| (b.min + b.max) * 0.5).collect();
        let mut builder = Builder {
            bounds: &bounds,
            centroids: &centroids,
            order: (0..triangles.len() as u32).collect(),
            nodes: Vec::with_capacity(2 * triangles.len() / MAX_LEAF_TRIANGLES + 1),
        };
        builder.build(0, triangles.len(), 0);
        Self {
            nodes: builder.nodes,
            triangle_order: builder.order,
        }
    }

    /// Visits the leaves whose bounds the ray enters before `t_max`, nearest child first.
    /// `visit` returns the new `t_max`, or `None` to stop.
    fn traverse_ray(
        &self,
        ray: &Ray,
        mut t_max: f32,
        mut visit: impl FnMut(&[u32], f32) -> Option<f32>,
    ) {
        if self.nodes.is_empty() {
            return;
        }
        let inv_direction = ray.direction.recip();
        let mut stack = [0u32; STACK_SIZE];
        let mut top = 0;
        let mut node_index = 0u32;
        loop {
            let node = &self.nodes[node_index as usize];
            if ray_box(node, ray.origin, inv_direction, t_max).is_some() {
                if node.is_leaf() {
                    let first = node.left_or_first as usize;
                    match visit(
                        &self.triangle_order[first..first + node.count as usize],
                        t_max,
                    ) {
                        Some(t) => t_max = t,
                        None => return,
                    }
                } else {
                    let (left, right) = (node_index + 1, node.left_or_first);
                    let t_left =
                        ray_box(&self.nodes[left as usize], ray.origin, inv_direction, t_max);
                    let t_right = ray_box(
                        &self.nodes[right as usize],
                        ray.origin,
                        inv_direction,
                        t_max,
                    );
                    let (near, far) = match (t_left, t_right) {
                        (Some(a), Some(b)) if b < a => (Some(right), Some(left)),
                        (Some(_), b) => (Some(left), b.map(|_| right)),
                        (None, Some(_)) => (Some(right), None),
                        (None, None) => (None, None),
                    };
                    if let Some(far) = far {
                        stack[top] = far;
                        top += 1;
                    }
                    if let Some(near) = near {
                        node_index = near;
                        continue;
                    }
                }
            }
            if top == 0 {
                return;
            }
            top -= 1;
            node_index = stack[top];
        }
    }

    /// Closest intersection with `ray` at a distance in `[0, t_max]`.
    pub fn closest_hit(
        &self,
        positions: &[[f32; 3]],
        triangles: &[[u32; 3]],
        ray: &Ray,
        t_max: f32,
    ) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        self.traverse_ray(ray, t_max, |leaf, t_max| {
            let mut t_max = t_max;
            for &triangle in leaf {
                let corners = triangle_corners(positions, triangles[triangle as usize]);
                if let Some((t, u, v)) = ray_triangle(ray, corners, t_max) {
                    t_max = t;
                    best = Some(Hit {
                        triangle,
                        t,
                        barycentric: [u, v],
                    });
                }
            }
            Some(t_max)
        });
        best
    }

    /// Whether `ray` hits any triangle at a distance in `[0, t_max]`.
    pub fn any_hit(
        &self,
        positions: &[[f32; 3]],
        triangles: &[[u32; 3]],
        ray: &Ray,
        t_max: f32,
    ) -> bool {
        let mut hit = false;
        self.traverse_ray(ray, t_max, |leaf, t_max| {
            hit = leaf.iter().any(|&triangle| {
                let corners = triangle_corners(positions, triangles[triangle as usize]);
                ray_triangle(ray, corners, t_max).is_some()
            });
            (!hit).then_some(t_max)
        });
        hit
    }

    /// Appends to `out` every triangle whose bounding box overlaps the box `min..max`.
    pub fn query_aabb(
        &self,
        positions: &[[f32; 3]],
        triangles: &[[u32; 3]],
        min: Vec3,
        max: Vec3,
        out: &mut Vec<u32>,
    ) {
        self.query_bounds(
            positions,
            triangles,
            |node_min, node_max| node_min.cmple(max).all() && node_max.cmpge(min).all(),
            out,
        );
    }

    /// Appends to `out` every triangle whose bounding box passes `overlaps`, a test of a
    /// box (minimum and maximum corners) that must hold for any box containing one that
    /// passes: subtrees whose bounds fail are skipped.
    pub fn query_bounds(
        &self,
        positions: &[[f32; 3]],
        triangles: &[[u32; 3]],
        mut overlaps: impl FnMut(Vec3, Vec3) -> bool,
        out: &mut Vec<u32>,
    ) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0u32];
        while let Some(index) = stack.pop() {
            let node = &self.nodes[index as usize];
            if !overlaps(Vec3::from(node.min), Vec3::from(node.max)) {
                continue;
            }
            if node.is_leaf() {
                let first = node.left_or_first as usize;
                for &triangle in &self.triangle_order[first..first + node.count as usize] {
                    let corners = triangle_corners(positions, triangles[triangle as usize]);
                    let t_min = corners[0].min(corners[1]).min(corners[2]);
                    let t_max = corners[0].max(corners[1]).max(corners[2]);
                    if overlaps(t_min, t_max) {
                        out.push(triangle);
                    }
                }
            } else {
                stack.push(node.left_or_first);
                stack.push(index + 1);
            }
        }
    }

    /// Depth of the deepest leaf (the root alone has depth 1).
    pub fn depth(&self) -> usize {
        fn depth(nodes: &[BvhNode], index: usize) -> usize {
            let node = &nodes[index];
            if node.is_leaf() {
                1
            } else {
                1 + depth(nodes, index + 1).max(depth(nodes, node.left_or_first as usize))
            }
        }
        if self.nodes.is_empty() {
            0
        } else {
            depth(&self.nodes, 0)
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::primitives;

    #[test]
    fn node_is_32_bytes() {
        assert_eq!(std::mem::size_of::<BvhNode>(), 32);
    }

    fn check_invariants(bvh: &Bvh, positions: &[[f32; 3]], triangles: &[[u32; 3]]) {
        let mut sorted = bvh.triangle_order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..triangles.len() as u32).collect::<Vec<_>>());
        let mut covered = vec![false; triangles.len()];
        for (index, node) in bvh.nodes.iter().enumerate() {
            let (min, max) = (Vec3::from(node.min), Vec3::from(node.max));
            if node.is_leaf() {
                let first = node.left_or_first as usize;
                for &triangle in &bvh.triangle_order[first..first + node.count as usize] {
                    assert!(!covered[triangle as usize]);
                    covered[triangle as usize] = true;
                    for corner in triangle_corners(positions, triangles[triangle as usize]) {
                        assert!(corner.cmpge(min).all() && corner.cmple(max).all());
                    }
                }
            } else {
                for child in [index + 1, node.left_or_first as usize] {
                    assert!(child > index, "children follow their parent");
                    let c = &bvh.nodes[child];
                    assert!(Vec3::from(c.min).cmpge(min).all());
                    assert!(Vec3::from(c.max).cmple(max).all());
                }
            }
        }
        assert!(covered.iter().all(|c| *c));
    }

    #[test]
    fn invariants_on_generated_meshes() {
        for mesh in [
            primitives::cube(),
            primitives::uv_sphere(64, 32),
            primitives::two_cubes(),
            primitives::plane(20),
        ] {
            let bvh = Bvh::build(&mesh.positions, &mesh.triangles);
            check_invariants(&bvh, &mesh.positions, &mesh.triangles);
            let leaves = bvh.nodes.iter().filter(|n| n.is_leaf());
            assert!(
                leaves
                    .clone()
                    .all(|n| n.count as usize <= 2 * MAX_LEAF_TRIANGLES)
            );
            assert!(bvh.depth() < 40, "{} depth {}", mesh.name, bvh.depth());
        }
    }

    #[test]
    fn empty_mesh_has_no_nodes() {
        let bvh = Bvh::build(&[], &[]);
        assert!(bvh.nodes.is_empty());
        let ray = Ray {
            origin: Vec3::ZERO,
            direction: Vec3::X,
        };
        assert!(bvh.closest_hit(&[], &[], &ray, f32::INFINITY).is_none());
    }

    fn brute_force(positions: &[[f32; 3]], triangles: &[[u32; 3]], ray: &Ray) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        for (index, &triangle) in triangles.iter().enumerate() {
            let t_max = best.map_or(f32::INFINITY, |hit| hit.t);
            if let Some((t, u, v)) = ray_triangle(ray, triangle_corners(positions, triangle), t_max)
            {
                best = Some(Hit {
                    triangle: index as u32,
                    t,
                    barycentric: [u, v],
                });
            }
        }
        best
    }

    #[test]
    fn ray_from_outside_hits_the_sphere_at_distance_two() {
        let mesh = primitives::uv_sphere(64, 32);
        let bvh = Bvh::build(&mesh.positions, &mesh.triangles);
        let ray = Ray {
            origin: Vec3::new(0.1, 0.2, 3.0),
            direction: Vec3::NEG_Z,
        };
        let hit = bvh
            .closest_hit(&mesh.positions, &mesh.triangles, &ray, f32::INFINITY)
            .unwrap();
        // Unit sphere: z = sqrt(1 - 0.05) ≈ 0.9747, so t ≈ 2.025 (tessellation lowers it).
        assert!((hit.t - 2.0253).abs() < 0.01, "{}", hit.t);
        assert!(bvh.any_hit(&mesh.positions, &mesh.triangles, &ray, f32::INFINITY));
        assert!(!bvh.any_hit(&mesh.positions, &mesh.triangles, &ray, 1.5));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn closest_hit_matches_brute_force(
            origin in prop::array::uniform3(-3.0f32..3.0),
            target in prop::array::uniform3(-1.2f32..1.2),
        ) {
            let mesh = primitives::two_cubes();
            let bvh = Bvh::build(&mesh.positions, &mesh.triangles);
            let origin = Vec3::from(origin);
            let direction = Vec3::from(target) - origin;
            prop_assume!(direction.length() > 1e-3);
            let ray = Ray { origin, direction };
            let expected = brute_force(&mesh.positions, &mesh.triangles, &ray);
            let actual = bvh.closest_hit(&mesh.positions, &mesh.triangles, &ray, f32::INFINITY);
            match (expected, actual) {
                (None, None) => {}
                (Some(e), Some(a)) => prop_assert!((e.t - a.t).abs() <= 1e-5 * e.t.max(1.0)),
                (e, a) => prop_assert!(false, "expected {:?}, got {:?}", e, a),
            }
            prop_assert_eq!(
                bvh.any_hit(&mesh.positions, &mesh.triangles, &ray, f32::INFINITY),
                expected.is_some()
            );
        }

        #[test]
        fn aabb_query_matches_brute_force(
            center in prop::array::uniform3(-3.0f32..3.0),
            half in prop::array::uniform3(0.0f32..1.5),
        ) {
            let mesh = primitives::uv_sphere(32, 16);
            let bvh = Bvh::build(&mesh.positions, &mesh.triangles);
            let (min, max) = (Vec3::from(center) - Vec3::from(half), Vec3::from(center) + Vec3::from(half));
            let mut actual = Vec::new();
            bvh.query_aabb(&mesh.positions, &mesh.triangles, min, max, &mut actual);
            actual.sort_unstable();
            let expected: Vec<u32> = (0..mesh.triangle_count() as u32)
                .filter(|&t| {
                    let c = triangle_corners(&mesh.positions, mesh.triangles[t as usize]);
                    let (t_min, t_max) = (c[0].min(c[1]).min(c[2]), c[0].max(c[1]).max(c[2]));
                    t_min.cmple(max).all() && t_max.cmpge(min).all()
                })
                .collect();
            prop_assert_eq!(actual, expected);
        }
    }
}
