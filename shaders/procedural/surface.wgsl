// Surface of a texture set at a texel: the closest point, on the triangle the surface map
// gives for the texel, to the texel center in UV space; then position, normal, tangent and
// UV interpolated there. Texels outside the UV islands thus extend the surface of their
// nearest triangle. Must match `patine_procedural::reference::surface`.

// #import procedural/mesh

@group(1) @binding(0) var<storage, read> mesh_vertices: array<Vertex>;
@group(1) @binding(1) var<storage, read> mesh_indices: array<u32>;
@group(1) @binding(2) var surface_map: texture_2d<u32>;

struct Surface {
    position: vec3<f32>,
    normal: vec3<f32>,
    // xyz: unit tangent, w: bitangent sign.
    tangent: vec4<f32>,
    uv: vec2<f32>,
}

// Barycentric coordinates of the point of triangle (a, b, c) closest to `p` (Ericson,
// "Real-Time Collision Detection", 5.1.5), in 2D.
fn closest_barycentric(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, c: vec2<f32>) -> vec3<f32> {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return vec3<f32>(1.0, 0.0, 0.0);
    }
    let bp = p - b;
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return vec3<f32>(0.0, 1.0, 0.0);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return vec3<f32>(1.0 - v, v, 0.0);
    }
    let cp = p - c;
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return vec3<f32>(0.0, 0.0, 1.0);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return vec3<f32>(1.0 - w, 0.0, w);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return vec3<f32>(0.0, 1.0 - w, w);
    }
    let sum = va + vb + vc;
    if sum <= 1e-20 {
        // Degenerate triangle in UV space: its first corner.
        return vec3<f32>(1.0, 0.0, 0.0);
    }
    let v = vb / sum;
    let w = vc / sum;
    return vec3<f32>(1.0 - v - w, v, w);
}

fn normalize_or(v: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let l = length(v);
    if l > 1e-12 {
        return v / l;
    }
    return fallback;
}

// The surface at texel `texel` of a `size` × `size` map; `None`-like (normal 0) when the
// texture set has no triangle.
fn surface_at(texel: vec2<u32>, size: f32) -> Surface {
    var s: Surface;
    let entry = textureLoad(surface_map, texel, 0).r;
    if entry == 0u {
        s.tangent = vec4<f32>(1.0, 0.0, 0.0, 1.0);
        return s;
    }
    let base = (entry - 1u) * 3u;
    let a = mesh_vertices[mesh_indices[base]];
    let b = mesh_vertices[mesh_indices[base + 1u]];
    let c = mesh_vertices[mesh_indices[base + 2u]];
    let p = (vec2<f32>(texel) + 0.5) / size;
    let w = closest_barycentric(p, a.uv.xy, b.uv.xy, c.uv.xy);
    s.position = a.position.xyz * w.x + b.position.xyz * w.y + c.position.xyz * w.z;
    s.normal = normalize_or(a.normal.xyz * w.x + b.normal.xyz * w.y + c.normal.xyz * w.z, vec3<f32>(0.0, 0.0, 1.0));
    let t = normalize_or(a.tangent.xyz * w.x + b.tangent.xyz * w.y + c.tangent.xyz * w.z, vec3<f32>(1.0, 0.0, 0.0));
    s.tangent = vec4<f32>(t, select(-1.0, 1.0, a.tangent.w >= 0.0));
    s.uv = a.uv.xy * w.x + b.uv.xy * w.y + c.uv.xy * w.z;
    return s;
}
