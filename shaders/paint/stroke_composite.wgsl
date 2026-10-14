// Composes a stroke onto a paint content texture, on the tiles of the scissor, with
// a = stroke buffer × opacity. To deposit, the blend state src + dst × (1 − src.a) and
// the output (value × a, a); to erase, the blend state dst × (1 − src.a) and an output
// whose alpha is a.
// Content layouts: vector (Rgba16Float: premultiplied rgb, coverage in a) or scalar
// (Rg16Float: premultiplied value, coverage in g; blending reads the output's alpha).
// Must match `CompositeUniforms` in crates/patine_paint/src/raster.rs.

// #import render/fullscreen

struct CompositeUniforms {
    value: vec4<f32>,
    opacity: f32,
    // 1 for the eraser.
    eraser: u32,
    // 1 for the scalar layout, 0 for the vector one.
    scalar: u32,
    padding: f32,
}

@group(0) @binding(0) var<uniform> u: CompositeUniforms;
@group(0) @binding(1) var stroke: texture_2d<f32>;

@fragment
fn fs_main(in: FullscreenOutput) -> @location(0) vec4<f32> {
    let a = textureLoad(stroke, vec2<u32>(in.clip.xy), 0).r * u.opacity;
    if u.eraser != 0u {
        return vec4<f32>(0.0, 0.0, 0.0, a);
    }
    if u.scalar != 0u {
        return vec4<f32>(u.value.r * a, a, 0.0, a);
    }
    return vec4<f32>(u.value.rgb * a, a);
}
