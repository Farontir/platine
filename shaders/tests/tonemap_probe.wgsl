// Test probe: applies every display operator to a buffer of colors, so the WGSL
// operators can be compared with reference::tone_map. Used by tests only.

// #import render/color
// #import render/tonemap_ops

@group(0) @binding(0) var<storage, read> inputs: array<vec4<f32>>;
// Four results per input: AgX, ACES, PBR Neutral, linear (sRGB-encoded in w = 1 rows).
@group(0) @binding(1) var<storage, read_write> outputs: array<vec4<f32>>;

@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= arrayLength(&inputs) {
        return;
    }
    let color = inputs[i].rgb;
    for (var op = 0u; op < 4u; op++) {
        outputs[i * 4u + op] = vec4<f32>(srgb_encode(tone_map(color, op)), 1.0);
    }
}
