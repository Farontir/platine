//! The WGSL noises and patterns against their CPU reference, and M7 acceptance
//! criterion 1: every noise is tileable (its edges agree to 1e-3).

use bytemuck::{Pod, Zeroable};
use patine_gpu::{GpuContext, gpu_test, readback, wgpu};
use patine_procedural::reference::noise::{
    fractal, perlin_noise, simplex_noise, value_noise, warp_noise, worley_noise,
};
use patine_procedural::reference::pattern::{
    pattern_bricks, pattern_checker, pattern_circles, pattern_fibers, pattern_gradient,
    pattern_scratches, pattern_stripes, pattern_tiles,
};
use patine_procedural::reference::{IVec2, Vec2};
use wgpu::util::DeviceExt as _;

/// Uniforms of `shaders/tests/noise_probe.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Probe {
    function: u32,
    seed: u32,
    a: i32,
    b: i32,
    period: [i32; 2],
    x: f32,
    y: f32,
    z: f32,
    count: u32,
    padding: [u32; 2],
}

/// One function of the library with its parameters.
#[derive(Clone, Copy, Debug)]
struct Case {
    name: &'static str,
    function: u32,
    a: i32,
    b: i32,
    x: f32,
    y: f32,
    z: f32,
    noise: bool,
}

const fn case(name: &'static str, function: u32, a: i32, b: i32, x: f32, y: f32, z: f32) -> Case {
    Case {
        name,
        function,
        a,
        b,
        x,
        y,
        z,
        noise: function <= 5,
    }
}

const CASES: [Case; 18] = [
    case("value", 0, 0, 0, 0.0, 0.0, 0.0),
    case("perlin", 1, 0, 0, 0.0, 0.0, 0.0),
    case("simplex", 2, 0, 0, 0.0, 0.0, 0.0),
    case("worley f1", 3, 0, 0, 0.0, 0.0, 0.0),
    case("worley f2", 3, 1, 0, 0.0, 0.0, 0.0),
    case("worley f2-f1", 3, 2, 0, 0.0, 0.0, 0.0),
    case("fbm", 4, 5, 0, 0.5, 0.0, 0.0),
    case("ridged", 4, 4, 1, 0.6, 0.0, 0.0),
    case("turbulence", 4, 4, 2, 0.5, 0.0, 0.0),
    case("warp", 5, 3, 0, 0.5, 0.7, 0.0),
    case("gradient radial", 6, 3, 0, 0.0, 0.0, 0.0),
    case("checker", 7, 0, 0, 0.0, 0.0, 0.0),
    case("tiles", 8, 4, 0, 0.08, 0.05, 0.3),
    case("bricks", 9, 3, 0, 0.06, 0.04, 0.4),
    case("stripes", 10, 6, 1, 0.4, 0.1, 0.0),
    case("circles", 11, 5, 0, 0.6, 0.1, 0.0),
    case("scratches", 12, 6, 0, 0.8, 0.04, 0.0),
    case("fibers", 13, 3, 6, 0.0, 0.0, 0.0),
];

impl Case {
    /// The CPU reference.
    fn cpu(&self, p: Vec2, period: IVec2, seed: u32) -> f32 {
        let (a, b, x, y, z) = (self.a, self.b, self.x, self.y, self.z);
        match self.function {
            0 => value_noise(p, period, seed),
            1 => perlin_noise(p, period, seed),
            2 => simplex_noise(p, period, seed),
            3 => worley_noise(p, period, seed, a as u32),
            4 => fractal(p, period, seed, a, x, b as u32),
            5 => warp_noise(p, period, seed, a, x, y),
            6 => pattern_gradient(p, a as u32),
            7 => pattern_checker(p),
            8 => pattern_tiles(p, a, x, y, z, seed),
            9 => pattern_bricks(p, a, x, y, z, seed),
            10 => pattern_stripes(p, a, x, y, b as u32),
            11 => pattern_circles(p, a, x, y),
            12 => pattern_scratches(p, a, x, y, seed),
            _ => pattern_fibers(p, a, b, seed),
        }
    }
}

/// The probe pipeline, compiled once per test thread.
fn pipeline(ctx: &GpuContext) -> wgpu::ComputePipeline {
    thread_local! {
        static PIPELINE: std::cell::OnceCell<wgpu::ComputePipeline> = const { std::cell::OnceCell::new() };
    }
    PIPELINE.with(|cell| {
        cell.get_or_init(|| {
            let module = ctx
                .create_shader_module("tests/noise_probe")
                .expect("the probe compiles");
            ctx.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("noise probe"),
                    layout: None,
                    module: &module,
                    entry_point: Some("cs_main"),
                    compilation_options: Default::default(),
                    cache: None,
                })
        })
        .clone()
    })
}

/// Evaluates `case` on the GPU at `points`.
fn gpu(ctx: &GpuContext, case: &Case, points: &[Vec2], period: IVec2, seed: u32) -> Vec<f32> {
    let pipeline = pipeline(ctx);
    let probe = Probe {
        function: case.function,
        seed,
        a: case.a,
        b: case.b,
        period: [period.x, period.y],
        x: case.x,
        y: case.y,
        z: case.z,
        count: points.len() as u32,
        padding: [0; 2],
    };
    let uniforms = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("probe"),
            contents: bytemuck::bytes_of(&probe),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let flat: Vec<[f32; 2]> = points.iter().map(|p| [p.x, p.y]).collect();
    let input = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("probe points"),
            contents: bytemuck::cast_slice(&flat),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let size = (points.len() * 4) as u64;
    let output = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("probe values"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("probe"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let mut encoder = ctx.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups((points.len() as u32).div_ceil(64), 1, 1);
    }
    ctx.queue.submit([encoder.finish()]);
    let bytes = readback::read_buffer(ctx, &output, size).expect("readback");
    bytemuck::cast_slice(&bytes).to_vec()
}

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

gpu_test!(
    /// Every function agrees with its reference to 1e-4 at random points, and its seed
    /// changes it.
    fn the_library_matches_its_reference(ctx) {
        let mut random = Random(5);
        let points: Vec<Vec2> = (0..2048)
            .map(|_| Vec2::new(random.next() * 12.0 - 3.0, random.next() * 12.0 - 3.0))
            .collect();
        let period = IVec2::new(4, 6);
        let mut failures = Vec::new();
        for case in &CASES {
            for seed in [0, 77] {
                let values = gpu(&ctx, case, &points, period, seed);
                let worst = points
                    .iter()
                    .zip(&values)
                    .map(|(p, gpu)| (gpu - case.cpu(*p, period, seed)).abs())
                    .fold(0.0f32, f32::max);
                if worst > 1e-4 {
                    failures.push(format!("{} (seed {seed}): {worst}", case.name));
                }
            }
            // Gradients, checkers, stripes and circles have no seed by design.
            let seeded = !matches!(case.function, 6 | 7 | 10 | 11);
            if seeded {
                let a = gpu(&ctx, case, &points, period, 0);
                let b = gpu(&ctx, case, &points, period, 1);
                let change = a.iter().zip(&b).map(|(a, b)| (a - b).abs()).sum::<f32>() / a.len() as f32;
                assert!(change > 0.01, "{}: the seed changes nothing ({change})", case.name);
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }
);

gpu_test!(
    /// M7 acceptance criterion 1: on the GPU and on the CPU, every noise (and every
    /// pattern) takes the same values on opposite edges of its period, to 1e-3, for
    /// several periods and seeds.
    fn every_noise_is_tileable(ctx) {
        let mut failures = Vec::new();
        for case in &CASES {
            let periods: &[IVec2] = if case.noise {
                &[IVec2::new(1, 1), IVec2::new(3, 3), IVec2::new(4, 7), IVec2::new(8, 8)]
            } else {
                // Patterns repeat once per unit.
                &[IVec2::new(1, 1)]
            };
            for &period in periods {
                for seed in [0, 12345] {
                    let size = Vec2::new(period.x as f32, period.y as f32);
                    let mut near = Vec::new();
                    let mut far = Vec::new();
                    for i in 0..64 {
                        let t = (i as f32 + 0.37) / 64.0;
                        // Left and right edges, then bottom and top edges.
                        near.push(Vec2::new(0.0, t * size.y));
                        far.push(Vec2::new(size.x, t * size.y));
                        near.push(Vec2::new(t * size.x, 0.0));
                        far.push(Vec2::new(t * size.x, size.y));
                    }
                    let (gpu_near, gpu_far) = (
                        gpu(&ctx, case, &near, period, seed),
                        gpu(&ctx, case, &far, period, seed),
                    );
                    for i in 0..near.len() {
                        let cpu = (case.cpu(near[i], period, seed) - case.cpu(far[i], period, seed)).abs();
                        let gpu = (gpu_near[i] - gpu_far[i]).abs();
                        if cpu > 1e-3 || gpu > 1e-3 {
                            failures.push(format!(
                                "{} period {period} seed {seed} at {}: cpu {cpu}, gpu {gpu}",
                                case.name, near[i]
                            ));
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{} failures: {:#?}", failures.len(), &failures[..failures.len().min(8)]);
    }
);
