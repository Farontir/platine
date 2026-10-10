// Cube face directions (faces +X, -X, +Y, -Y, +Z, -Z, rows going down) and the
// equirectangular mapping (u from -Z towards +X, v from +Y down).
// Mirrors crates/patine_render/src/reference.rs and environment.rs.

// #import render/math

// Direction of face `face` at coordinates s, t in [-1, 1] (t down).
fn cube_direction(face: u32, s: f32, t: f32) -> vec3<f32> {
    var d: vec3<f32>;
    switch face {
        case 0u: { d = vec3<f32>(1.0, -t, -s); }
        case 1u: { d = vec3<f32>(-1.0, -t, s); }
        case 2u: { d = vec3<f32>(s, 1.0, t); }
        case 3u: { d = vec3<f32>(s, -1.0, -t); }
        case 4u: { d = vec3<f32>(s, -t, 1.0); }
        default: { d = vec3<f32>(-s, -t, -1.0); }
    }
    return normalize(d);
}

// Direction through the center of texel `texel` of a size² face.
fn cube_texel_direction(face: u32, texel: vec2<u32>, size: u32) -> vec3<f32> {
    let st = (vec2<f32>(texel) + 0.5) / f32(size) * 2.0 - 1.0;
    return cube_direction(face, st.x, st.y);
}

fn direction_to_equirect(d: vec3<f32>) -> vec2<f32> {
    let u = atan2(d.x, -d.z) / TAU + 0.5;
    let v = acos(clamp(d.y, -1.0, 1.0)) / PI;
    return vec2<f32>(u, v);
}
