//! `patine_cli bench viewport`: frames per second of the viewport on the hardware
//! adapter, for the performance budgets of PLAN.md.

use std::time::Instant;

use anyhow::{Context as _, bail};
use patine_compositor::gpu_document::GpuDocument;
use patine_core::channel::{Channel, ChannelDef};
use patine_core::project::{Project, ProjectSettings, Resolution};
use patine_gpu::pool::TexturePool;
use patine_gpu::{AdapterChoice, GpuContext, wgpu};
use patine_mesh::primitives;
use patine_render::camera::OrbitCamera;
use patine_render::material::{MaterialTextures, Slot};
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
    /// Resolution of the channel textures; `None` for the constant material.
    pub textures: Option<Resolution>,
}

impl ViewportBench {
    /// Parses `--triangles N --frames N --size WxH --no-shadows --textures N` (0 for the
    /// constant material).
    pub fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut bench = Self {
            triangles: 500_000,
            frames: 300,
            width: 1920,
            height: 1080,
            shadows: true,
            textures: Resolution::new(4096),
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
                "--textures" => {
                    let pixels: u32 = value()?.parse()?;
                    bench.textures = match pixels {
                        0 => None,
                        _ => Some(Resolution::new(pixels).context("resolution 512 to 8192")?),
                    };
                }
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

        // The six channels the lit shader reads, allocated by the document's own path.
        let mut pool = TexturePool::new(u64::MAX);
        let mut document = GpuDocument::new();
        let materials = match self.textures {
            Some(resolution) => {
                let mut project = Project::new(
                    mesh.clone(),
                    "bench".into(),
                    ProjectSettings {
                        resolution,
                        ..ProjectSettings::default()
                    },
                );
                for set in &mut project.texture_sets {
                    for channel in [Channel::AmbientOcclusion, Channel::Emissive] {
                        set.channels.push(ChannelDef::standard(channel));
                    }
                }
                document.sync(&ctx, &mut pool, &project);
                ctx.device.poll(wgpu::PollType::wait_indefinitely())?;
                let set = &document.texture_sets()[0];
                let mut textures = MaterialTextures::default();
                for texture in &set.channels {
                    if let Some(slot) = Slot::of(texture.def.channel) {
                        textures = textures.with(slot, &texture.view);
                    }
                }
                println!(
                    "textures   {} channels of {}², {:.0} MB",
                    set.channels.len(),
                    resolution.pixels(),
                    pool.memory() as f64 / 1e6
                );
                vec![viewport.material(&ctx, &textures)]
            }
            None => Vec::new(),
        };

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
                materials: (!materials.is_empty()).then_some(materials.as_slice()),
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
            "render     {}x{}, MSAA 4x, IBL, AgX, shadows {}, {}",
            self.width,
            self.height,
            if self.shadows { "on" } else { "off" },
            match self.textures {
                Some(resolution) => format!("textures {}²", resolution.pixels()),
                None => "constant material".to_owned(),
            }
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
