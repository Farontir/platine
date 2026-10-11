// One mip level from the previous one: average of each 2 × 2 block, renormalized for
// normals. Must match `reference::downsample`.

// #import compositor/blend_ops
// #import render/fullscreen

struct Down {
    // x: channel kind (2 for normals).
    params: vec4<f32>,
}

@group(0) @binding(0) var<uniform> down: Down;
@group(0) @binding(1) var source: texture_2d<f32>;

@fragment
fn fs_main(in: FullscreenOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<u32>(in.clip.xy) * 2u;
    let last = textureDimensions(source) - vec2<u32>(1u);
    var sum = vec4<f32>(0.0);
    for (var i = 0u; i < 4u; i++) {
        let offset = vec2<u32>(i & 1u, i >> 1u);
        sum += textureLoad(source, min(pixel + offset, last), 0);
    }
    var average = sum * 0.25;
    if u32(down.params.x) == 2u {
        average = vec4<f32>(safe_normalize(average.xyz), average.w);
    }
    return average;
}
