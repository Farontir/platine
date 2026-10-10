// Environment lighting resources (bind group 1) and lookups.
// Must match `EnvironmentUniforms` in crates/patine_render/src/viewport.rs.

struct Environment {
    // Diffuse harmonics (outgoing radiance of a white Lambertian surface), unrotated.
    sh: array<vec4<f32>, 9>,
    // The same without the dominant light, used when shadows are on.
    residual: array<vec4<f32>, 9>,
    // xyz: direction towards the dominant light, rotated with the environment.
    light_direction: vec4<f32>,
    // rgb: white Lambertian radiance of the light at normal incidence.
    light_color: vec4<f32>,
    // x: specular mip count, y: source cube mip count.
    mips: vec4<f32>,
}

@group(1) @binding(0) var<uniform> environment: Environment;
@group(1) @binding(1) var specular_cube: texture_cube<f32>;
@group(1) @binding(2) var source_cube: texture_cube<f32>;
@group(1) @binding(3) var brdf_lut: texture_2d<f32>;
@group(1) @binding(4) var environment_sampler: sampler;
@group(1) @binding(5) var shadow_map: texture_depth_2d;
@group(1) @binding(6) var shadow_sampler: sampler_comparison;

// World direction to environment direction: the environment is rotated by `angle`
// around +Y, so lookups use the inverse rotation.
fn to_environment(d: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3<f32>(c * d.x - s * d.z, d.y, s * d.x + c * d.z);
}

// Order-2 SH basis, same order and constants as environment::sh_basis.
fn sh_diffuse(coefficients: array<vec4<f32>, 9>, d: vec3<f32>) -> vec3<f32> {
    var result = coefficients[0].rgb * 0.282095;
    result += coefficients[1].rgb * (0.488603 * d.y);
    result += coefficients[2].rgb * (0.488603 * d.z);
    result += coefficients[3].rgb * (0.488603 * d.x);
    result += coefficients[4].rgb * (1.092548 * d.x * d.y);
    result += coefficients[5].rgb * (1.092548 * d.y * d.z);
    result += coefficients[6].rgb * (0.315392 * (3.0 * d.z * d.z - 1.0));
    result += coefficients[7].rgb * (1.092548 * d.x * d.z);
    result += coefficients[8].rgb * (0.546274 * (d.x * d.x - d.y * d.y));
    return max(result, vec3<f32>(0.0));
}
