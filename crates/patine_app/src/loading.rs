//! Startup under a loading page.
//!
//! Building the GPU objects of the application takes from one second to a minute: most
//! of it compiles shaders, slowly with FXC (DirectX 12 without dxcompiler.dll) or the
//! software renderer. A worker thread builds them while the main thread draws this page,
//! so that the window is never blank and keeps answering. The interface shows once
//! everything is ready: the views, the compositor, the thumbnails of the library (one
//! shader each), the environment and the first sync of the startup project.

use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Instant;

use anyhow::Context as _;
use egui::RichText;
use fluent_bundle::FluentArgs;
use patine_compositor::cache::RasterCache;
use patine_compositor::compositor::Compositor;
use patine_compositor::gpu_document::GpuDocument;
use patine_core::environment::EnvironmentSource;
use patine_core::project::Project;
use patine_core::resource::{Resource, ResourceHash};
use patine_gpu::GpuContext;
use patine_gpu::pool::TexturePool;
use patine_render::mesh_gpu::GpuMesh;
use patine_render::uv_view::UvView;
use patine_render::viewport::Viewport;

use crate::i18n::I18n;

/// A step of the startup, named by the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Connecting to the GPU, before the worker starts.
    Gpu,
    /// Pipelines of the 3D view.
    Viewport,
    /// Pipelines of the 2D view.
    View2d,
    /// Pipelines of the layer compositor.
    Compositor,
    /// Shaders and thumbnails of the library.
    Materials,
    /// Decoding and filtering the environment.
    Environment,
    /// Mesh upload and first evaluation of the startup project.
    Project,
}

impl Step {
    /// Translation key of the step.
    pub fn key(self) -> &'static str {
        match self {
            Step::Gpu => "loading-gpu",
            Step::Viewport => "loading-viewport",
            Step::View2d => "loading-view-2d",
            Step::Compositor => "loading-compositor",
            Step::Materials => "loading-materials",
            Step::Environment => "loading-environment",
            Step::Project => "loading-project",
        }
    }
}

/// Translation keys of the page, for tests.
pub const TRANSLATION_KEYS: [&str; 7] = [
    "loading-gpu",
    "loading-viewport",
    "loading-view-2d",
    "loading-compositor",
    "loading-materials",
    "loading-environment",
    "loading-project",
];

/// Where the startup is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    /// Step under way.
    pub step: Step,
    /// Steps done, out of `total`.
    pub done: u32,
    /// Number of steps.
    pub total: u32,
    /// Item of the step under way and number of items (thumbnails).
    pub item: Option<(u32, u32)>,
}

impl Progress {
    /// Fraction done, for the progress bar.
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.done as f32 / self.total as f32
        }
    }
}

/// What the worker builds from.
pub struct Job {
    /// Handle to the device of the window.
    pub gpu: GpuContext,
    /// Budget of the texture pool.
    pub pool_budget: u64,
    /// Project opened at startup.
    pub project: Option<Project>,
    /// Its raster caches, given to its first sync.
    pub caches: Option<Vec<(u64, RasterCache)>>,
    /// Environment to show, with the resources it may come from; `None` keeps the
    /// studio sky the viewport starts with.
    pub environment: Option<(EnvironmentSource, BTreeMap<ResourceHash, Resource>)>,
    /// When the window was created, for the step times in the log.
    pub started: Instant,
}

/// What the worker built.
pub struct Prepared {
    /// The 3D view.
    pub viewport: Viewport,
    /// The 2D view.
    pub uv_view: UvView,
    /// The layer compositor.
    pub compositor: Compositor,
    /// The startup project's mesh on the GPU.
    pub gpu_mesh: Option<GpuMesh>,
    /// The texture pool, holding the startup project's textures.
    pub pool: TexturePool,
    /// The startup project's channel textures.
    pub gpu_document: GpuDocument,
    /// RGBA8 thumbnails of the library, by procedural id.
    pub thumbnails: Vec<(String, Vec<u8>)>,
    /// The environment the viewport shows, or why it does not.
    pub environment: Option<(EnvironmentSource, Result<(), String>)>,
}

enum Message {
    Progress(Progress),
    Done(anyhow::Result<Box<Prepared>>),
}

/// Sends the progress of the worker.
struct Reporter {
    sender: Sender<Message>,
    ctx: egui::Context,
    started: Instant,
    progress: Progress,
}

impl Reporter {
    /// Announces `step` (and its item) as under way.
    fn start(&mut self, step: Step, item: Option<(u32, u32)>) {
        self.progress.step = step;
        self.progress.item = item;
        // The page may be gone (window closed): nobody listens then.
        let _ = self.sender.send(Message::Progress(self.progress));
        self.ctx.request_repaint();
    }

    /// Counts the step under way as done.
    fn finish(&mut self) {
        self.progress.done += 1;
    }

    fn log(&self, name: &str) {
        tracing::info!(
            elapsed_ms = self.started.elapsed().as_millis() as u64,
            "startup: {name}"
        );
    }
}

/// The startup in progress: the worker and what it reported last.
pub struct Loader {
    receiver: Receiver<Message>,
    /// Last reported progress.
    pub progress: Progress,
}

impl Loader {
    /// Starts the worker on `job`.
    pub fn start(job: Job, ctx: &egui::Context) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        let steps = 3
            + patine_procedural::library::all().len() as u32
            + u32::from(job.environment.is_some())
            + u32::from(job.project.is_some());
        let progress = Progress {
            step: Step::Gpu,
            done: 0,
            total: steps,
            item: None,
        };
        let mut reporter = Reporter {
            sender: sender.clone(),
            ctx: ctx.clone(),
            started: job.started,
            progress,
        };
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("startup".into())
            .spawn(move || {
                let result = prepare(job, &mut reporter).map(Box::new);
                let _ = sender.send(Message::Done(result));
                ctx.request_repaint();
            })
            .expect("the system can start a thread");
        Self { receiver, progress }
    }

    /// Takes the reports of the worker; returns its result once it is done.
    pub fn poll(&mut self) -> Option<anyhow::Result<Box<Prepared>>> {
        loop {
            match self.receiver.try_recv() {
                Ok(Message::Progress(progress)) => self.progress = progress,
                Ok(Message::Done(result)) => return Some(result),
                Err(TryRecvError::Empty) => return None,
                // A panic of the worker, logged by the panic hook.
                Err(TryRecvError::Disconnected) => {
                    return Some(Err(anyhow::anyhow!("the startup thread stopped")));
                }
            }
        }
    }
}

/// Builds everything the interface needs, reporting each step.
fn prepare(job: Job, reporter: &mut Reporter) -> anyhow::Result<Prepared> {
    let gpu = &job.gpu;
    reporter.start(Step::Viewport, None);
    let viewport = Viewport::new(gpu).context("failed to create the viewport")?;
    reporter.finish();
    reporter.log("viewport ready");

    reporter.start(Step::View2d, None);
    let uv_view = UvView::new(gpu).context("failed to create the 2D view")?;
    reporter.finish();

    reporter.start(Step::Compositor, None);
    let compositor = Compositor::new(gpu).context("failed to create the compositor")?;
    reporter.finish();
    reporter.log("compositor ready");

    let defs = patine_procedural::library::all();
    let mut thumbnails = Vec::with_capacity(defs.len());
    for (index, def) in defs.iter().enumerate() {
        reporter.start(Step::Materials, Some((index as u32 + 1, defs.len() as u32)));
        let pixels = crate::library_panel::render_thumbnail(gpu, &compositor, def);
        thumbnails.push((def.id.clone(), pixels));
        reporter.finish();
    }
    reporter.log("library thumbnails ready");

    let mut viewport = viewport;
    let environment = job.environment.map(|(source, resources)| {
        reporter.start(Step::Environment, None);
        let start = Instant::now();
        let result = crate::environment_sync::decode(&source, &resources)().and_then(|image| {
            viewport
                .set_environment(gpu, &image)
                .map_err(|error| error.to_string())
        });
        match &result {
            Ok(()) => tracing::info!(
                ?source,
                ms = start.elapsed().as_millis() as u64,
                "environment processed"
            ),
            Err(message) => tracing::error!(?source, %message, "environment failed"),
        }
        reporter.finish();
        (source, result)
    });

    let mut pool = TexturePool::new(job.pool_budget);
    let mut gpu_document = GpuDocument::new();
    let mut gpu_mesh = None;
    if let Some(project) = &job.project {
        reporter.start(Step::Project, None);
        gpu_mesh = crate::app::upload_mesh(gpu, &viewport, &project.mesh);
        // The first sync of an opened project uses its raster caches.
        if let Some(caches) = job.caches {
            gpu_document.provide_caches(caches);
        }
        let start = Instant::now();
        gpu_document.sync(gpu, &mut pool, &compositor, project);
        let paints = gpu_document.paints();
        tracing::info!(
            from_caches = paints.cache_loads(),
            strokes_replayed = paints.replays(),
            ms = start.elapsed().as_millis() as u64,
            "paint layers ready"
        );
        reporter.finish();
    }
    reporter.log("ready");

    Ok(Prepared {
        viewport,
        uv_view,
        compositor,
        gpu_mesh,
        pool,
        gpu_document,
        thumbnails,
        environment,
    })
}

/// Draws the loading page: name, version, progress bar, step under way and GPU.
pub fn show(ui: &mut egui::Ui, i18n: &I18n, progress: &Progress, gpu: &str) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.vertical_centered(|ui| {
            // Roughly centered: the block is about 170 points high.
            ui.add_space(((ui.available_height() - 170.0) / 2.0).max(0.0));
            ui.label(RichText::new(i18n.tr("app-title")).size(44.0).strong());
            ui.label(RichText::new(env!("CARGO_PKG_VERSION")).weak());
            ui.add_space(28.0);
            ui.add(
                egui::ProgressBar::new(progress.fraction())
                    .desired_width(360.0)
                    .animate(true),
            );
            ui.add_space(10.0);
            let mut args = FluentArgs::new();
            if let Some((item, count)) = progress.item {
                args.set("item", item);
                args.set("count", count);
            }
            ui.label(i18n.tr_args(progress.step.key(), &args));
            ui.add_space(18.0);
            ui.label(RichText::new(gpu).small().weak());
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn steps_are_translated() {
        for language in Language::ALL {
            let i18n = I18n::new(language);
            for key in TRANSLATION_KEYS {
                assert!(i18n.has(key), "{language:?}: {key}");
            }
        }
        let steps = [
            Step::Gpu,
            Step::Viewport,
            Step::View2d,
            Step::Compositor,
            Step::Materials,
            Step::Environment,
            Step::Project,
        ];
        for step in steps {
            assert!(TRANSLATION_KEYS.contains(&step.key()), "{step:?}");
        }
    }

    #[test]
    fn progress_fraction() {
        let mut progress = Progress {
            step: Step::Materials,
            done: 0,
            total: 0,
            item: None,
        };
        assert_eq!(progress.fraction(), 0.0);
        progress.total = 8;
        progress.done = 2;
        assert_eq!(progress.fraction(), 0.25);
    }
}
