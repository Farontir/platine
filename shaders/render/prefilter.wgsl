// GGX-prefiltered specular radiance for one mip level (n = v = r), by importance
// sampling with mip-filtered lookups (Karis 2013, Křivánek 2008). Converges to
// reference::prefilter_brute_force.

// #import render/cube
// #import render/brdf

struct Params {
    roughness: f32,
    samples: u32,
    // Face size of mip 0 of the source cube.
    source_size: f32,
    // First face row of this dispatch (tiling against driver timeouts).
    row_offset: u32,
}

@group(0) @binding(0) var source: texture_cube<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var output: texture_storage_2d_array<rgba16float, write>;
@group(0) @binding(3) var<uniform> params: Params;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output).x;
    let texel = vec2<u32>(id.x, id.y + params.row_offset);
    if texel.x >= size || texel.y >= size {
        return;
    }
    let n = cube_texel_direction(id.z, texel, size);
    var color: vec3<f32>;
    if params.roughness <= 0.0 {
        // Mirror: the source at the resolution of this level.
        let lod = max(log2(params.source_size / f32(size)), 0.0);
        color = textureSampleLevel(source, source_sampler, n, lod).rgb;
    } else {
        let alpha = params.roughness * params.roughness;
        // Solid angle of a source texel at mip 0.
        let texel_solid_angle = 4.0 * PI / (6.0 * params.source_size * params.source_size);
        var sum = vec3<f32>(0.0);
        var weight = 0.0;
        for (var i = 0u; i < params.samples; i++) {
            let h = importance_sample_ggx(hammersley(i, params.samples), n, alpha);
            let n_dot_h = dot(n, h);
            let l = 2.0 * n_dot_h * h - n;
            let n_dot_l = dot(n, l);
            if n_dot_l > 0.0 {
                // pdf(l) = D·(n·h) / (4 v·h) with v = n: D / 4.
                let pdf = ggx_distribution(n_dot_h, alpha) * 0.25;
                let sample_solid_angle = 1.0 / (f32(params.samples) * pdf + 1e-7);
                let lod = max(0.5 * log2(sample_solid_angle / texel_solid_angle) + 1.0, 0.0);
                sum += textureSampleLevel(source, source_sampler, l, lod).rgb * n_dot_l;
                weight += n_dot_l;
            }
        }
        color = sum / max(weight, 1e-7);
    }
    textureStore(output, vec2<i32>(texel), i32(id.z), vec4<f32>(color, 1.0));
}
