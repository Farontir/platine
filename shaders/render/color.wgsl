// Color space helpers.

// Linear to sRGB transfer function (IEC 61966-2-1), per channel, input clamped to [0, 1].
fn srgb_encode(linear: vec3<f32>) -> vec3<f32> {
    let c = clamp(linear, vec3<f32>(0.0), vec3<f32>(1.0));
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}

// sRGB to linear transfer function, the inverse of srgb_encode on [0, 1].
fn srgb_decode(encoded: vec3<f32>) -> vec3<f32> {
    let c = clamp(encoded, vec3<f32>(0.0), vec3<f32>(1.0));
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}
