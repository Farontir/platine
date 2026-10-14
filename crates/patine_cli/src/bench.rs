//! `patine_cli bench viewport`: frames per second of the viewport on the hardware
//! adapter, for the performance budgets of PLAN.md.

use std::time::Instant;

use anyhow::{Context as _, bail};
use patine_compositor::compositor::Compositor;
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
                let compositor = Compositor::new(&ctx)?;
                document.sync(&ctx, &mut pool, &compositor, &project);
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

/// Options of the compositor benchmark (budget "parameter of a fill at the bottom of a
/// stack of 30 layers 4K" of PLAN.md).
pub struct CompositorBench {
    /// Number of fill layers.
    pub layers: usize,
    /// Texture set resolution.
    pub resolution: Resolution,
    /// Measured edits.
    pub edits: u32,
    /// Whether every fill has a mask (a fill and a levels).
    pub masks: bool,
}

impl CompositorBench {
    /// Parses `--layers N --resolution N --edits N --masks`.
    pub fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut bench = Self {
            layers: 30,
            resolution: Resolution::new(4096).context("supported")?,
            edits: 20,
            masks: false,
        };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let mut value = || args.next().context("missing value");
            match arg.as_str() {
                "--layers" => bench.layers = value()?.parse()?,
                "--resolution" => {
                    let pixels = value()?.parse()?;
                    bench.resolution = Resolution::new(pixels).context("resolution 512 to 8192")?;
                }
                "--edits" => bench.edits = value()?.parse()?,
                "--masks" => bench.masks = true,
                other => bail!("unknown option {other}"),
            }
        }
        Ok(bench)
    }

    /// Runs the benchmark and prints the results.
    pub fn run(&self) -> anyhow::Result<()> {
        use patine_core::command::Command;
        use patine_core::commands::{SetFillValue, SetLayerVisible, SetMaskEffect};
        use patine_core::ids::{IdSequence, LayerId};
        use patine_core::layer::{BlendMode, Layer};
        use patine_core::mask::{
            LevelsParams, MaskEffect, MaskEffectEntry, MaskInitial, MaskStack,
        };

        let ctx = GpuContext::new_headless(AdapterChoice::Hardware)?;
        let info = ctx.adapter_info();
        println!("adapter    {} ({:?})", info.name, info.backend);
        let mut project = Project::new(
            primitives::uv_sphere(32, 16),
            "bench".into(),
            ProjectSettings {
                resolution: self.resolution,
                ..ProjectSettings::default()
            },
        );
        let modes = [
            BlendMode::Normal,
            BlendMode::Multiply,
            BlendMode::Overlay,
            BlendMode::Screen,
            BlendMode::SoftLight,
        ];
        let mut ids = IdSequence::starting_at(project.next_id);
        let set = &mut project.texture_sets[0];
        let channels = set.channels.clone();
        set.stack.layers = (0..self.layers)
            .map(|i| {
                let mut layer = Layer::fill_for(ids.layer(), format!("fill {i}"), &channels);
                for (channel, blend) in &mut layer.channels {
                    if *channel != Channel::Normal {
                        blend.mode = modes[i % modes.len()];
                    }
                    blend.opacity = 0.7;
                }
                if self.masks {
                    let mut mask = MaskStack::new(MaskInitial::White);
                    let fill = MaskEffectEntry {
                        blend: BlendMode::Multiply,
                        ..MaskEffectEntry::new(ids.effect(), MaskEffect::uniform(0.8))
                    };
                    let levels = MaskEffectEntry::new(
                        ids.effect(),
                        MaskEffect::Levels(LevelsParams {
                            gamma: 1.5,
                            ..LevelsParams::default()
                        }),
                    );
                    mask.effects = vec![fill, levels];
                    layer.mask = Some(mask);
                }
                layer
            })
            .collect();
        project.next_id = ids.next_raw();
        let bottom = set.stack.layers[0].id;
        let bottom_mask = set.stack.layers[0].mask.clone();
        let channel_count = channels.len();

        let compositor = Compositor::new(&ctx)?;
        let mut pool = TexturePool::new(u64::MAX);
        let mut document = GpuDocument::new();
        let start = Instant::now();
        document.sync(&ctx, &mut pool, &compositor, &project);
        ctx.device.poll(wgpu::PollType::wait_indefinitely())?;
        println!(
            "stack      {} fills{}, {} channels of {}², first evaluation {:.0} ms",
            self.layers,
            if self.masks { " with masks" } else { "" },
            channel_count,
            self.resolution.pixels(),
            start.elapsed().as_secs_f64() * 1e3
        );
        println!(
            "memory     {:.0} MB in the pool, of which {:.0} MB of masks",
            pool.memory() as f64 / 1e6,
            document.mask_memory() as f64 / 1e6
        );

        let mut measure =
            |label: &str, edit: &mut dyn FnMut(&mut Project, u32)| -> anyhow::Result<()> {
                let mut times = Vec::new();
                for step in 0..self.edits {
                    edit(&mut project, step);
                    let start = Instant::now();
                    document.sync(&ctx, &mut pool, &compositor, &project);
                    ctx.device.poll(wgpu::PollType::wait_indefinitely())?;
                    times.push(start.elapsed().as_secs_f64() * 1e3);
                }
                let average = times.iter().sum::<f64>() / times.len() as f64;
                let max = times.iter().copied().fold(0.0, f64::max);
                println!(
                    "{label} average {average:.1} ms, max {max:.1} ms over {} edits",
                    times.len()
                );
                Ok(())
            };
        measure(
            "value      base color of the bottom fill:",
            &mut |project, step| {
                let value = [0.2 + 0.03 * step as f32, 0.4, 0.6, 1.0];
                SetFillValue::new(bottom, Channel::BaseColor, Some(value), "")
                    .apply(project)
                    .expect("the bottom layer is a fill");
            },
        )?;
        measure(
            "visibility of the bottom fill (every channel):",
            &mut |project, step| {
                SetLayerVisible::new(LayerId(bottom.0), step % 2 == 1, "")
                    .apply(project)
                    .expect("the bottom layer exists");
            },
        )?;
        if let Some(mask) = bottom_mask {
            measure(
                "mask       fill value in the mask of the bottom fill (every channel):",
                &mut |project, step| {
                    let mut entry = mask.effects[0].clone();
                    entry.effect = MaskEffect::uniform(0.3 + 0.02 * step as f32);
                    SetMaskEffect::new(bottom, entry, "")
                        .apply(project)
                        .expect("the bottom fill has a mask");
                },
            )?;
        }
        println!("budget     < 100 ms on the RTX 3080 Ti, < 250 ms on the RTX A2000");
        Ok(())
    }
}

/// Options of the procedural benchmark (M7 criterion 3: a modified parameter shows in
/// less than 100 ms).
pub struct ProceduralBench {
    /// Fill layers above the procedural one.
    pub layers: usize,
    /// Texture set resolution.
    pub resolution: Resolution,
    /// Measured edits.
    pub edits: u32,
    /// Material at the bottom of the stack.
    pub material: String,
    /// Triplanar projection instead of UV.
    pub triplanar: bool,
}

impl ProceduralBench {
    /// Parses `--layers N --resolution N --edits N --material ID --triplanar`.
    pub fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut bench = Self {
            layers: 29,
            resolution: Resolution::new(4096).context("supported")?,
            edits: 20,
            material: "material.concrete".into(),
            triplanar: false,
        };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let mut value = || args.next().context("missing value");
            match arg.as_str() {
                "--layers" => bench.layers = value()?.parse()?,
                "--resolution" => {
                    let pixels = value()?.parse()?;
                    bench.resolution = Resolution::new(pixels).context("resolution 512 to 8192")?;
                }
                "--edits" => bench.edits = value()?.parse()?,
                "--material" => bench.material = value()?.clone(),
                "--triplanar" => bench.triplanar = true,
                other => bail!("unknown option {other}"),
            }
        }
        Ok(bench)
    }

    /// Runs the benchmark and prints the results.
    pub fn run(&self) -> anyhow::Result<()> {
        use patine_core::command::Command;
        use patine_core::commands::SetFillSource;
        use patine_core::ids::IdSequence;
        use patine_core::layer::{
            BlendMode, FillSource, Layer, LayerKind, Projection, ProjectionKind,
        };
        use patine_core::mask::ParamValue;
        use patine_procedural::library::{self, MATERIAL_CHANNELS};

        let def = library::find(&self.material).context("unknown material")?;
        let ctx = GpuContext::new_headless(AdapterChoice::Hardware)?;
        let info = ctx.adapter_info();
        println!("adapter    {} ({:?})", info.name, info.backend);
        let mut project = Project::new(
            primitives::uv_sphere(64, 32),
            "bench".into(),
            ProjectSettings {
                resolution: self.resolution,
                ..ProjectSettings::default()
            },
        );
        let mut ids = IdSequence::starting_at(project.next_id);
        let set = &mut project.texture_sets[0];
        let channels: Vec<Channel> = MATERIAL_CHANNELS
            .into_iter()
            .filter(|channel| set.channels.iter().any(|d| d.channel == *channel))
            .collect();
        let bottom = ids.layer();
        let mut procedural =
            Layer::fill_procedural(bottom, "procedural", def.instance(), &channels);
        if self.triplanar
            && let LayerKind::Fill { projection, .. } = &mut procedural.kind
        {
            *projection = Projection {
                kind: ProjectionKind::Triplanar { blend: 0.5 },
                ..Projection::default()
            };
        }
        let defs = set.channels.clone();
        let mut layers = vec![procedural];
        layers.extend((0..self.layers).map(|i| {
            let mut layer = Layer::fill_for(ids.layer(), format!("fill {i}"), &defs);
            for (channel, blend) in &mut layer.channels {
                if *channel != Channel::Normal {
                    blend.mode =
                        [BlendMode::Normal, BlendMode::Multiply, BlendMode::Overlay][i % 3];
                }
                blend.opacity = 0.5;
            }
            layer
        }));
        set.stack.layers = layers;
        project.next_id = ids.next_raw();

        let compositor = Compositor::new(&ctx)?;
        let mut pool = TexturePool::new(u64::MAX);
        let mut document = GpuDocument::new();
        let start = Instant::now();
        document.sync(&ctx, &mut pool, &compositor, &project);
        ctx.device.poll(wgpu::PollType::wait_indefinitely())?;
        println!(
            "stack      {} ({}) under {} fills, {} channels of {}², first evaluation {:.0} ms \
             (surface map and pipelines included)",
            def.id,
            if self.triplanar { "triplanar" } else { "UV" },
            self.layers,
            defs.len(),
            self.resolution.pixels(),
            start.elapsed().as_secs_f64() * 1e3
        );
        println!(
            "memory     {:.0} MB in the pool, of which {:.0} MB of contents and surface map",
            pool.memory() as f64 / 1e6,
            document.content_memory() as f64 / 1e6
        );
        let mut times = Vec::new();
        for step in 0..self.edits {
            let mut instance = def.instance();
            instance.params.insert(
                "roughness".into(),
                ParamValue::Float(0.2 + 0.03 * step as f32),
            );
            SetFillSource::new(bottom, FillSource::Procedural(instance), "")
                .apply(&mut project)
                .expect("the bottom layer is a fill");
            let start = Instant::now();
            document.sync(&ctx, &mut pool, &compositor, &project);
            ctx.device.poll(wgpu::PollType::wait_indefinitely())?;
            times.push(start.elapsed().as_secs_f64() * 1e3);
        }
        let average = times.iter().sum::<f64>() / times.len() as f64;
        let max = times.iter().copied().fold(0.0, f64::max);
        println!(
            "parameter  roughness of the material: average {average:.1} ms, max {max:.1} ms over {} edits",
            times.len()
        );
        println!("budget     < 100 ms (M7 criterion 3)");
        Ok(())
    }
}

/// Options of the paint benchmark (M8 criterion 3: a dab shows in less than 8 ms on a
/// 4K stack of 20 layers).
pub struct PaintBench {
    /// Layers of the stack, the paint layer included (in the middle).
    pub layers: usize,
    /// Texture set resolution.
    pub resolution: Resolution,
    /// Dabs of the measured stroke, one per frame.
    pub dabs: usize,
    /// Brush diameter in pixels.
    pub size: f32,
}

impl PaintBench {
    /// Parses `--layers N --resolution N --dabs N --size PX`.
    pub fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut bench = Self {
            layers: 20,
            resolution: Resolution::new(4096).context("supported")?,
            dabs: 200,
            size: 80.0,
        };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let mut value = || args.next().context("missing value");
            match arg.as_str() {
                "--layers" => bench.layers = value()?.parse()?,
                "--resolution" => {
                    let pixels = value()?.parse()?;
                    bench.resolution = Resolution::new(pixels).context("resolution 512 to 8192")?;
                }
                "--dabs" => bench.dabs = value()?.parse()?,
                "--size" => bench.size = value()?.parse()?,
                other => bail!("unknown option {other}"),
            }
        }
        Ok(bench)
    }

    /// Runs the benchmark and prints the results.
    pub fn run(&self) -> anyhow::Result<()> {
        use patine_core::command::Command;
        use patine_core::commands::{PaintStroke, PaintTarget};
        use patine_core::ids::IdSequence;
        use patine_core::layer::{BlendMode, FillSource, Layer};
        use patine_core::stroke::{
            BrushSnapshot, Dab, SizeSpace, Stroke, StrokeCamera, Symmetry, Tool,
        };

        let ctx = GpuContext::new_headless(AdapterChoice::Hardware)?;
        let info = ctx.adapter_info();
        println!("adapter    {} ({:?})", info.name, info.backend);
        let mesh = primitives::uv_sphere(128, 64);
        let bounds = mesh.bounds().context("the sphere has bounds")?;
        let mut project = Project::new(
            mesh,
            "bench".into(),
            ProjectSettings {
                resolution: self.resolution,
                ..ProjectSettings::default()
            },
        );
        let mut ids = IdSequence::starting_at(project.next_id);
        let set = &mut project.texture_sets[0];
        let defs = set.channels.clone();
        let channels: Vec<Channel> = defs.iter().map(|def| def.channel).collect();
        let below = self.layers.saturating_sub(1) / 2;
        let fill = |i: usize, ids: &mut IdSequence| {
            let mut layer = Layer::fill_for(ids.layer(), format!("fill {i}"), &defs);
            for (channel, blend) in &mut layer.channels {
                if *channel != Channel::Normal {
                    blend.mode =
                        [BlendMode::Normal, BlendMode::Multiply, BlendMode::Overlay][i % 3];
                }
                blend.opacity = 0.5;
            }
            layer
        };
        let mut layers = Vec::new();
        for i in 0..below {
            layers.push(fill(i, &mut ids));
        }
        let paint = ids.layer();
        layers.push(Layer::paint(paint, "paint", &channels));
        for i in below..self.layers.saturating_sub(1) {
            layers.push(fill(i, &mut ids));
        }
        set.stack.layers = layers;
        project.next_id = ids.next_raw();

        let (width, height) = (1920u32, 1080u32);
        let aspect = width as f32 / height as f32;
        let mut camera = OrbitCamera::default();
        camera.frame(&bounds, aspect);
        let stroke_camera = StrokeCamera {
            view_proj: camera.view_projection(aspect).to_cols_array_2d(),
            viewport: [width, height],
        };
        // A circle around the sphere's center on screen, one dab per frame.
        let dabs: Vec<Dab> = (0..self.dabs)
            .map(|i| {
                let angle = i as f32 / self.dabs as f32 * std::f32::consts::TAU;
                Dab {
                    position: [960.0 + 250.0 * angle.cos(), 540.0 + 250.0 * angle.sin()],
                    pressure: 1.0,
                    tilt: [0.0; 2],
                    time: i as f32 / 60.0,
                }
            })
            .collect();
        let stroke = Stroke {
            brush: BrushSnapshot {
                size: self.size,
                size_space: SizeSpace::Screen,
                flow: 0.8,
                opacity: 1.0,
                hardness: 0.6,
                spacing: 0.1,
                angle: 0.0,
                size_jitter: 0.0,
                angle_jitter: 0.0,
                flow_jitter: 0.0,
                scatter: 0.0,
                follow_path: false,
                alpha: None,
                material: FillSource::Uniform(
                    [
                        (Channel::BaseColor, [0.7, 0.2, 0.1, 1.0]),
                        (Channel::Roughness, [0.3; 4]),
                    ]
                    .into(),
                ),
            },
            tool: Tool::Brush,
            camera: stroke_camera,
            symmetry: Symmetry::default(),
            seed: 0,
            dabs: Vec::new(),
        };

        let compositor = Compositor::new(&ctx)?;
        let mut pool = TexturePool::new(u64::MAX);
        let mut document = GpuDocument::new();
        let start = Instant::now();
        document.sync(&ctx, &mut pool, &compositor, &project);
        ctx.device.poll(wgpu::PollType::wait_indefinitely())?;
        println!(
            "stack      paint layer in the middle of {} layers, {} channels of {}², first evaluation {:.0} ms (surface map and pipelines included)",
            self.layers,
            defs.len(),
            self.resolution.pixels(),
            start.elapsed().as_secs_f64() * 1e3
        );
        let target = PaintTarget::Layer(paint);
        let start = Instant::now();
        document
            .begin_stroke(&ctx, &mut pool, &compositor, &project, target, stroke, None)
            .context("the paint layer takes strokes")?;
        ctx.device.poll(wgpu::PollType::wait_indefinitely())?;
        let begin = start.elapsed().as_secs_f64() * 1e3;
        let mut times = Vec::new();
        for dab in &dabs {
            let start = Instant::now();
            document.paint(
                &ctx,
                &mut pool,
                &compositor,
                &project,
                std::slice::from_ref(dab),
            );
            ctx.device.poll(wgpu::PollType::wait_indefinitely())?;
            times.push(start.elapsed().as_secs_f64() * 1e3);
        }
        let slowest = times
            .iter()
            .copied()
            .enumerate()
            .skip(1)
            .max_by(|a, b| a.1.total_cmp(&b.1));
        let first = times.first().copied().unwrap_or_default();
        let rest = &times[1.min(times.len())..];
        let average = rest.iter().sum::<f64>() / rest.len().max(1) as f64;
        let max = rest.iter().copied().fold(0.0, f64::max);
        let mut sorted = rest.to_vec();
        sorted.sort_by(f64::total_cmp);
        let p95 = sorted
            .get(sorted.len() * 95 / 100)
            .copied()
            .unwrap_or_default();
        println!(
            "stroke     start {begin:.1} ms; first dab {first:.1} ms (composes the layers below once)"
        );
        println!(
            "dab        {} px: average {average:.2} ms, 95th percentile {p95:.2} ms, max {max:.2} ms over {} dabs",
            self.size,
            rest.len()
        );
        if let Some((index, ms)) = slowest {
            println!(
                "slowest    dab {index}: {ms:.2} ms (new tiles saved for undo may need GPU memory)"
            );
        }
        println!("budget     < 8 ms on the RTX 3080 Ti, < 16 ms on the RTX A2000 (M8 criterion 3)");

        let start = Instant::now();
        let painted = document
            .end_stroke(&mut pool, &compositor)
            .context("dabs were painted")?;
        PaintStroke::new(target, painted, None, "paint")
            .apply(&mut project)
            .expect("the paint layer takes the stroke");
        document.sync(&ctx, &mut pool, &compositor, &project);
        ctx.device.poll(wgpu::PollType::wait_indefinitely())?;
        println!(
            "end        stroke added and synced in {:.1} ms (adopted, nothing rasterized again)",
            start.elapsed().as_secs_f64() * 1e3
        );
        println!(
            "memory     {:.0} MB in the pool, of which {:.0} MB of paint contents",
            pool.memory() as f64 / 1e6,
            document.paint_memory() as f64 / 1e6
        );
        let start = Instant::now();
        let mut fresh_pool = TexturePool::new(u64::MAX);
        let mut fresh = GpuDocument::new();
        fresh.sync(&ctx, &mut fresh_pool, &compositor, &project);
        ctx.device.poll(wgpu::PollType::wait_indefinitely())?;
        println!(
            "replay     a new document with the stroke: {:.0} ms (full evaluation and replay)",
            start.elapsed().as_secs_f64() * 1e3
        );
        Ok(())
    }
}
