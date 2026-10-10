// Depth-only pass of the dominant light's shadow map.

// #import render/frame

@group(0) @binding(0) var<uniform> frame: Frame;

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return frame.light_view_proj * vec4<f32>(position, 1.0);
}
