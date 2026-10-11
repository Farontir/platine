// Display encoding of a channel value, shared by the channel display mode of the 3D view
// and the 2D view.

// #import render/color

// Display-encoded value of a channel of kind `kind` (0 color, 1 scalar, 2 normal):
// colors sRGB-encoded, data as is (0.5 = middle gray), normals as n × 0.5 + 0.5.
fn display_encode(value: vec4<f32>, kind: f32) -> vec3<f32> {
    if kind < 0.5 {
        return srgb_encode(value.rgb);
    }
    if kind < 1.5 {
        return vec3<f32>(clamp(value.r, 0.0, 1.0));
    }
    return clamp(value.xyz * 0.5 + 0.5, vec3<f32>(0.0), vec3<f32>(1.0));
}
