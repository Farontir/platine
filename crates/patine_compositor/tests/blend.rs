//! M5 acceptance criterion 1: every blend mode matches the CPU reference to 1/512, for
//! colors, scalars and normals. Also checks that a pass restricted to some tiles leaves
//! the others alone, and the mip reduction.

use patine_compositor::compositor::{BlendStep, Compositor, Ping, Region, Source, format_of};
use patine_compositor::reference::{self, ALL_MODES, ChannelKind, round_f16};
use patine_core::layer::BlendMode;
use patine_gpu::{GpuContext, gpu_test, readback, wgpu};

const SIZE: u32 = 16;

/// Deterministic pseudo-random numbers in [0, 1).
struct Random(u64);

impl Random {
    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / (1u64 << 24) as f32
    }
}

fn unit(v: [f32; 3]) -> [f32; 4] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / l, v[1] / l, v[2] / l, 0.0]
}

/// Destination texels for `kind`, rounded to f16 as stored: values in [0, 1] including
/// the exact ends for colors and scalars, unit vectors with z > 0.3 for normals.
fn destination(kind: ChannelKind, random: &mut Random) -> Vec<[f32; 4]> {
    (0..SIZE * SIZE)
        .map(|i| {
            let value = match kind {
                ChannelKind::Normal => unit([
                    random.next() * 2.0 - 1.0,
                    random.next() * 2.0 - 1.0,
                    0.3 + random.next(),
                ]),
                _ if i == 0 => [0.0; 4],
                _ if i == 1 => [1.0; 4],
                _ => [random.next(), random.next(), random.next(), random.next()],
            };
            let value = if kind == ChannelKind::Scalar {
                [value[0], 0.0, 0.0, 0.0]
            } else {
                value
            };
            value.map(round_f16)
        })
        .collect()
}

fn texture(ctx: &GpuContext, kind: ChannelKind, texels: &[[f32; 4]]) -> wgpu::Texture {
    let format = format_of(kind);
    let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("blend test"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let components = if kind == ChannelKind::Scalar { 1 } else { 4 };
    let bytes: Vec<u8> = texels
        .iter()
        .flat_map(|t| t[..components].to_vec())
        .flat_map(|v| half::f16::from_f32(v).to_le_bytes())
        .collect();
    ctx.queue.write_texture(
        texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(SIZE * 2 * components as u32),
            rows_per_image: None,
        },
        texture.size(),
    );
    texture
}

fn read(ctx: &GpuContext, kind: ChannelKind, texture: &wgpu::Texture) -> Vec<[f32; 4]> {
    match kind {
        ChannelKind::Scalar => readback::read_r16f_level(ctx, texture, 0, 0)
            .expect("readback")
            .into_iter()
            .map(|r| [r, 0.0, 0.0, 0.0])
            .collect(),
        _ => readback::read_rgba16f(ctx, texture).expect("readback"),
    }
}

/// Runs one blend pass of `step` over a destination and returns (destination, result).
fn run(
    ctx: &GpuContext,
    compositor: &Compositor,
    kind: ChannelKind,
    dst: &[[f32; 4]],
    step: BlendStep<'_>,
    region: &Region,
) -> Vec<[f32; 4]> {
    let source = texture(ctx, kind, dst);
    // The target starts as a copy of the destination, as in the compositor's ping-pong.
    let target = texture(ctx, kind, dst);
    let mut encoder = ctx.device.create_command_encoder(&Default::default());
    compositor.blend(
        ctx,
        &mut encoder,
        kind,
        Ping {
            read: &source.create_view(&Default::default()),
            write: &target.create_view(&Default::default()),
        },
        &step,
        region,
    );
    ctx.queue.submit([encoder.finish()]);
    read(ctx, kind, &target)
}

/// 1/512, relative above 1 (f16 storage of values beyond 1).
fn within(gpu: f32, cpu: f32) -> bool {
    (gpu - cpu).abs() <= cpu.abs().max(1.0) / 512.0
}

gpu_test!(
    fn every_blend_mode_matches_the_reference(ctx) {
        let compositor = Compositor::new(&ctx).expect("shaders compile");
        let mut random = Random(7);
        let region = Region::full(SIZE);
        let mut checked = 0;
        for kind in [ChannelKind::Color, ChannelKind::Scalar, ChannelKind::Normal] {
            let dst = destination(kind, &mut random);
            let sources: Vec<[f32; 4]> = match kind {
                ChannelKind::Normal => vec![
                    unit([0.4, -0.3, 0.85]),
                    unit([-0.7, 0.1, 0.7]),
                    [0.0, 0.0, 1.0, 0.0],
                ],
                // Zero and one exercise the special cases of Divide and the dodges.
                _ => vec![[0.0; 4], [1.0; 4], [0.3, 0.6, 0.9, 1.0], [0.55, 0.2, 0.75, 0.5]],
            };
            for mode in ALL_MODES {
                for src in &sources {
                    for opacity in [1.0, 0.65, 0.0] {
                        let step = BlendStep {
                            mode,
                            opacity,
                            source: Source::Value(*src),
                            mask: None,
                        };
                        let gpu = run(&ctx, &compositor, kind, &dst, step, &region);
                        for (i, (g, d)) in gpu.iter().zip(&dst).enumerate() {
                            let cpu = reference::apply(mode, kind, *d, *src, opacity).map(round_f16);
                            let components = if kind == ChannelKind::Scalar { 1 } else { 4 };
                            for c in 0..components {
                                assert!(
                                    within(g[c], cpu[c]),
                                    "{mode:?} {kind:?} src {src:?} opacity {opacity} texel {i}: \
                                     dst {d:?}, gpu {g:?}, cpu {cpu:?}"
                                );
                            }
                            checked += 1;
                        }
                    }
                }
            }
        }
        eprintln!("{checked} texels checked");
    }
);

gpu_test!(
    /// Folder results and other textures blend like constants.
    fn texture_sources_match_the_reference(ctx) {
        let compositor = Compositor::new(&ctx).expect("shaders compile");
        let mut random = Random(11);
        let kind = ChannelKind::Color;
        let dst = destination(kind, &mut random);
        let src = destination(kind, &mut random);
        let source = texture(&ctx, kind, &src);
        let view = source.create_view(&Default::default());
        for mode in [BlendMode::Normal, BlendMode::Overlay, BlendMode::Color] {
            let step = BlendStep {
                mode,
                opacity: 0.8,
                source: Source::Texture(&view),
                mask: None,
            };
            let gpu = run(&ctx, &compositor, kind, &dst, step, &Region::full(SIZE));
            for ((g, d), s) in gpu.iter().zip(&dst).zip(&src) {
                let cpu = reference::apply(mode, kind, *d, *s, 0.8).map(round_f16);
                assert!((0..4).all(|c| within(g[c], cpu[c])), "{mode:?}: {g:?} vs {cpu:?}");
            }
        }
    }
);

gpu_test!(
    fn passes_only_touch_their_tiles(ctx) {
        let compositor = Compositor::new(&ctx).expect("shaders compile");
        let size = 256;
        let kind = ChannelKind::Scalar;
        let make = |value: f32| {
            let texels = vec![[value, 0.0, 0.0, 0.0]; (size * size) as usize];
            let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("tiles"),
                size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: format_of(kind),
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let bytes: Vec<u8> = texels.iter().flat_map(|t| half::f16::from_f32(t[0]).to_le_bytes()).collect();
            ctx.queue.write_texture(
                texture.as_image_copy(),
                &bytes,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size * 2), rows_per_image: None },
                texture.size(),
            );
            texture
        };
        let dst = make(0.25);
        let target = make(0.25);
        let mut region = Region::empty(size);
        region.add_tile(1, 0);
        let step = BlendStep {
            mode: BlendMode::Normal,
            opacity: 1.0,
            source: Source::Value([0.75; 4]),
            mask: None,
        };
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        compositor.blend(
            &ctx,
            &mut encoder,
            kind,
            Ping {
                read: &dst.create_view(&Default::default()),
                write: &target.create_view(&Default::default()),
            },
            &step,
            &region,
        );
        ctx.queue.submit([encoder.finish()]);
        let texels = readback::read_r16f_level(&ctx, &target, 0, 0).unwrap();
        for y in 0..size {
            for x in 0..size {
                let inside = (128..256).contains(&x) && y < 128;
                let expected = if inside { 0.75 } else { 0.25 };
                assert_eq!(texels[(y * size + x) as usize], expected, "texel {x}, {y}");
            }
        }
    }
);

gpu_test!(
    fn mips_match_the_reference_reduction(ctx) {
        let compositor = Compositor::new(&ctx).expect("shaders compile");
        let mut random = Random(3);
        for kind in [ChannelKind::Color, ChannelKind::Normal] {
            let level0 = destination(kind, &mut random);
            let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("mips"),
                size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
                mip_level_count: 5,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: format_of(kind),
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let bytes: Vec<u8> = level0
                .iter()
                .flatten()
                .flat_map(|v| half::f16::from_f32(*v).to_le_bytes())
                .collect();
            ctx.queue.write_texture(
                texture.as_image_copy(),
                &bytes,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(SIZE * 8), rows_per_image: None },
                wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
            );
            let mut encoder = ctx.device.create_command_encoder(&Default::default());
            compositor.build_mips(&ctx, &mut encoder, kind, &texture);
            ctx.queue.submit([encoder.finish()]);
            let mut expected = level0;
            let mut size = SIZE as usize;
            for level in 1..5 {
                expected = reference::downsample(&expected, size, size, kind)
                    .into_iter()
                    .map(|t| t.map(round_f16))
                    .collect();
                size /= 2;
                let gpu = readback::read_rgba16f_level(&ctx, &texture, level, 0).unwrap();
                for (g, c) in gpu.iter().zip(&expected) {
                    assert!((0..4).all(|i| within(g[i], c[i])), "{kind:?} mip {level}: {g:?} vs {c:?}");
                }
                // Continue from what the GPU stored, as the next level does.
                expected = gpu;
            }
        }
    }
);
