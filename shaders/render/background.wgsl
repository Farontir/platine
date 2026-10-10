// Background behind the mesh: the environment (optionally blurred) or a solid color.

// #import render/frame
// #import render/environment
// #import render/fullscreen

@group(0) @binding(0) var<uniform> frame: Frame;

@fragment
fn fs_main(in: FullscreenOutput) -> @location(0) vec4<f32> {
    if frame.background.a < 0.5 {
        return vec4<f32>(frame.background.rgb, 1.0);
    }
    // View ray through the pixel, valid for both projections (reverse-Z: depth 1 is
    // the near plane, 0.5 lies further along the ray).
    let ndc = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let near = frame.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    let far = frame.inv_view_proj * vec4<f32>(ndc, 0.5, 1.0);
    let direction = normalize(far.xyz / far.w - near.xyz / near.w);
    let blur = clamp(frame.environment.y, 0.0, 1.0);
    let d = to_environment(direction, frame.environment.x);
    var radiance: vec3<f32>;
    if blur <= 0.0 {
        radiance = textureSampleLevel(source_cube, environment_sampler, d, 0.0).rgb;
    } else {
        // The GGX-prefiltered cube blurs smoothly, unlike box-filtered mips.
        radiance = textureSampleLevel(specular_cube, environment_sampler, d, blur * (environment.mips.x - 1.0)).rgb;
    }
    return vec4<f32>(radiance, 1.0);
}
