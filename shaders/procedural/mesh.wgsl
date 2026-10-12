// Mesh data of a texture set, as read by the procedural passes from storage buffers.
// Must match `GpuVertex` in crates/patine_procedural/src/surface.rs.

struct Vertex {
    // xyz: object-space position.
    position: vec4<f32>,
    // xyz: unit normal.
    normal: vec4<f32>,
    // xyz: tangent, w: bitangent sign (mikktspace).
    tangent: vec4<f32>,
    // xy: first UV set.
    uv: vec4<f32>,
}

// Packs a texel position into a surface-map seed (0 means none).
fn pack_texel(texel: vec2<u32>) -> u32 {
    return (texel.x | (texel.y << 16u)) + 1u;
}

// Unpacks a seed made by `pack_texel` (not 0).
fn unpack_texel(seed: u32) -> vec2<u32> {
    let v = seed - 1u;
    return vec2<u32>(v & 0xffffu, v >> 16u);
}
