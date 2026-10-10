// Mesh shading pass. Provisional lighting (one key light and a sky/ground ambient) until
// the PBR shader replaces it.

// #import render/frame

@group(0) @binding(0) var<uniform> frame: Frame;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec4<f32>,
    @location(3) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    // Interpolated without renormalization in the vertex stage (mikktspace rule).
    @location(1) normal: vec3<f32>,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip = frame.view_proj * vec4<f32>(in.position, 1.0);
    out.world_position = in.position;
    out.normal = in.normal;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let light = normalize(vec3<f32>(0.5, 0.8, 0.3));
    let sky = mix(vec3<f32>(0.05), vec3<f32>(0.25), n.y * 0.5 + 0.5);
    let color = frame.base_color.rgb * (sky + max(dot(n, light), 0.0));
    return vec4<f32>(color, 1.0);
}
