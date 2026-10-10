// Polygon edges drawn as lines over the shaded mesh.

// #import render/frame

@group(0) @binding(0) var<uniform> frame: Frame;

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    var clip = frame.view_proj * vec4<f32>(position, 1.0);
    // Pull lines slightly towards the viewer (reverse-Z: larger depth is closer) so they
    // win the depth test against the faces they lie on.
    clip.z = clip.z + 2e-4 * clip.w;
    return clip;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return frame.wire_color;
}
