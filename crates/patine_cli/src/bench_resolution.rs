//! M10 benchmarks: replaying the strokes of a project at other resolutions
//! (`bench resolution`), and opening a project with or without its raster caches
//! (`bench open`, budget: a 4K project of 30 layers opens in less than 5 s).

use std::io::Cursor;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, bail};
use patine_compositor::cache::RasterCache;
use patine_compositor::compositor::Compositor;
use patine_compositor::gpu_document::GpuDocument;
use patine_core::channel::Channel;
use patine_core::command::Command as _;
use patine_core::commands::{PaintStroke, PaintTarget};
use patine_core::file;
use patine_core::ids::{IdSequence, LayerId};
use patine_core::layer::{BlendMode, FillSource, Layer};
use patine_core::project::{Project, ProjectSettings, Resolution};
use patine_core::stroke::{
    BrushSnapshot, Dab, PressureCurves, SizeSpace, Stroke, StrokeCamera, StrokeSpace, Symmetry,
    Tool,
};
use patine_gpu::pool::TexturePool;
use patine_gpu::{AdapterChoice, GpuContext, wgpu};
use patine_mesh::primitives;
use patine_render::camera::OrbitCamera;

/// A sphere at `resolution` with `layers` layers: `paint` paint layers spread among
/// fills, each with `strokes` strokes of `dabs` dabs. Returns the project and its paint
/// layers.
fn painted_project(
    resolution: Resolution,
    layers: usize,
    paint: usize,
    strokes: usize,
    dabs: usize,
) -> anyhow::Result<(Project, Vec<LayerId>)> {
    let mesh = primitives::uv_sphere(128, 64);
    let bounds = mesh.bounds().context("the sphere has bounds")?;
    let mut project = Project::new(
        mesh,
        "bench".into(),
        ProjectSettings {
            resolution,
            ..ProjectSettings::default()
        },
    );
    let mut ids = IdSequence::starting_at(project.next_id);
    let defs = project.texture_sets[0].channels.clone();
    let channels: Vec<Channel> = defs.iter().map(|def| def.channel).collect();
    let paint = paint.min(layers);
    let mut stack = Vec::new();
    let mut painted = Vec::new();
    for i in 0..layers {
        // Paint layers spread evenly among the fills.
        if painted.len() < paint && i * paint / layers.max(1) >= painted.len() {
            let id = ids.layer();
            stack.push(Layer::paint(id, format!("paint {i}"), &channels));
            painted.push(id);
        } else {
            let mut fill = Layer::fill_for(ids.layer(), format!("fill {i}"), &defs);
            for (channel, blend) in &mut fill.channels {
                if *channel != Channel::Normal {
                    blend.mode =
                        [BlendMode::Normal, BlendMode::Multiply, BlendMode::Overlay][i % 3];
                }
                blend.opacity = 0.5;
            }
            stack.push(fill);
        }
    }
    project.texture_sets[0].stack.layers = stack;
    project.next_id = ids.next_raw();

    let (width, height) = (1920u32, 1080u32);
    let aspect = width as f32 / height as f32;
    let mut camera = OrbitCamera::default();
    camera.frame(&bounds, aspect);
    let stroke_camera = StrokeCamera {
        view_proj: camera.view_projection(aspect).to_cols_array_2d(),
        viewport: [width, height],
    };
    for (l, layer) in painted.iter().enumerate() {
        for s in 0..strokes {
            // Arcs around the screen center, a different radius and start for each.
            let radius = 120.0 + 40.0 * ((l * strokes + s) % 8) as f32;
            let start = (l * 37 + s * 11) as f32 * 0.21;
            let points: Vec<Dab> = (0..dabs)
                .map(|i| {
                    let angle = start + i as f32 / dabs as f32 * 2.5;
                    Dab {
                        position: [960.0 + radius * angle.cos(), 540.0 + radius * angle.sin()],
                        pressure: 1.0,
                        tilt: [0.0; 2],
                        time: i as f32 / 60.0,
                    }
                })
                .collect();
            let stroke = Stroke {
                brush: BrushSnapshot {
                    size: 60.0,
                    size_space: SizeSpace::Screen,
                    flow: 0.8,
                    opacity: 1.0,
                    hardness: 0.6,
                    spacing: 0.1,
                    angle: 0.0,
                    size_jitter: 0.2,
                    angle_jitter: 0.0,
                    flow_jitter: 0.2,
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
                    pressure: PressureCurves::none(),
                    tilt_angle: false,
                    projection: BrushSnapshot::default_projection(),
                },
                tool: Tool::Brush,
                camera: stroke_camera,
                space: StrokeSpace::View,
                symmetry: Symmetry::default(),
                seed: (l * strokes + s) as u64,
                dabs: points,
            };
            PaintStroke::new(PaintTarget::Layer(*layer), Arc::new(stroke), None, "bench")
                .apply(&mut project)
                .map_err(|error| anyhow::anyhow!("{error}"))?;
        }
    }
    Ok((project, painted))
}

fn wait(ctx: &GpuContext) -> anyhow::Result<()> {
    ctx.device.poll(wgpu::PollType::wait_indefinitely())?;
    Ok(())
}

fn resolution(value: &str) -> anyhow::Result<Resolution> {
    Resolution::new(value.parse()?).context("resolution 512 to 8192, a power of 2")
}

/// Options of `bench resolution`.
pub struct ResolutionBench {
    /// Paint layers.
    pub layers: usize,
    /// Strokes per paint layer.
    pub strokes: usize,
    /// Dabs per stroke.
    pub dabs: usize,
}

impl ResolutionBench {
    /// Parses `--layers N --strokes N --dabs N`.
    pub fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut bench = Self {
            layers: 4,
            strokes: 25,
            dabs: 400,
        };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let mut value = || args.next().context("missing value");
            match arg.as_str() {
                "--layers" => bench.layers = value()?.parse()?,
                "--strokes" => bench.strokes = value()?.parse()?,
                "--dabs" => bench.dabs = value()?.parse()?,
                other => bail!("unknown option {other}"),
            }
        }
        Ok(bench)
    }

    /// Replays the project at 1K, 4K, 8K and 1K again, then measures the longest stroke
    /// replayed alone at 8K.
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = GpuContext::new_headless(AdapterChoice::Hardware)?;
        let info = ctx.adapter_info();
        println!("adapter    {} ({:?})", info.name, info.backend);
        let low = resolution("1024")?;
        let (mut project, painted) =
            painted_project(low, self.layers, self.layers, self.strokes, self.dabs)?;
        println!(
            "project    sphere, {} paint layers of {} strokes of {} dabs (60 px)",
            self.layers, self.strokes, self.dabs
        );
        let compositor = Compositor::new(&ctx)?;
        let mut pool = TexturePool::new(u64::MAX);
        let mut document = GpuDocument::new();
        for pixels in ["1024", "4096", "8192", "1024"] {
            project.texture_sets[0].resolution = resolution(pixels)?;
            let replays = document.paints().replays();
            let start = Instant::now();
            document.sync(&ctx, &mut pool, &compositor, &project);
            wait(&ctx)?;
            println!(
                "{pixels:>5}²     {} strokes replayed and composed in {} ms, {:.0} MB in the pool",
                document.paints().replays() - replays,
                start.elapsed().as_millis(),
                pool.memory() as f64 / 1e6
            );
        }
        document.clear(&mut pool);
        drop(pool);
        wait(&ctx)?;

        // One stroke at a time at 8K: the longest GPU work a replay submits at once.
        project.texture_sets[0].resolution = resolution("8192")?;
        let full: Vec<_> = project.texture_sets[0].stack.layers.clone();
        let mut alone = project.clone();
        let target = painted[0];
        let mut pool = TexturePool::new(u64::MAX);
        let mut document = GpuDocument::new();
        let strokes = match &full
            .iter()
            .find(|l| l.id == target)
            .context("painted")?
            .kind
        {
            patine_core::layer::LayerKind::Paint { strokes } => strokes.strokes.clone(),
            _ => bail!("not a paint layer"),
        };
        alone.texture_sets[0]
            .stack
            .layers
            .retain(|l| l.id == target);
        set_strokes(&mut alone, target, Vec::new());
        document.sync(&ctx, &mut pool, &compositor, &alone);
        wait(&ctx)?;
        let mut longest = 0.0f64;
        for i in 0..strokes.len() {
            set_strokes(&mut alone, target, strokes[..=i].to_vec());
            let start = Instant::now();
            document.sync(&ctx, &mut pool, &compositor, &alone);
            wait(&ctx)?;
            longest = longest.max(start.elapsed().as_secs_f64() * 1e3);
        }
        println!(
            "8192²      longest stroke ({} dabs) replayed and composed alone: {longest:.0} ms (driver timeout about 2000 ms)",
            self.dabs
        );
        document.clear(&mut pool);
        Ok(())
    }
}

/// Replaces the strokes of paint layer `layer`.
fn set_strokes(project: &mut Project, layer: LayerId, strokes: Vec<Arc<Stroke>>) {
    for set in &mut project.texture_sets {
        set.stack.visit_mut(&mut |candidate| {
            if candidate.id == layer
                && let patine_core::layer::LayerKind::Paint { strokes: log } = &mut candidate.kind
            {
                *log = Arc::new(patine_core::stroke::StrokeLog {
                    strokes: strokes.clone(),
                });
            }
        });
    }
}

/// Options of `bench open`.
pub struct OpenBench {
    /// Layers of the stack.
    pub layers: usize,
    /// Paint layers among them.
    pub paint: usize,
    /// Strokes per paint layer.
    pub strokes: usize,
    /// Texture set resolution.
    pub resolution: Resolution,
    /// Keeps the file written with caches.
    pub out: Option<std::path::PathBuf>,
}

impl OpenBench {
    /// Parses `--layers N --paint N --strokes N --resolution N --out FILE`.
    pub fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut bench = Self {
            layers: 30,
            paint: 10,
            strokes: 20,
            resolution: resolution("4096")?,
            out: None,
        };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let mut value = || args.next().context("missing value");
            match arg.as_str() {
                "--layers" => bench.layers = value()?.parse()?,
                "--paint" => bench.paint = value()?.parse()?,
                "--strokes" => bench.strokes = value()?.parse()?,
                "--resolution" => bench.resolution = resolution(value()?)?,
                "--out" => bench.out = Some(file::with_extension(value()?)),
                other => bail!("unknown option {other}"),
            }
        }
        Ok(bench)
    }

    /// Saves the project with and without raster caches, then opens each, up to its
    /// first full evaluation on the GPU.
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = GpuContext::new_headless(AdapterChoice::Hardware)?;
        let info = ctx.adapter_info();
        println!("adapter    {} ({:?})", info.name, info.backend);
        let (project, _) =
            painted_project(self.resolution, self.layers, self.paint, self.strokes, 300)?;
        println!(
            "project    {} layers, {} of them painted ({} strokes of 300 dabs each), {}²",
            self.layers,
            self.paint,
            self.strokes,
            self.resolution.pixels()
        );
        let compositor = Compositor::new(&ctx)?;

        // The painted document, then its caches.
        let mut pool = TexturePool::new(u64::MAX);
        let mut document = GpuDocument::new();
        let start = Instant::now();
        document.sync(&ctx, &mut pool, &compositor, &project);
        wait(&ctx)?;
        println!(
            "evaluate   first evaluation with replay: {} ms",
            start.elapsed().as_millis()
        );
        let start = Instant::now();
        let caches = document.raster_caches(&ctx, &pool, &project)?;
        let readback = start.elapsed();
        let start = Instant::now();
        let encoded: file::Caches = patine_compositor::cache::encode_all(&caches)
            .into_iter()
            .collect();
        let encode = start.elapsed();
        let tiles: usize = caches
            .iter()
            .flat_map(|(_, cache)| &cache.channels)
            .map(|channel| channel.tiles.len())
            .sum();
        println!(
            "caches     {} nodes, {tiles} channel tiles: read back in {} ms (main thread), encoded in {} ms",
            caches.len(),
            readback.as_millis(),
            encode.as_millis()
        );
        document.clear(&mut pool);
        drop(pool);
        wait(&ctx)?;

        for with_caches in [false, true] {
            let empty = file::Caches::new();
            let start = Instant::now();
            let bytes = file::write_with_caches(
                &project,
                Cursor::new(Vec::new()),
                None,
                if with_caches { &encoded } else { &empty },
            )?
            .into_inner();
            let write = start.elapsed();
            if with_caches && let Some(out) = &self.out {
                std::fs::write(out, &bytes)
                    .with_context(|| format!("cannot write {}", out.display()))?;
            }

            let start = Instant::now();
            let (read, raw) = file::read_with_caches(Cursor::new(bytes.as_slice()))?;
            let stored: Vec<(u64, Vec<u8>)> = raw.into_iter().collect();
            let decoded: Vec<(u64, RasterCache)> = patine_compositor::cache::decode_all(&stored)
                .into_iter()
                .filter_map(|(id, cache)| Some((id, cache.ok()?)))
                .collect();
            let read_time = start.elapsed();
            let mut pool = TexturePool::new(u64::MAX);
            let mut document = GpuDocument::new();
            document.provide_caches(decoded);
            let gpu_start = Instant::now();
            document.sync(&ctx, &mut pool, &compositor, &read);
            wait(&ctx)?;
            let gpu = gpu_start.elapsed();
            println!(
                "{}  file {:.1} MB written in {} ms; open: read {} ms + evaluate {} ms = {} ms ({} from caches, {} strokes replayed)",
                if with_caches {
                    "caches   "
                } else {
                    "no caches"
                },
                bytes.len() as f64 / 1e6,
                write.as_millis(),
                read_time.as_millis(),
                gpu.as_millis(),
                (read_time + gpu).as_millis(),
                document.paints().cache_loads(),
                document.paints().replays()
            );
            document.clear(&mut pool);
            drop(pool);
            wait(&ctx)?;
        }
        println!(
            "budget     open a 4K project of 30 layers < 5 s on the RTX 3080 Ti, < 10 s on the RTX A2000"
        );
        if let Some(out) = &self.out {
            println!("written    {}", out.display());
        }
        Ok(())
    }
}
