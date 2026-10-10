// Next cube mip level: 2 × 2 box filter inside each face (like reference::downsample_cube).

@group(0) @binding(0) var source: texture_2d_array<f32>;
@group(0) @binding(1) var output: texture_storage_2d_array<rgba16float, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output).x;
    if id.x >= size || id.y >= size {
        return;
    }
    let last = i32(textureDimensions(source).x) - 1;
    let base = vec2<i32>(id.xy) * 2;
    let layer = i32(id.z);
    var sum = vec3<f32>(0.0);
    for (var dy = 0; dy < 2; dy++) {
        for (var dx = 0; dx < 2; dx++) {
            let p = min(base + vec2<i32>(dx, dy), vec2<i32>(last));
            sum += textureLoad(source, p, layer, 0).rgb;
        }
    }
    textureStore(output, vec2<i32>(id.xy), layer, vec4<f32>(sum * 0.25, 1.0));
}
