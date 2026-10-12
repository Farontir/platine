// Jump flooding of the surface map: every texel finds the nearest covered texel, and
// takes its triangle. Mode 0 seeds the covered texels, mode 1 is one step of `step`
// texels, mode 2 resolves seeds into triangle indices (+ 1).
// Must match `JfaUniforms` in crates/patine_procedural/src/surface.rs.

// #import procedural/mesh
// #import render/fullscreen

struct Jfa {
    mode: u32,
    step: i32,
    size: u32,
    padding: u32,
}

@group(0) @binding(0) var<uniform> jfa: Jfa;
// Mode 0 and 2: the rasterized triangles. Mode 1: the previous seeds.
@group(0) @binding(1) var source: texture_2d<u32>;
// Mode 2: the seeds.
@group(0) @binding(2) var seeds: texture_2d<u32>;

@fragment
fn fs_main(in: FullscreenOutput) -> @location(0) u32 {
    let texel = vec2<u32>(in.clip.xy);
    switch jfa.mode {
        case 0u: {
            if textureLoad(source, texel, 0).r != 0u {
                return pack_texel(texel);
            }
            return 0u;
        }
        case 1u: {
            var best = 0u;
            var best_distance = 0x7fffffff;
            let size = i32(jfa.size);
            for (var dy = -1; dy <= 1; dy++) {
                for (var dx = -1; dx <= 1; dx++) {
                    let at = vec2<i32>(texel) + vec2<i32>(dx, dy) * jfa.step;
                    if at.x < 0 || at.y < 0 || at.x >= size || at.y >= size {
                        continue;
                    }
                    let seed = textureLoad(source, vec2<u32>(at), 0).r;
                    if seed == 0u {
                        continue;
                    }
                    let d = vec2<i32>(unpack_texel(seed)) - vec2<i32>(texel);
                    let distance = d.x * d.x + d.y * d.y;
                    // Ties go to the smaller seed, so that the result does not depend
                    // on the visiting order.
                    if distance < best_distance || (distance == best_distance && seed < best) {
                        best = seed;
                        best_distance = distance;
                    }
                }
            }
            return best;
        }
        default: {
            let seed = textureLoad(seeds, texel, 0).r;
            if seed == 0u {
                return 0u;
            }
            return textureLoad(source, unpack_texel(seed), 0).r;
        }
    }
}
