// Shared part of the procedural passes: the evaluation uniforms (projection, output), the
// surface of the current texel, and its projection into pattern space. An entry shader
// (`procedural/noises/*`, `patterns/*`, `materials/*`) declares its `Params` at
// @group(0) @binding(1) and blends its function over the planes of `project`.
// Must match `EvaluationUniforms` in crates/patine_procedural/src/evaluate.rs and
// `patine_procedural::reference::projection`.

// #import procedural/surface
// #import render/fullscreen

struct Evaluation {
    // 0 UV, 1 triplanar, 2 planar, 3 spherical, 4 cylindrical.
    kind: u32,
    // Triplanar blend, 0 (sharp) to 1 (smooth).
    blend: f32,
    // 1: the surface is the unit square (thumbnails), not the mesh.
    flat: u32,
    // Output: channel index for materials (see `Output`), unused for gray procedurals.
    output: u32,
    scale: vec2<f32>,
    offset: vec2<f32>,
    // cos and sin of the rotation.
    rotation: vec2<f32>,
    // Integer repetitions when the projection tiles (UV, spherical, cylindrical with an
    // integer scale), (0, 0) otherwise.
    tile: vec2<i32>,
    // xyz: center of the mesh bounds, w: their largest extent.
    center: vec4<f32>,
    // Side of the target, in texels.
    size: f32,
    padding0: f32,
    padding1: f32,
    padding2: f32,
}

@group(0) @binding(0) var<uniform> evaluation: Evaluation;

const PI: f32 = 3.14159265;

// Up to three pattern-space coordinates and their weights.
struct Projected {
    q0: vec2<f32>,
    q1: vec2<f32>,
    q2: vec2<f32>,
    w: vec3<f32>,
}

// The surface at the current texel (the unit square for thumbnails).
fn current_surface(clip: vec4<f32>) -> Surface {
    let texel = vec2<u32>(clip.xy);
    if evaluation.flat == 1u {
        var s: Surface;
        s.uv = (vec2<f32>(texel) + 0.5) / evaluation.size;
        s.position = vec3<f32>(s.uv, 0.0);
        s.normal = vec3<f32>(0.0, 0.0, 1.0);
        s.tangent = vec4<f32>(1.0, 0.0, 0.0, 1.0);
        return s;
    }
    return surface_at(texel, evaluation.size);
}

// Scale, rotation, then offset.
fn place(x: vec2<f32>) -> vec2<f32> {
    let s = x * evaluation.scale;
    let r = evaluation.rotation;
    return vec2<f32>(r.x * s.x - r.y * s.y, r.y * s.x + r.x * s.y) + evaluation.offset;
}

// Triplanar weights of a normal: |n| minus a threshold that sharpens the transitions.
fn triplanar_weights(n: vec3<f32>) -> vec3<f32> {
    let w = max(abs(n) - 0.57 * (1.0 - evaluation.blend), vec3<f32>(0.0));
    return w / max(w.x + w.y + w.z, 1e-6);
}

fn project(s: Surface) -> Projected {
    var out: Projected;
    out.w = vec3<f32>(1.0, 0.0, 0.0);
    // Position relative to the mesh bounds, in [0, 1] along their largest extent.
    let pn = (s.position - evaluation.center.xyz) / evaluation.center.w + 0.5;
    switch evaluation.kind {
        case 1u: {
            out.q0 = place(vec2<f32>(pn.z, 1.0 - pn.y));
            out.q1 = place(vec2<f32>(pn.x, pn.z));
            out.q2 = place(vec2<f32>(pn.x, 1.0 - pn.y));
            out.w = triplanar_weights(s.normal);
        }
        case 2u: {
            out.q0 = place(vec2<f32>(pn.x, 1.0 - pn.y));
        }
        case 3u: {
            let d = normalize_or(s.position - evaluation.center.xyz, vec3<f32>(0.0, 1.0, 0.0));
            let u = atan2(d.z, d.x) / (2.0 * PI) + 0.5;
            let v = acos(clamp(d.y, -1.0, 1.0)) / PI;
            out.q0 = place(vec2<f32>(u, v));
        }
        case 4u: {
            let d = s.position - evaluation.center.xyz;
            let u = atan2(d.z, d.x) / (2.0 * PI) + 0.5;
            out.q0 = place(vec2<f32>(u, 0.5 - d.y / evaluation.center.w));
        }
        default: {
            out.q0 = place(s.uv);
        }
    }
    return out;
}

// Period of a noise of `cells` cells per projection unit: tiling when the projection
// tiles, otherwise too long to show.
fn noise_period(cells: i32) -> vec2<i32> {
    if evaluation.tile.x > 0 {
        return evaluation.tile * cells;
    }
    return vec2<i32>(1 << 20);
}
