// Metal/roughness shading: Cook-Torrance (GGX, height-correlated Smith, Schlick) lit by
// the environment. Diffuse from spherical harmonics, specular from the prefiltered cube
// and the split-sum LUT. With shadows on, the dominant light is taken out of the
// harmonics and added back as a shadowed directional light (diffuse only: the
// environment's specular reflection is not shadowed).

// #import render/frame
// #import render/brdf
// #import render/environment

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
    // Not renormalized after interpolation (mikktspace rule).
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

// Fraction of the dominant light reaching `world_position`: 3 × 3 PCF in the shadow
// map. The point is offset along the normal by a few shadow texels to avoid acne.
fn shadow_factor(world_position: vec3<f32>, n: vec3<f32>) -> f32 {
    let texel_world = frame.environment.w;
    let p = world_position + n * (1.5 * texel_world);
    let clip = frame.light_view_proj * vec4<f32>(p, 1.0);
    let ndc = clip.xyz / clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || ndc.z > 1.0 {
        return 1.0;
    }
    let texel = 1.0 / vec2<f32>(textureDimensions(shadow_map));
    var lit = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel;
            lit += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + offset, ndc.z);
        }
    }
    return lit / 9.0;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let v = normalize(frame.camera_position.xyz - in.world_position);
    let base_color = frame.base_color.rgb;
    let metallic = clamp(frame.material.x, 0.0, 1.0);
    let roughness = clamp(frame.material.y, 0.02, 1.0);
    let rotation = frame.environment.x;
    let shadows = frame.environment.z > 0.5;

    let f0 = mix(vec3<f32>(0.04), base_color, metallic);
    let diffuse_color = base_color * (1.0 - metallic);
    let n_dot_v = clamp(dot(n, v), 1e-4, 1.0);

    // Split-sum specular: prefiltered radiance × (F0·A + B).
    let lut = textureSampleLevel(brdf_lut, environment_sampler, vec2<f32>(n_dot_v, roughness), 0.0).rg;
    let specular_albedo = f0 * lut.x + lut.y;
    let r = reflect(-v, n);
    let lod = roughness * (environment.mips.x - 1.0);
    let prefiltered = textureSampleLevel(specular_cube, environment_sampler, to_environment(r, rotation), lod).rgb;
    let specular = prefiltered * specular_albedo;

    // Diffuse: energy not reflected specularly.
    let diffuse_weight = diffuse_color * (vec3<f32>(1.0) - specular_albedo);
    var irradiance: vec3<f32>;
    if shadows {
        let l = environment.light_direction.xyz;
        let n_dot_l = max(dot(n, l), 0.0);
        var shadow = 1.0;
        if n_dot_l > 0.0 {
            shadow = shadow_factor(in.world_position, n);
        }
        let direct = environment.light_color.rgb * n_dot_l * shadow;
        irradiance = sh_diffuse(environment.residual, to_environment(n, rotation)) + direct;
    } else {
        irradiance = sh_diffuse(environment.sh, to_environment(n, rotation));
    }
    // Ambient occlusion would scale the indirect terms only (channel added in M4).
    let color = diffuse_weight * irradiance + specular;
    return vec4<f32>(color, 1.0);
}
