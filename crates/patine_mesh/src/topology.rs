//! Triangle adjacency, UV islands and connected parts.
//!
//! Two triangles are *neighbors* when they share a geometric edge: two corners at
//! bit-identical positions, whatever their normals or UVs. They are in the same *UV
//! island* when, in addition, both ends of that edge have identical UVs on both sides.
//! *Parts* are the connected components of the neighbor graph. Non-manifold edges
//! (shared by more than two triangles) connect every triangle around them.

use patine_core::mesh::MeshAsset;

use crate::hash::{FxHashMap, float_key, position_key};

/// Partition of the triangles into connected components.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Components {
    /// Component of each triangle. Components are numbered in order of their first
    /// triangle.
    pub of_triangle: Vec<u32>,
    /// Number of components.
    pub count: u32,
}

impl Components {
    /// Triangles of component `component`, in increasing order.
    pub fn triangles(&self, component: u32) -> impl Iterator<Item = u32> + '_ {
        self.of_triangle
            .iter()
            .enumerate()
            .filter(move |(_, c)| **c == component)
            .map(|(triangle, _)| triangle as u32)
    }
}

/// Connectivity of a mesh, derived from its geometry. Never stored in the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshTopology {
    neighbor_offsets: Vec<u32>,
    neighbors: Vec<u32>,
    /// UV islands, if the mesh has UVs.
    pub uv_islands: Option<Components>,
    /// Connected parts.
    pub parts: Components,
}

/// Union-find with path halving and union by size.
struct DisjointSets {
    parent: Vec<u32>,
    size: Vec<u32>,
}

impl DisjointSets {
    fn new(count: usize) -> Self {
        Self {
            parent: (0..count as u32).collect(),
            size: vec![1; count],
        }
    }

    fn find(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            let grandparent = self.parent[self.parent[x as usize] as usize];
            self.parent[x as usize] = grandparent;
            x = grandparent;
        }
        x
    }

    fn union(&mut self, a: u32, b: u32) {
        let (mut a, mut b) = (self.find(a), self.find(b));
        if a == b {
            return;
        }
        if self.size[a as usize] < self.size[b as usize] {
            std::mem::swap(&mut a, &mut b);
        }
        self.parent[b as usize] = a;
        self.size[a as usize] += self.size[b as usize];
    }

    /// Component labels numbered in order of first appearance.
    fn into_components(mut self) -> Components {
        let mut label = vec![u32::MAX; self.parent.len()];
        let mut count = 0;
        let of_triangle = (0..self.parent.len() as u32)
            .map(|x| {
                let root = self.find(x) as usize;
                if label[root] == u32::MAX {
                    label[root] = count;
                    count += 1;
                }
                label[root]
            })
            .collect();
        Components { of_triangle, count }
    }
}

/// One side of a geometric edge.
struct EdgeSide {
    /// Position ids of the two ends, smallest first.
    key: (u32, u32),
    triangle: u32,
    /// UV bits at the `key.0` end and at the `key.1` end.
    uvs: Option<([u32; 2], [u32; 2])>,
}

impl MeshTopology {
    /// Computes the topology of `mesh`.
    pub fn new(mesh: &MeshAsset) -> Self {
        let mut position_ids: FxHashMap<[u32; 3], u32> = FxHashMap::default();
        let vertex_position: Vec<u32> = mesh
            .positions
            .iter()
            .map(|p| {
                let next = position_ids.len() as u32;
                *position_ids.entry(position_key(*p)).or_insert(next)
            })
            .collect();
        let uv_key = |vertex: u32| {
            mesh.uv0
                .as_ref()
                .map(|uvs| uvs[vertex as usize].map(float_key))
        };

        let mut sides = Vec::with_capacity(mesh.triangle_count() * 3);
        for (triangle, corners) in mesh.triangles.iter().enumerate() {
            for edge in 0..3 {
                let (va, vb) = (corners[edge], corners[(edge + 1) % 3]);
                let (pa, pb) = (vertex_position[va as usize], vertex_position[vb as usize]);
                if pa == pb {
                    continue;
                }
                let (low, high) = if pa < pb { (va, vb) } else { (vb, va) };
                sides.push(EdgeSide {
                    key: (pa.min(pb), pa.max(pb)),
                    triangle: triangle as u32,
                    uvs: uv_key(low).zip(uv_key(high)),
                });
            }
        }
        sides.sort_unstable_by_key(|side| (side.key, side.triangle));

        let triangles = mesh.triangle_count();
        let mut parts = DisjointSets::new(triangles);
        let mut islands = mesh.uv0.as_ref().map(|_| DisjointSets::new(triangles));
        let mut pairs: Vec<(u32, u32)> = Vec::new();
        for group in sides.chunk_by(|a, b| a.key == b.key) {
            for (i, a) in group.iter().enumerate() {
                for b in &group[i + 1..] {
                    if a.triangle == b.triangle {
                        continue;
                    }
                    pairs.push((a.triangle, b.triangle));
                    pairs.push((b.triangle, a.triangle));
                    parts.union(a.triangle, b.triangle);
                    if let Some(islands) = &mut islands
                        && a.uvs == b.uvs
                    {
                        islands.union(a.triangle, b.triangle);
                    }
                }
            }
        }

        // Compressed adjacency lists, sorted and deduplicated.
        pairs.sort_unstable();
        pairs.dedup();
        let mut neighbor_offsets = vec![0u32; triangles + 1];
        for &(triangle, _) in &pairs {
            neighbor_offsets[triangle as usize + 1] += 1;
        }
        for i in 0..triangles {
            neighbor_offsets[i + 1] += neighbor_offsets[i];
        }
        let neighbors = pairs.into_iter().map(|(_, neighbor)| neighbor).collect();

        Self {
            neighbor_offsets,
            neighbors,
            uv_islands: islands.map(DisjointSets::into_components),
            parts: parts.into_components(),
        }
    }

    /// Triangles sharing a geometric edge with `triangle`, sorted.
    pub fn neighbors(&self, triangle: u32) -> &[u32] {
        let start = self.neighbor_offsets[triangle as usize] as usize;
        let end = self.neighbor_offsets[triangle as usize + 1] as usize;
        &self.neighbors[start..end]
    }

    /// Number of UV islands, or `None` without UVs.
    pub fn island_count(&self) -> Option<u32> {
        self.uv_islands.as_ref().map(|islands| islands.count)
    }

    /// Number of connected parts.
    pub fn part_count(&self) -> u32 {
        self.parts.count
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::primitives;

    /// Acceptance criterion 2 of M1: exact UV island counts on the generated meshes.
    #[test]
    fn island_and_part_counts_of_generated_meshes() {
        let cases = [
            (primitives::plane(5), 1, 1),
            (primitives::cube(), 6, 1),
            (primitives::cube_cross(), 1, 1),
            (primitives::uv_sphere(24, 12), 1, 1),
            (primitives::cylinder(16), 3, 1),
            (primitives::two_cubes(), 12, 2),
        ];
        for (mesh, islands, parts) in cases {
            let topology = MeshTopology::new(&mesh);
            assert_eq!(
                topology.island_count(),
                Some(islands),
                "{} islands",
                mesh.name
            );
            assert_eq!(topology.part_count(), parts, "{} parts", mesh.name);
        }
    }

    #[test]
    fn closed_meshes_have_three_neighbors_per_triangle() {
        for mesh in [
            primitives::cube(),
            primitives::uv_sphere(16, 8),
            primitives::cylinder(8),
        ] {
            let topology = MeshTopology::new(&mesh);
            for triangle in 0..mesh.triangle_count() as u32 {
                assert_eq!(topology.neighbors(triangle).len(), 3, "{}", mesh.name);
            }
        }
    }

    #[test]
    fn plane_borders_have_fewer_neighbors() {
        let mesh = primitives::plane(3);
        let topology = MeshTopology::new(&mesh);
        let counts: Vec<usize> = (0..mesh.triangle_count() as u32)
            .map(|t| topology.neighbors(t).len())
            .collect();
        assert!(counts.iter().all(|&c| (1..=3).contains(&c)));
        assert!(counts.contains(&1) && counts.contains(&3));
        // Adjacency is symmetric.
        for t in 0..mesh.triangle_count() as u32 {
            for &n in topology.neighbors(t) {
                assert!(topology.neighbors(n).contains(&t));
            }
        }
    }

    #[test]
    fn mesh_without_uvs_has_parts_but_no_islands() {
        let pair = primitives::low_high_pair();
        let topology = MeshTopology::new(&pair.high);
        assert_eq!(topology.island_count(), None);
        assert_eq!(topology.part_count(), 1);
    }

    #[test]
    fn non_manifold_edge_connects_all_its_triangles() {
        // Three quads sharing the edge x = 0 on the Z axis (a "book" with three pages).
        let mut mesh = primitives::plane(1);
        let base = mesh.clone();
        for angle in [2.0f32, 4.0] {
            let (sin, cos) = angle.sin_cos();
            let offset = mesh.positions.len() as u32;
            mesh.positions.extend(base.positions.iter().map(|p| {
                // Rotate around Z after moving the edge x = -1 onto the axis.
                let x = p[0] + 1.0;
                [x * cos - 1.0, x * sin, p[2]]
            }));
            mesh.normals.extend(&base.normals);
            mesh.uv0
                .as_mut()
                .unwrap()
                .extend(base.uv0.as_ref().unwrap());
            mesh.tangents
                .as_mut()
                .unwrap()
                .extend(base.tangents.as_ref().unwrap());
            mesh.triangles
                .extend(base.triangles.iter().map(|t| t.map(|v| v + offset)));
            mesh.triangle_polygon.extend(&base.triangle_polygon);
            mesh.triangle_object.extend(&base.triangle_object);
        }
        mesh.submeshes[0].triangles.end = mesh.triangles.len() as u32;
        mesh.validate().unwrap();
        let topology = MeshTopology::new(&mesh);
        assert_eq!(topology.part_count(), 1);
        let shared = (0..mesh.triangle_count() as u32)
            .filter(|&t| topology.neighbors(t).len() >= 3)
            .count();
        assert!(shared >= 3);
    }

    /// Shuffles triangles (Fisher-Yates with a small LCG), keeping per-triangle arrays
    /// in step. Submeshes stay valid because the meshes have a single material.
    fn shuffle(mesh: &MeshAsset, seed: u64) -> MeshAsset {
        let mut order: Vec<usize> = (0..mesh.triangle_count()).collect();
        let mut state = seed | 1;
        for i in (1..order.len()).rev() {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            order.swap(i, (state >> 33) as usize % (i + 1));
        }
        let mut shuffled = mesh.clone();
        shuffled.triangles = order.iter().map(|&t| mesh.triangles[t]).collect();
        shuffled.triangle_polygon = order.iter().map(|&t| mesh.triangle_polygon[t]).collect();
        shuffled.triangle_object = order.iter().map(|&t| mesh.triangle_object[t]).collect();
        shuffled
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn counts_do_not_depend_on_triangle_order(seed in any::<u64>()) {
            for mesh in [primitives::cube(), primitives::cylinder(10), primitives::two_cubes()] {
                let reference = MeshTopology::new(&mesh);
                let shuffled = shuffle(&mesh, seed);
                shuffled.validate().unwrap();
                let topology = MeshTopology::new(&shuffled);
                prop_assert_eq!(topology.island_count(), reference.island_count());
                prop_assert_eq!(topology.part_count(), reference.part_count());
            }
        }
    }
}
