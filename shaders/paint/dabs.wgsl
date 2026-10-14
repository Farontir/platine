// `rasterize_dabs`: accumulates a batch of dabs into the stroke buffer (R32Float, read
// and written in place), on a list of tiles. Every texel finds its surface point (from
// the surface map), projects it with the stroke's camera, checks that the camera sees it,
// then adds the dabs in order: s = s + (1 − s) × flow × coverage × angle.
// Must match `patine_paint::reference` and `DabUniforms` / `GpuDab` in
// crates/patine_paint/src/raster.rs and projection.rs.

// #import procedural/surface

const TILE: u32 = 128u;
const W_MIN: f32 = 1e-6;

struct DabUniforms {
    view_proj: mat4x4<f32>,
    depth_axis: vec4<f32>,
    // xyz: view direction, away from the camera.
    forward: vec4<f32>,
    viewport: vec2<f32>,
    // Side of the texture set's textures.
    size: u32,
    dab_count: u32,
    // Largest extent of the mesh (scales the depth bias).
    extent: f32,
    has_alpha: u32,
    dilation: f32,
    depth_bias: f32,
    grazing: vec2<f32>,
    padding: vec2<f32>,
}

struct Dab {
    center: vec2<f32>,
    radius: f32,
    flow: f32,
    // Cosine and sine of the rotation.
    rotation: vec2<f32>,
    hardness: f32,
    // Most paint the stroke may hold where this dab lands.
    opacity: f32,
}

@group(0) @binding(0) var<uniform> u: DabUniforms;
@group(0) @binding(1) var<storage, read> dabs: array<Dab>;
@group(0) @binding(2) var<storage, read> tiles: array<vec2<u32>>;
@group(0) @binding(3) var depth_image: texture_2d<f32>;
@group(0) @binding(4) var alpha_image: texture_2d<f32>;
@group(0) @binding(5) var stroke: texture_storage_2d<r32float, read_write>;

fn row(i: u32) -> vec4<f32> {
    return vec4<f32>(u.view_proj[0][i], u.view_proj[1][i], u.view_proj[2][i], u.view_proj[3][i]);
}

fn depth_at(x: i32, y: i32) -> f32 {
    let size = vec2<i32>(textureDimensions(depth_image));
    let p = clamp(vec2<i32>(x, y), vec2<i32>(0), size - vec2<i32>(1));
    return textureLoad(depth_image, p, 0).r;
}

// The farthest depth among the four pixels around `pixel`.
fn depth_around(pixel: vec2<f32>) -> f32 {
    let p = pixel - 0.5;
    let x = i32(floor(p.x));
    let y = i32(floor(p.y));
    return max(max(depth_at(x, y), depth_at(x + 1, y)), max(depth_at(x, y + 1), depth_at(x + 1, y + 1)));
}

// Unit direction toward the camera from a point at clip position `clip`.
fn toward_camera(clip: vec4<f32>) -> vec3<f32> {
    let ndc = clip.xy / clip.w;
    let a = (row(0u) - row(3u) * ndc.x).xyz;
    let b = (row(1u) - row(3u) * ndc.y).xyz;
    let d = cross(a, b);
    let l = length(d);
    if l <= 1e-20 {
        return -normalize(u.forward.xyz);
    }
    let n = d / l;
    if dot(n, u.forward.xyz) > 0.0 {
        return -n;
    }
    return n;
}

fn alpha_texel(x: i32, y: i32) -> f32 {
    let size = vec2<i32>(textureDimensions(alpha_image));
    if x < 0 || y < 0 || x >= size.x || y >= size.y {
        return 0.0;
    }
    return textureLoad(alpha_image, vec2<i32>(x, y), 0).r;
}

// Bilinear between pixel centers, 0 outside.
fn sample_alpha(uv: vec2<f32>) -> f32 {
    let size = vec2<f32>(textureDimensions(alpha_image));
    let p = vec2<f32>(uv.x * size.x - 0.5, uv.y * size.y - 0.5);
    let base = floor(p);
    let f = p - base;
    let x = i32(base.x);
    let y = i32(base.y);
    let top = alpha_texel(x, y) * (1.0 - f.x) + alpha_texel(x + 1, y) * f.x;
    let bottom = alpha_texel(x, y + 1) * (1.0 - f.x) + alpha_texel(x + 1, y + 1) * f.x;
    return top * (1.0 - f.y) + bottom * f.y;
}

fn dab_coverage(dab: Dab, pixel: vec2<f32>) -> f32 {
    let d = pixel - dab.center;
    let c = dab.rotation.x;
    let s = dab.rotation.y;
    let local = vec2<f32>(d.x * c + d.y * s, -d.x * s + d.y * c) / dab.radius;
    if u.has_alpha != 0u {
        if abs(local.x) > 1.0 || abs(local.y) > 1.0 {
            return 0.0;
        }
        return sample_alpha(local * 0.5 + 0.5);
    }
    let distance = length(local) * dab.radius;
    let soft = max((1.0 - dab.hardness) * dab.radius, 1.0);
    return smoothstep(0.0, 1.0, (dab.radius - distance) / soft);
}

@compute @workgroup_size(8, 8)
fn cs_main(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_id) local: vec3<u32>) {
    let texel = tiles[group.z] * TILE + group.xy * 8u + local.xy;
    if texel.x >= u.size || texel.y >= u.size {
        return;
    }
    let size = f32(u.size);
    let surface = surface_at(texel, size);
    if all(surface.normal == vec3<f32>(0.0)) {
        return;
    }
    let p = (vec2<f32>(texel) + 0.5) / size;
    if length(p - surface.uv) * size > u.dilation {
        return;
    }
    let clip = u.view_proj * vec4<f32>(surface.position, 1.0);
    if clip.w <= W_MIN {
        return;
    }
    let ndc = clip.xy / clip.w;
    let pixel = vec2<f32>((ndc.x * 0.5 + 0.5) * u.viewport.x, (0.5 - ndc.y * 0.5) * u.viewport.y);
    let facing = dot(surface.normal, toward_camera(clip));
    let angle = smoothstep(u.grazing.x, u.grazing.y, facing);
    if angle <= 0.0 {
        return;
    }
    if dot(u.depth_axis, clip) > depth_around(pixel) + u.depth_bias * u.extent {
        return;
    }
    var s = textureLoad(stroke, texel).r;
    for (var i = 0u; i < u.dab_count; i++) {
        let dab = dabs[i];
        s = s + (1.0 - s) * (dab.flow * dab_coverage(dab, pixel) * angle);
    }
    textureStore(stroke, texel, vec4<f32>(s, 0.0, 0.0, 0.0));
}
