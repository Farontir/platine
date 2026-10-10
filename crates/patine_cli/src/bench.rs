//! `patine_cli bench viewport`: frames per second of the viewport on the hardware
//! adapter, for the performance budgets of PLAN.md.

use std::time::Instant;

use anyhow::{Context as _, bail};
use patine_gpu::{AdapterChoice, GpuContext, wgpu};
use patine_mesh::primitives;
use patine_render::camera::OrbitCamera;
use patine_render::mesh_gpu::GpuMesh;
use patine_render::settings::ViewSettings;
use patine_render::viewport::{Scene, Viewport};

/// Options of the viewport benchmark.
pub struct ViewportBench {
    /// Approximate triangle count of the test sphere.
    pub triangles: u32,
    /// Measured frames.
    pub frames: u32,
    /// Image size.
    pub width: u32,
    /// Image size.
    pub height: u32,
    /// Shadows on (default) or off.
    pub shadows: bool,
}

impl ViewportBench {
    /// Parses `--triangles N --frames N --size WxH --no-shadows`.
    pub fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut bench = Self {
            triangles: 500_000,
            frames: 300,
            width: 1920,
            height: 1080,
            shadows: true,
        };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let mut value = || args.next().context("missing value");
            match arg.as_str() {
                "--triangles" => bench.triangles = value()?.parse()?,
                "--frames" => bench.frames = value()?.parse()?,
                "--size" => {
                    let size = value()?;
                    let (w, h) = size.split_once('x').context("size is WIDTHxHEIGHT")?;
                    bench.width = w.parse()?;
                    bench.height = h.parse()?;
                }
                "--no-shadows" => bench.shadows = false,
                other => bail!("unknown option {other}"),
            }
        }
        Ok(bench)
    }

    /// Runs the benchmark and prints the results.
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = GpuContext::new_headless(AdapterChoice::Hardware)?;
        let info = ctx.adapter_info();
        println!("adapter    {} ({:?})", info.name, info.backend);

        // A UV sphere with 2 × rings segments has about 4 × rings² triangles.
        let rings = ((self.triangles as f32 / 4.0).sqrt().round() as u32).max(4);
        let start = Instant::now();
        let mesh = primitives::uv_sphere(2 * rings, rings);
        println!(
            "mesh       {} triangles, generated in {:.0} ms",
            mesh.triangle_count(),
            start.elapsed().as_secs_f64() * 1e3
        );

        let start = Instant::now();
        let mut viewport = Viewport::new(&ctx)?;
        ctx.device.poll(wgpu::PollType::wait_indefinitely())?;
        println!(
            "viewport   created in {:.0} ms (LUT, studio sky, cube, prefiltering)",
            start.elapsed().as_secs_f64() * 1e3
        );
        let gpu_mesh = if viewport.needs_flat_pick_buffers() {
            GpuMesh::with_flat_picking(&ctx, &mesh)
        } else {
            GpuMesh::new(&ctx, &mesh)
        }
        .context("empty mesh")?;

        let mut camera = OrbitCamera::default();
        camera.frame(&gpu_mesh.bounds, self.width as f32 / self.height as f32);
        let settings = ViewSettings {
            shadows: self.shadows,
            ..ViewSettings::default()
        };
        let mut render = |frame: u32, viewport: &mut Viewport| {
            // Orbit a little every frame, as during interaction.
            camera.orbit(0.01, 0.0);
            let scene = Scene {
                mesh: Some(&gpu_mesh),
                camera: &camera,
                settings: &settings,
            };
            viewport.render(&ctx, &scene, self.width, self.height);
            // Keep the queue from growing without bound, like a presenting swapchain.
            if frame % 3 == 2 {
                let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
            }
        };
        for frame in 0..30 {
            render(frame, &mut viewport);
        }
        ctx.device.poll(wgpu::PollType::wait_indefinitely())?;

        let start = Instant::now();
        for frame in 0..self.frames {
            render(frame, &mut viewport);
        }
        ctx.device.poll(wgpu::PollType::wait_indefinitely())?;
        let seconds = start.elapsed().as_secs_f64();
        println!(
            "render     {}x{}, MSAA 4x, IBL, AgX, shadows {}",
            self.width,
            self.height,
            if self.shadows { "on" } else { "off" }
        );
        println!(
            "result     {:.0} frames/s ({:.2} ms/frame over {} frames)",
            f64::from(self.frames) / seconds,
            seconds * 1e3 / f64::from(self.frames),
            self.frames
        );
        Ok(())
    }
}
