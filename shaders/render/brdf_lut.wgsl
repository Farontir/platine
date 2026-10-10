// Split-sum BRDF lookup table: x = n·v, y = perceptual roughness, rg = (A, B) such that
// the specular integral is F0·A + B. Same algorithm as reference::brdf_lut.

// #import render/brdf

struct Params {
    samples: u32,
    row_offset: u32,
}

@group(0) @binding(0) var output: texture_storage_2d<rgba16float, write>;
@group(0) @binding(1) var<uniform> params: Params;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output);
    let texel = vec2<u32>(id.x, id.y + params.row_offset);
    if texel.x >= size.x || texel.y >= size.y {
        return;
    }
    let n_dot_v = (f32(texel.x) + 0.5) / f32(size.x);
    let roughness = (f32(texel.y) + 0.5) / f32(size.y);
    let alpha = roughness * roughness;
    let v = vec3<f32>(sqrt(max(1.0 - n_dot_v * n_dot_v, 0.0)), 0.0, n_dot_v);
    var result = vec2<f32>(0.0);
    for (var i = 0u; i < params.samples; i++) {
        let h = importance_sample_ggx(hammersley(i, params.samples), vec3<f32>(0.0, 0.0, 1.0), alpha);
        let l = 2.0 * dot(v, h) * h - v;
        let n_dot_l = l.z;
        let n_dot_h = max(h.z, 0.0);
        let v_dot_h = max(dot(v, h), 0.0);
        if n_dot_l > 0.0 {
            let visibility = smith_visibility(n_dot_v, n_dot_l, alpha);
            let weight = visibility * 4.0 * v_dot_h * n_dot_l / max(n_dot_h, 1e-7);
            let fc = pow(1.0 - v_dot_h, 5.0);
            result += vec2<f32>((1.0 - fc) * weight, fc * weight);
        }
    }
    textureStore(output, vec2<i32>(texel), vec4<f32>(result / f32(params.samples), 0.0, 1.0));
}
