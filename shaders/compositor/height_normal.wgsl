// Shading normal of a texture set: the normal channel (or a flat normal) combined, by
// reoriented normal mapping, with the normal of the height channel's relief:
// normalize(−k ∂h/∂u, −k ∂h/∂v, 1), gradients by central differences in UV units, with
// k = height force × HEIGHT_SCALE.
// Must match `reference::height_normal` and `HeightNormalUniforms` in
// crates/patine_compositor/src/compositor.rs.

// #import compositor/blend_ops
// #import render/fullscreen

struct HeightNormal {
    // x: k, y: texture size, z: 1 to read the normal channel.
    params: vec4<f32>,
}

@group(0) @binding(0) var<uniform> hn: HeightNormal;
@group(0) @binding(1) var height: texture_2d<f32>;
@group(0) @binding(2) var normal: texture_2d<f32>;

fn h(p: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(height));
    return textureLoad(height, clamp(p, vec2<i32>(0), size - vec2<i32>(1)), 0).r;
}

@fragment
fn fs_main(in: FullscreenOutput) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.clip.xy);
    let k = hn.params.x;
    let size = hn.params.y;
    let du = (h(p + vec2<i32>(1, 0)) - h(p - vec2<i32>(1, 0))) * 0.5 * size;
    let dv = (h(p + vec2<i32>(0, 1)) - h(p - vec2<i32>(0, 1))) * 0.5 * size;
    let detail = safe_normalize(vec3<f32>(-k * du, -k * dv, 1.0));
    var base = vec3<f32>(0.0, 0.0, 1.0);
    if hn.params.z > 0.5 {
        base = textureLoad(normal, p, 0).xyz;
    }
    return vec4<f32>(reoriented_normal(base, detail), 0.0);
}
