//! Mesh import (glTF, OBJ, FBX), mikktspace tangents, adjacency, UV islands and the SAH BVH.
//!
//! Importers and generators produce a [`RawMesh`] (unindexed triangle corners);
//! [`assemble`] turns it into the document's [`MeshAsset`](patine_core::mesh::MeshAsset).

mod assemble;
mod error;
mod hash;
pub mod primitives;
pub mod raw;

pub use assemble::{BuildOptions, BuildReport, assemble};
pub use error::MeshError;
pub use raw::RawMesh;
