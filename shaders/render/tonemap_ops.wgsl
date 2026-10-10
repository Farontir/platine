// Display operators on linear Rec.709 radiance, returning linear display values in
// [0, 1] (sRGB encoding comes after). Mirrors reference::tone_map.

// AgX base look: inset matrix, log2 encoding over [-12.47, 4.03] EV, polynomial
// sigmoid, outset matrix, display gamma 2.2 (approximation of Troy Sobotka's AgX).
fn agx(color: vec3<f32>) -> vec3<f32> {
    let inset = mat3x3<f32>(
        0.842479062253094, 0.0423282422610123, 0.0423756549057051,
        0.0784335999999992, 0.878468636469772, 0.0784336,
        0.0792237451477643, 0.0791661274605434, 0.879142973793104,
    );
    let outset = mat3x3<f32>(
        1.19687900512017, -0.0528968517574562, -0.0529716355144438,
        -0.0980208811401368, 1.15190312990417, -0.0980434501171241,
        -0.0990297440797205, -0.0989611768448433, 1.15107367264116,
    );
    let min_ev = -12.47393;
    let max_ev = 4.026069;
    var v = inset * max(color, vec3<f32>(1e-10));
    v = (clamp(log2(v), vec3<f32>(min_ev), vec3<f32>(max_ev)) - min_ev) / (max_ev - min_ev);
    let v2 = v * v;
    let v4 = v2 * v2;
    v = 15.5 * v4 * v2 - 40.14 * v4 * v + 31.96 * v4 - 6.868 * v2 * v + 0.4298 * v2 + 0.1191 * v - 0.00232;
    v = outset * v;
    return pow(clamp(v, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
}

// ACES filmic, Stephen Hill's fit of the RRT and ODT.
fn aces(color: vec3<f32>) -> vec3<f32> {
    let input = mat3x3<f32>(
        0.59719, 0.07600, 0.02840,
        0.35458, 0.90834, 0.13383,
        0.04823, 0.01566, 0.83777,
    );
    let output = mat3x3<f32>(
        1.60475, -0.10208, -0.00327,
        -0.53108, 1.10813, -0.07276,
        -0.07367, -0.00605, 1.07602,
    );
    let v = input * color;
    let a = v * (v + 0.0245786) - 0.000090537;
    let b = v * (0.983729 * v + 0.4329510) + 0.238081;
    return clamp(output * (a / b), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Khronos PBR Neutral.
fn pbr_neutral(input: vec3<f32>) -> vec3<f32> {
    let start_compression = 0.8 - 0.04;
    let desaturation = 0.15;
    var color = max(input, vec3<f32>(0.0));
    let x = min(color.r, min(color.g, color.b));
    let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
    color -= vec3<f32>(offset);
    let peak = max(color.r, max(color.g, color.b));
    if peak < start_compression {
        return clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    let d = 1.0 - start_compression;
    let new_peak = 1.0 - d * d / (peak + d - start_compression);
    color *= new_peak / peak;
    let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return clamp(mix(color, vec3<f32>(new_peak), g), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Operator `index`: 0 AgX, 1 ACES, 2 PBR Neutral, 3 linear (clamp).
fn tone_map(color: vec3<f32>, index: u32) -> vec3<f32> {
    switch index {
        case 0u: { return agx(color); }
        case 1u: { return aces(color); }
        case 2u: { return pbr_neutral(color); }
        default: { return clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)); }
    }
}
