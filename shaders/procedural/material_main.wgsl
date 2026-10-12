// Entry point of the procedural materials, imported by each material shader
// (`procedural/materials/*`). The importer defines:
// - `fn material(q: vec2<f32>) -> MaterialSample`: the material at pattern-space `q`;
// - `fn material_height(q: vec2<f32>) -> f32`: its height alone (for the normal).
// This file is therefore not valid on its own; the shader test validates it through
// every importer. Must match `patine_procedural::reference::material`.
//
// The pass renders every output at once, to five targets: 0 base color, 1 metallic,
// 2 roughness, 3 normal (tangent space), 4 height.

// #import procedural/evaluate

struct MaterialSample {
    base_color: vec3<f32>,
    metallic: f32,
    roughness: f32,
    height: f32,
}

// Slope of the normal per unit of height gradient (in UV units).
const RELIEF: f32 = 0.02;

fn weighted(a: MaterialSample, w: f32) -> MaterialSample {
    var out: MaterialSample;
    out.base_color = a.base_color * w;
    out.metallic = a.metallic * w;
    out.roughness = a.roughness * w;
    out.height = a.height * w;
    return out;
}

fn added(a: MaterialSample, b: MaterialSample) -> MaterialSample {
    var out: MaterialSample;
    out.base_color = a.base_color + b.base_color;
    out.metallic = a.metallic + b.metallic;
    out.roughness = a.roughness + b.roughness;
    out.height = a.height + b.height;
    return out;
}

// The material blended over the planes of the projection.
fn blended_material(p: Projected) -> MaterialSample {
    var m = weighted(material(p.q0), p.w.x);
    if p.w.y > 0.0 {
        m = added(m, weighted(material(p.q1), p.w.y));
    }
    if p.w.z > 0.0 {
        m = added(m, weighted(material(p.q2), p.w.z));
    }
    return m;
}

// The height blended over the planes of the projection.
fn blended_height(p: Projected) -> f32 {
    var h = p.w.x * material_height(p.q0);
    if p.w.y > 0.0 {
        h += p.w.y * material_height(p.q1);
    }
    if p.w.z > 0.0 {
        h += p.w.z * material_height(p.q2);
    }
    return h;
}

// Height at surface `s` moved by `du` in UV (UV projection) or `dp` in space (others).
fn height_near(s: Surface, du: vec2<f32>, dp: vec3<f32>) -> f32 {
    var moved = s;
    moved.uv = s.uv + du;
    moved.position = s.position + dp;
    return blended_height(project(moved));
}

// Tangent-space normal from the height gradient, by forward differences over half a texel
// from the height `h0` at `s`: along U and V for the UV projection, along the tangent and
// bitangent (scaled to UV-like units by the mesh extent) for the others.
fn material_normal(s: Surface, h0: f32) -> vec3<f32> {
    let e = 0.5 / evaluation.size;
    var g: vec2<f32>;
    if evaluation.kind == 0u || evaluation.flat == 1u {
        g = vec2<f32>(
            height_near(s, vec2<f32>(e, 0.0), vec3<f32>(0.0)) - h0,
            height_near(s, vec2<f32>(0.0, e), vec3<f32>(0.0)) - h0,
        ) / e;
    } else {
        let n = s.normal;
        let t = normalize_or(s.tangent.xyz - n * dot(n, s.tangent.xyz), vec3<f32>(1.0, 0.0, 0.0));
        let b = cross(n, t) * s.tangent.w;
        let step = e * evaluation.center.w;
        g = vec2<f32>(
            height_near(s, vec2<f32>(0.0), t * step) - h0,
            height_near(s, vec2<f32>(0.0), b * step) - h0,
        ) / e;
    }
    return normalize(vec3<f32>(-g * RELIEF, 1.0));
}

struct MaterialOutput {
    @location(0) base_color: vec4<f32>,
    @location(1) metallic: vec4<f32>,
    @location(2) roughness: vec4<f32>,
    @location(3) normal: vec4<f32>,
    @location(4) height: vec4<f32>,
}

@fragment
fn fs_main(in: FullscreenOutput) -> MaterialOutput {
    let s = current_surface(in.clip);
    let m = blended_material(project(s));
    var out: MaterialOutput;
    out.base_color = vec4<f32>(m.base_color, 1.0);
    out.metallic = vec4<f32>(m.metallic, 0.0, 0.0, 0.0);
    out.roughness = vec4<f32>(m.roughness, 0.0, 0.0, 0.0);
    out.normal = vec4<f32>(material_normal(s, m.height), 0.0);
    out.height = vec4<f32>(m.height, 0.0, 0.0, 0.0);
    return out;
}
