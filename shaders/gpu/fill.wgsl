// Fills an Rgba16Float storage texture with a constant color.

struct FillParams {
    color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: FillParams;
@group(0) @binding(1) var output: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    textureStore(output, vec2<i32>(id.xy), params.color);
}
