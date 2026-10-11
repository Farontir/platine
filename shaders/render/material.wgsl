// Material of the mesh (bind group 2): channel textures and their factors.
// Must match `MaterialUniforms` and `MaterialResources` in
// crates/patine_render/src/material.rs.

struct Material {
    // rgb: base color factor.
    base_color: vec4<f32>,
    // x: metallic, y: roughness, z: occlusion strength, w: emissive factor.
    factors: vec4<f32>,
    // x: inspected channel kind (0 color, 1 scalar, 2 normal); y: 1 when it exists.
    inspect: vec4<f32>,
}

@group(2) @binding(0) var<uniform> material: Material;
@group(2) @binding(1) var base_color_texture: texture_2d<f32>;
@group(2) @binding(2) var metallic_texture: texture_2d<f32>;
@group(2) @binding(3) var roughness_texture: texture_2d<f32>;
@group(2) @binding(4) var normal_texture: texture_2d<f32>;
@group(2) @binding(5) var occlusion_texture: texture_2d<f32>;
@group(2) @binding(6) var emissive_texture: texture_2d<f32>;
@group(2) @binding(7) var inspect_texture: texture_2d<f32>;
@group(2) @binding(8) var material_sampler: sampler;

// Surface values of the material at `uv`.
struct Surface {
    base_color: vec3<f32>,
    metallic: f32,
    roughness: f32,
    // Tangent-space normal.
    normal: vec3<f32>,
    occlusion: f32,
    emissive: vec3<f32>,
}

fn sample_surface(uv: vec2<f32>) -> Surface {
    var s: Surface;
    s.base_color = textureSample(base_color_texture, material_sampler, uv).rgb * material.base_color.rgb;
    s.metallic = clamp(textureSample(metallic_texture, material_sampler, uv).r * material.factors.x, 0.0, 1.0);
    s.roughness = clamp(textureSample(roughness_texture, material_sampler, uv).r * material.factors.y, 0.02, 1.0);
    s.normal = textureSample(normal_texture, material_sampler, uv).xyz;
    s.occlusion = mix(1.0, textureSample(occlusion_texture, material_sampler, uv).r, material.factors.z);
    s.emissive = textureSample(emissive_texture, material_sampler, uv).rgb * material.factors.w;
    return s;
}

// World normal from a tangent-space normal and the interpolated mikktspace basis: the
// bitangent is sign × cross(normal, tangent), with the interpolated vectors used as they
// are (not renormalized) before the final normalization.
fn perturb_normal(tangent_space: vec3<f32>, normal: vec3<f32>, tangent: vec4<f32>) -> vec3<f32> {
    let bitangent = tangent.w * cross(normal, tangent.xyz);
    return normalize(tangent_space.x * tangent.xyz + tangent_space.y * bitangent + tangent_space.z * normal);
}
