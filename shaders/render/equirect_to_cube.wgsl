// Equirectangular radiance (Rgba32Float, read with textureLoad) to one cube mip level.
// Bilinear sampling with wrapping longitude and clamped colatitude, like
// EquirectImage::sample.

// #import render/cube

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var output: texture_storage_2d_array<rgba16float, write>;

fn texel(p: vec2<i32>, size: vec2<i32>) -> vec3<f32> {
    let x = ((p.x % size.x) + size.x) % size.x;
    let y = clamp(p.y, 0, size.y - 1);
    return textureLoad(source, vec2<i32>(x, y), 0).rgb;
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let face_size = textureDimensions(output).x;
    if id.x >= face_size || id.y >= face_size {
        return;
    }
    let d = cube_texel_direction(id.z, id.xy, face_size);
    let size = vec2<i32>(textureDimensions(source));
    let uv = direction_to_equirect(d);
    let p = uv * vec2<f32>(size) - 0.5;
    let p0 = floor(p);
    let f = p - p0;
    let i = vec2<i32>(p0);
    let top = mix(texel(i, size), texel(i + vec2<i32>(1, 0), size), f.x);
    let bottom = mix(texel(i + vec2<i32>(0, 1), size), texel(i + vec2<i32>(1, 1), size), f.x);
    textureStore(output, vec2<i32>(id.xy), i32(id.z), vec4<f32>(mix(top, bottom, f.y), 1.0));
}
