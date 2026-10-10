// Constants and sampling helpers. Mirrors crates/patine_render/src/reference.rs.

const PI: f32 = 3.14159265358979;
const TAU: f32 = 6.28318530717959;

// i-th point of the n-point Hammersley set.
fn hammersley(i: u32, n: u32) -> vec2<f32> {
    return vec2<f32>(f32(i) / f32(n), f32(reverseBits(i)) * 2.3283064365386963e-10);
}

// Half vector sampled proportionally to D(h)·(n·h) around n, alpha = roughness².
fn importance_sample_ggx(xi: vec2<f32>, n: vec3<f32>, alpha: f32) -> vec3<f32> {
    let a2 = alpha * alpha;
    let phi = TAU * xi.x;
    let cos_theta = sqrt((1.0 - xi.y) / (1.0 + (a2 - 1.0) * xi.y));
    let sin_theta = sqrt(max(1.0 - cos_theta * cos_theta, 0.0));
    let local = vec3<f32>(sin_theta * cos(phi), sin_theta * sin(phi), cos_theta);
    let up = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(n.z) < 0.999);
    let tangent = normalize(cross(up, n));
    let bitangent = cross(n, tangent);
    return normalize(tangent * local.x + bitangent * local.y + n * local.z);
}
