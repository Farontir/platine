//! Main window: winit event loop, wgpu surface and egui rendering.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context as _;
use patine_gpu::{AdapterChoice, GpuContext, wgpu};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use patine_compositor::compositor::Compositor;
use patine_compositor::gpu_document::GpuDocument;
use patine_core::channel::{Channel, ChannelDef};
use patine_core::environment::{EnvironmentSettings, EnvironmentSource};
use patine_core::ids::LayerId;
use patine_core::project::{Project, ProjectSettings};
use patine_core::resource::ResourceHash;
use patine_gpu::pool::TexturePool;
use patine_mesh::BuildOptions;
use patine_render::camera::{OrbitCamera, Projection};
use patine_render::material::{GpuMaterial, InspectKind, MaterialTextures, Slot};
use patine_render::mesh_gpu::GpuMesh;
use patine_render::settings::DisplayMode;
use patine_render::uv_view::{UvScene, UvView};
use patine_render::viewport::{Scene, Viewport};

use crate::actions::{ActionContext, ActionId, ActionRegistry};
use crate::autosave::{self, Autosaver, Orphan, Session};
use crate::cli::{StartupDialog, StartupEdit, StartupOptions};
use crate::display_panel::EnvironmentView;
use crate::document::{Document, DraftEnvironment, EnvironmentChange, environment_command};
use crate::environment_sync::EnvironmentSync;
use crate::history_panel;
use crate::i18n::{I18n, Language};
use crate::logging::LogBuffer;
use crate::new_project::{CreatedProject, NewProjectDialog};
use crate::profiling::Profiler;
use crate::project_io::{self, FileJobs, JobResult};
use crate::ui::{self, FrameInfo, UiState, UnsavedChoice};
use crate::viewport_panel::PaintEvent;

/// Everything that only exists while the window is open.
struct Graphics {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: GpuContext,
    gpu_name: String,
    gpu_backend: String,
    egui_state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    viewport: Viewport,
    /// GPU copy of the project mesh.
    gpu_mesh: Option<GpuMesh>,
    /// What the last pick request looked at (pixel, image size, camera): a new request is
    /// only sent when it changes, so hovering does not redraw continuously.
    last_pick: Option<([u32; 2], [u32; 2], OrbitCamera)>,
    /// Pool of the document's GPU textures.
    pool: TexturePool,
    /// Final channel textures of the document.
    gpu_document: GpuDocument,
    /// Blend passes evaluating the layer stacks.
    compositor: Compositor,
    /// Material of each texture set, by material index.
    materials: Vec<GpuMaterial>,
    /// What the materials were built to inspect.
    materials_inspect: Option<Inspect>,
    /// Shown as the mask of a layer without mask: 1×1 white.
    white_mask: wgpu::TextureView,
    /// Renderer of the 2D view.
    uv_view: UvView,
}

/// Budget of the texture pool. The document's channel textures are pinned; the budget
/// only bounds discardable caches (M5 onwards).
const POOL_BUDGET: u64 = 8 << 30;

/// What the unlit display modes show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Inspect {
    /// A channel of every texture set.
    Channel(Channel),
    /// The mask of the selected layer, if any, on its texture set.
    Mask(Option<LayerId>),
}

/// What the display mode `display` inspects.
fn inspect_of(
    display: DisplayMode,
    channel: Channel,
    selected: Option<LayerId>,
) -> Option<Inspect> {
    match display {
        DisplayMode::Channel => Some(Inspect::Channel(channel)),
        DisplayMode::Mask => Some(Inspect::Mask(selected)),
        DisplayMode::Lit | DisplayMode::UvChecker => None,
    }
}

/// The texture shown for `inspect` on texture set `set`: a channel, or the mask of the
/// selected layer when it belongs to `set` (white when it has no mask).
fn inspected<'a>(
    inspect: Inspect,
    set: &patine_core::project::TextureSet,
    document: &'a GpuDocument,
    white_mask: &'a wgpu::TextureView,
) -> Option<(&'a wgpu::TextureView, InspectKind)> {
    match inspect {
        Inspect::Channel(channel) => document
            .texture_set(set.id)
            .and_then(|gpu| gpu.channel(channel))
            .map(|texture| (&texture.view, InspectKind::of(channel))),
        Inspect::Mask(Some(layer)) if set.stack.find(layer).is_some() => Some((
            document.mask(layer).unwrap_or(white_mask),
            InspectKind::Scalar,
        )),
        Inspect::Mask(_) => None,
    }
}

/// A 1×1 `R16Float` texture of value 1.
fn white_pixel(gpu: &GpuContext) -> wgpu::TextureView {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("white mask"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue.write_texture(
        texture.as_image_copy(),
        // 1.0 in f16 (sign 0, exponent 15, mantissa 0), little-endian.
        &0x3C00u16.to_le_bytes(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: None,
            rows_per_image: None,
        },
        texture.size(),
    );
    texture.create_view(&Default::default())
}

/// Binds the channel textures of every texture set, by material index. `inspect` is what
/// the unlit display modes show.
fn build_materials(
    gpu: &GpuContext,
    viewport: &Viewport,
    document: &GpuDocument,
    project: &Project,
    inspect: Option<Inspect>,
    white_mask: &wgpu::TextureView,
) -> Vec<GpuMaterial> {
    (0..project.mesh.materials.len() as u32)
        .map(|material| {
            let set = project
                .texture_sets
                .iter()
                .find(|set| set.material_index == material);
            let mut textures = MaterialTextures::default();
            if let Some((set, gpu_set)) =
                set.and_then(|set| document.texture_set(set.id).map(|gpu| (set, gpu)))
            {
                for texture in &gpu_set.channels {
                    if let Some(slot) = Slot::of(texture.def.channel) {
                        textures = textures.with(slot, &texture.view);
                    }
                }
                // Shading uses the normal with the height relief.
                if let Some(normal) = gpu_set.shading_normal() {
                    textures = textures.with(Slot::Normal, normal);
                }
                textures.inspect =
                    inspect.and_then(|inspect| inspected(inspect, set, document, white_mask));
            }
            viewport.material(gpu, &textures)
        })
        .collect()
}

/// The command setting parameter `name` of the selected procedural to `value` (parsed
/// after the parameter's type): the selected mask effect's, or the selected layer's.
fn startup_param(
    project: &Project,
    layers: &crate::layers_panel::LayersState,
    name: &str,
    value: &str,
) -> Option<Box<dyn patine_core::command::Command>> {
    use patine_core::layer::{FillSource, LayerKind};
    use patine_core::mask::{MaskEffect, ParamValue};
    use patine_procedural::def::ParamKind;

    let layer = layers.selected.and_then(|id| {
        project
            .texture_sets
            .iter()
            .find_map(|set| set.stack.find(id))
    })?;
    let parse = |instance: &patine_core::layer::ProceduralRef| {
        let def = patine_procedural::library::find(&instance.id)?;
        let parsed = match &def.param(name)?.kind {
            ParamKind::Float { .. } => ParamValue::Float(value.parse().ok()?),
            ParamKind::Int { .. } | ParamKind::Seed => ParamValue::Int(value.parse().ok()?),
            ParamKind::Bool { .. } => ParamValue::Bool(value.parse().ok()?),
            ParamKind::Color { .. } => {
                let c: Vec<f32> = value
                    .split(',')
                    .map(|v| v.parse().ok())
                    .collect::<Option<_>>()?;
                let [r, g, b] = c.as_slice() else {
                    return None;
                };
                ParamValue::Color([*r, *g, *b, 1.0])
            }
            ParamKind::Enum { .. } => ParamValue::Enum(value.to_owned()),
            _ => return None,
        };
        let mut changed = instance.clone();
        changed.params.insert(name.to_owned(), parsed);
        Some(changed)
    };
    if let crate::masks::LayerPart::MaskEffect(effect) = layers.part {
        let mask = layer.mask.as_ref()?;
        let mut entry = mask.effects[mask.index_of(effect)?].clone();
        let MaskEffect::Fill {
            source: FillSource::Procedural(instance),
        } = &entry.effect
        else {
            return None;
        };
        entry.effect = MaskEffect::Fill {
            source: FillSource::Procedural(parse(instance)?),
        };
        return Some(Box::new(patine_core::commands::SetMaskEffect::new(
            layer.id,
            entry,
            layer.name.clone(),
        )));
    }
    let LayerKind::Fill {
        source: FillSource::Procedural(instance),
        ..
    } = &layer.kind
    else {
        return None;
    };
    Some(Box::new(patine_core::commands::SetFillSource::new(
        layer.id,
        FillSource::Procedural(parse(instance)?),
        layer.name.clone(),
    )))
}

/// What to do once the user has decided about unsaved changes.
#[derive(Clone, Debug, PartialEq, Eq)]
enum PendingAction {
    /// Open the New Project dialog.
    NewProject,
    /// Open a project file (picked in a dialog when `None`).
    Open(Option<PathBuf>),
    /// Quit the application.
    Quit,
}

/// A stroke being painted: its sampler, when it started, where it goes, the image of
/// its alpha and the name of its layer (for the history).
struct StrokeInProgress {
    sampler: patine_paint::sampler::StrokeSampler,
    start: Instant,
    target: patine_core::commands::PaintTarget,
    alpha: Option<(ResourceHash, patine_core::resource::Resource)>,
    name: String,
}

/// Application state.
pub struct App {
    logs: LogBuffer,
    profiler: Profiler,
    i18n: I18n,
    actions: ActionRegistry,
    ui: UiState,
    /// The open document. Replaced as a whole by New Project; edited through commands.
    document: Option<Document>,
    /// Environment used without a project, and given to the next new project.
    draft_environment: DraftEnvironment,
    /// Decoding of the environment shown by the viewport.
    environment_sync: EnvironmentSync,
    egui_ctx: egui::Context,
    graphics: Option<Graphics>,
    /// First fatal error; the event loop exits and `run` returns it.
    error: Option<anyhow::Error>,
    /// Command line options (window size, forced hover, camera angles).
    startup: StartupOptions,
    /// Window capture in progress: PNG path and frames left before it.
    capture: Option<(PathBuf, u32)>,
    /// Project file being opened or saved.
    file_jobs: FileJobs,
    /// Action waiting for the answer about unsaved changes.
    pending: Option<PendingAction>,
    /// Action to run once the running save succeeds.
    after_save: Option<PendingAction>,
    /// Set when the application must exit at the end of the frame.
    quit_requested: bool,
    /// Title currently shown by the window.
    title: String,
    /// Autosave of the open document; `None` in capture mode or without a data folder.
    autosaver: Option<Autosaver>,
    /// Incremented whenever the document is replaced, so that a late autosave of the
    /// previous document is not taken for one of the new document.
    document_generation: u64,
    /// Projects left by crashed sessions, still to propose for recovery.
    orphans: Vec<Orphan>,
    /// The stroke being painted.
    stroke: Option<StrokeInProgress>,
}

/// Opens the main window and runs until it is closed.
///
/// `options` come from the command line: an optional mesh opened as a project with default
/// settings, and developer options setting the view up (see [`crate::cli`]).
pub fn run(logs: LogBuffer, profiler: Profiler, options: StartupOptions) -> anyhow::Result<()> {
    let event_loop = EventLoop::new().context("failed to create the event loop")?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let egui_ctx = egui::Context::default();
    ui::setup_style(&egui_ctx);
    let mut app = App {
        logs,
        profiler,
        i18n: I18n::new(options.language.unwrap_or_else(Language::system)),
        actions: ActionRegistry::default(),
        ui: UiState::default(),
        document: None,
        draft_environment: DraftEnvironment::default(),
        environment_sync: EnvironmentSync::default(),
        egui_ctx,
        graphics: None,
        error: None,
        capture: options
            .capture
            .clone()
            .map(|path| (path, options.capture_frames)),
        startup: options,
        file_jobs: FileJobs::default(),
        pending: None,
        after_save: None,
        quit_requested: false,
        title: String::new(),
        autosaver: None,
        document_generation: 0,
        orphans: Vec::new(),
        stroke: None,
    };
    // Captures show the application as configured, without recovery prompts, unless an
    // autosave folder is given explicitly (to capture the recovery dialog).
    if app.capture.is_none() || std::env::var_os("PATINE_AUTOSAVE_DIR").is_some() {
        app.start_autosave();
    }
    app.apply_startup_environment()?;
    match app.startup.mesh.clone() {
        Some(path) if is_project_file(&path) => {
            let project = patine_core::file::load(&path)
                .with_context(|| format!("cannot open {}", path.display()))?;
            tracing::info!(path = %path.display(), "project opened");
            app.replace_document(Document::opened(project, path));
        }
        Some(path) => {
            let imported = patine_mesh::import(&path, BuildOptions::default())
                .with_context(|| format!("cannot import {}", path.display()))?;
            app.open_project(CreatedProject {
                project: Project::new(imported.mesh, path, ProjectSettings::default()),
                report: imported.report,
            });
        }
        None => {}
    }
    app.apply_startup_options()?;
    app.propose_recovery();
    event_loop.run_app(&mut app).context("event loop failed")?;
    // A clean exit: the autosave session is no longer needed.
    if let Some(autosaver) = app.autosaver.take() {
        autosaver.finish();
    }
    match app.error.take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether `path` names a project file rather than a mesh.
fn is_project_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(patine_core::file::EXTENSION))
}

/// Saves the surface texture `texture` (8-bit RGBA or BGRA) to a PNG file.
fn save_capture(gpu: &GpuContext, texture: &wgpu::Texture, path: &Path) -> anyhow::Result<()> {
    let mut pixels = patine_gpu::readback::read_texture(gpu, texture)?;
    let bgra = matches!(
        texture.format(),
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    );
    for pixel in pixels.as_chunks_mut::<4>().0 {
        if bgra {
            pixel.swap(0, 2);
        }
        pixel[3] = 255;
    }
    let image = image::RgbaImage::from_raw(texture.width(), texture.height(), pixels)
        .context("capture size matches the texture")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    image.save(path)?;
    tracing::info!(path = %path.display(), "window captured");
    Ok(())
}

impl App {
    /// Applies the environment options of the command line to the draft environment,
    /// which the startup project then takes.
    fn apply_startup_environment(&mut self) -> anyhow::Result<()> {
        let options = self.startup.clone();
        match &options.hdri {
            Some(Some(path)) => {
                let picked = crate::display_panel::read_hdri(path).map_err(|message| {
                    anyhow::anyhow!("cannot load {}: {message}", path.display())
                })?;
                let hash = ResourceHash::of(&picked.bytes);
                self.environment_sync.preload(
                    &self.egui_ctx,
                    EnvironmentSource::Resource(hash),
                    picked.image,
                );
                self.draft_environment.apply(EnvironmentChange::Imported {
                    name: picked.name,
                    bytes: picked.bytes,
                });
            }
            Some(None) => self
                .draft_environment
                .apply(EnvironmentChange::Source(EnvironmentSource::Studio)),
            None => {}
        }
        if let Some(rotation) = options.rotation {
            self.draft_environment
                .apply(EnvironmentChange::Rotation(rotation.to_radians()));
        }
        if let Some(exposure) = options.exposure {
            self.draft_environment
                .apply(EnvironmentChange::Exposure(exposure));
        }
        Ok(())
    }

    /// Applies the view options of the command line.
    fn apply_startup_options(&mut self) -> anyhow::Result<()> {
        let options = self.startup.clone();
        if let Some(set) = options.texture_set {
            self.ui.texture_sets.selected = patine_core::project::TextureSetId(set);
        }
        if let (Some((base_color, metallic, roughness)), Some(document)) =
            (options.material, &mut self.document)
        {
            let [r, g, b] = base_color;
            let values = [
                (Channel::BaseColor, [r, g, b, 1.0]),
                (Channel::Metallic, [metallic; 4]),
                (Channel::Roughness, [roughness; 4]),
            ];
            let sets: Vec<_> = document.project.texture_sets.iter().map(|s| s.id).collect();
            for set in sets {
                for (channel, value) in values {
                    let mut def = ChannelDef::standard(channel);
                    def.default = value;
                    document.execute(Box::new(patine_core::commands::SetChannelDef::set(
                        set, def,
                    )));
                    document.history.seal();
                }
            }
        }
        if let Some(document) = &mut self.document {
            for edit in &options.edits {
                let command: Box<dyn patine_core::command::Command> = match edit {
                    StartupEdit::Rotation(degrees) => environment_command(
                        &document.project.environment,
                        EnvironmentChange::Rotation(degrees.to_radians()),
                    ),
                    StartupEdit::Exposure(ev) => environment_command(
                        &document.project.environment,
                        EnvironmentChange::Exposure(*ev),
                    ),
                    StartupEdit::RenameTextureSet(set, name) => {
                        Box::new(patine_core::commands::RenameTextureSet::new(
                            patine_core::project::TextureSetId(*set),
                            name.clone(),
                        ))
                    }
                    StartupEdit::Undo(count) => {
                        for _ in 0..*count {
                            document.undo();
                        }
                        continue;
                    }
                    StartupEdit::Fill(rgb, roughness) => {
                        let set_id = self.ui.texture_sets.selected;
                        let Some(set) = document.project.texture_set(set_id) else {
                            continue;
                        };
                        let insert = crate::layers_panel::new_fill(
                            &self.i18n,
                            &document.project,
                            set,
                            self.ui.layers.selected,
                        );
                        let id = patine_core::ids::LayerId(document.project.next_id);
                        document.execute(Box::new(insert));
                        document.history.seal();
                        let name = document
                            .project
                            .texture_set(set_id)
                            .and_then(|set| set.stack.find(id))
                            .map(|layer| layer.name.clone())
                            .unwrap_or_default();
                        self.ui.layers.selected = Some(id);
                        document.execute(Box::new(patine_core::commands::SetFillValue::new(
                            id,
                            Channel::Roughness,
                            Some([*roughness; 4]),
                            name.clone(),
                        )));
                        document.history.seal();
                        let [r, g, b] = *rgb;
                        Box::new(patine_core::commands::SetFillValue::new(
                            id,
                            Channel::BaseColor,
                            Some([r, g, b, 1.0]),
                            name,
                        ))
                    }
                    StartupEdit::Mode(name, opacity) => {
                        let Some(id) = self.ui.layers.selected else {
                            continue;
                        };
                        let layer_name = document
                            .project
                            .texture_set(self.ui.texture_sets.selected)
                            .and_then(|set| set.stack.find(id))
                            .map(|layer| layer.name.clone())
                            .unwrap_or_default();
                        let Some(mode) =
                            patine_compositor::reference::ALL_MODES
                                .into_iter()
                                .find(|mode| {
                                    crate::layers_panel::mode_key(*mode) == format!("blend-{name}")
                                })
                        else {
                            anyhow::bail!("unknown blend mode {name}");
                        };
                        Box::new(patine_core::commands::SetChannelBlend::new(
                            id,
                            Channel::BaseColor,
                            Some(patine_core::layer::ChannelBlend {
                                mode,
                                opacity: *opacity,
                            }),
                            layer_name,
                        ))
                    }
                    StartupEdit::Group => {
                        let (Some(id), Some(set)) = (
                            self.ui.layers.selected,
                            document.project.texture_set(self.ui.texture_sets.selected),
                        ) else {
                            continue;
                        };
                        match crate::layers_panel::group(&self.i18n, &document.project, set, id) {
                            Some(group) => Box::new(group),
                            None => continue,
                        }
                    }
                    StartupEdit::Material(id) => {
                        let Some(def) = patine_procedural::library::find(id) else {
                            anyhow::bail!("unknown material {id}");
                        };
                        let Some(insert) = crate::library_panel::new_material_layer(
                            &self.i18n,
                            &document.project,
                            self.ui.texture_sets.selected,
                            self.ui.layers.selected,
                            def,
                        ) else {
                            continue;
                        };
                        self.ui.layers.selected =
                            Some(patine_core::ids::LayerId(document.project.next_id));
                        self.ui.layers.part = crate::masks::LayerPart::Content;
                        Box::new(insert)
                    }
                    StartupEdit::MaskProcedural(id) => {
                        let Some(def) = patine_procedural::library::find(id) else {
                            anyhow::bail!("unknown procedural {id}");
                        };
                        let Some(insert) = crate::library_panel::new_mask_procedural(
                            &document.project,
                            &self.ui.layers,
                            def,
                        ) else {
                            continue;
                        };
                        self.ui.layers.part = crate::masks::LayerPart::MaskEffect(
                            patine_core::ids::EffectId(document.project.next_id),
                        );
                        Box::new(insert)
                    }
                    StartupEdit::Param(name, value) => {
                        match startup_param(&document.project, &self.ui.layers, name, value) {
                            Some(command) => command,
                            None => anyhow::bail!("cannot set parameter {name} to {value}"),
                        }
                    }
                    StartupEdit::Projection(kind, scale) => {
                        let Some(layer) = self.ui.layers.selected.and_then(|id| {
                            document
                                .project
                                .texture_sets
                                .iter()
                                .find_map(|set| set.stack.find(id))
                        }) else {
                            continue;
                        };
                        let kind = match kind.as_str() {
                            "uv" => patine_core::layer::ProjectionKind::Uv,
                            "triplanar" => {
                                patine_core::layer::ProjectionKind::Triplanar { blend: 0.5 }
                            }
                            "planar" => patine_core::layer::ProjectionKind::Planar,
                            "spherical" => patine_core::layer::ProjectionKind::Spherical,
                            "cylindrical" => patine_core::layer::ProjectionKind::Cylindrical,
                            other => anyhow::bail!("unknown projection {other}"),
                        };
                        Box::new(patine_core::commands::SetProjection::new(
                            layer.id,
                            patine_core::layer::Projection {
                                kind,
                                scale: [*scale; 2],
                                ..patine_core::layer::Projection::default()
                            },
                            layer.name.clone(),
                        ))
                    }
                    StartupEdit::PaintLayer => {
                        let Some(set) = document.project.texture_set(self.ui.texture_sets.selected)
                        else {
                            continue;
                        };
                        let insert = crate::layers_panel::new_paint(
                            &self.i18n,
                            &document.project,
                            set,
                            self.ui.layers.selected,
                        );
                        self.ui.layers.selected = Some(LayerId(document.project.next_id));
                        self.ui.layers.part = crate::masks::LayerPart::Content;
                        Box::new(insert)
                    }
                    StartupEdit::MaskPaint => {
                        let Some(layer) = self.ui.layers.selected.and_then(|id| {
                            document
                                .project
                                .texture_sets
                                .iter()
                                .find_map(|set| set.stack.find(id))
                        }) else {
                            continue;
                        };
                        let Some(insert) = crate::masks::insert_effect(
                            &document.project,
                            layer,
                            patine_core::mask::MaskEffect::paint(),
                            patine_core::layer::BlendMode::Normal,
                            1.0,
                            None,
                        ) else {
                            continue;
                        };
                        self.ui.layers.part = crate::masks::LayerPart::MaskEffect(
                            patine_core::ids::EffectId(document.project.next_id),
                        );
                        Box::new(insert)
                    }
                    StartupEdit::Mask(white) => {
                        let Some(layer) = self.ui.layers.selected.and_then(|id| {
                            document
                                .project
                                .texture_sets
                                .iter()
                                .find_map(|set| set.stack.find(id))
                        }) else {
                            continue;
                        };
                        let initial = if *white {
                            patine_core::mask::MaskInitial::White
                        } else {
                            patine_core::mask::MaskInitial::Black
                        };
                        self.ui.layers.part = crate::masks::LayerPart::Mask;
                        Box::new(crate::masks::add_mask(layer, initial))
                    }
                    StartupEdit::MaskFill(..)
                    | StartupEdit::MaskLevels(_)
                    | StartupEdit::MaskInvert => {
                        let Some(layer) = self.ui.layers.selected.and_then(|id| {
                            document
                                .project
                                .texture_sets
                                .iter()
                                .find_map(|set| set.stack.find(id))
                        }) else {
                            continue;
                        };
                        let (effect, mode, opacity) = match edit {
                            StartupEdit::MaskFill(value, name, opacity) => {
                                let Some(mode) = patine_compositor::reference::ALL_MODES
                                    .into_iter()
                                    .find(|mode| {
                                        crate::layers_panel::mode_key(*mode)
                                            == format!("blend-{name}")
                                    })
                                else {
                                    anyhow::bail!("unknown blend mode {name}");
                                };
                                (
                                    patine_core::mask::MaskEffect::uniform(*value),
                                    mode,
                                    *opacity,
                                )
                            }
                            StartupEdit::MaskLevels(
                                [in_low, in_high, gamma, out_low, out_high],
                            ) => (
                                patine_core::mask::MaskEffect::Levels(
                                    patine_core::mask::LevelsParams {
                                        in_low: *in_low,
                                        in_high: *in_high,
                                        gamma: *gamma,
                                        out_low: *out_low,
                                        out_high: *out_high,
                                    },
                                ),
                                patine_core::layer::BlendMode::Normal,
                                1.0,
                            ),
                            _ => (
                                patine_core::mask::MaskEffect::invert(),
                                patine_core::layer::BlendMode::Normal,
                                1.0,
                            ),
                        };
                        let above = match self.ui.layers.part {
                            crate::masks::LayerPart::MaskEffect(effect) => Some(effect),
                            _ => None,
                        };
                        let Some(insert) = crate::masks::insert_effect(
                            &document.project,
                            layer,
                            effect,
                            mode,
                            opacity,
                            above,
                        ) else {
                            continue;
                        };
                        self.ui.layers.part = crate::masks::LayerPart::MaskEffect(
                            patine_core::ids::EffectId(document.project.next_id),
                        );
                        Box::new(insert)
                    }
                };
                document.execute(command);
                document.history.seal();
            }
        }
        if let Some(eraser) = options.eraser {
            self.ui.tool.tool = if eraser {
                crate::paint_tool::Tool::Eraser
            } else {
                crate::paint_tool::Tool::Brush
            };
        }
        for option in &options.brush {
            use crate::cli::BrushOption;
            let tool = &mut self.ui.tool;
            match option {
                BrushOption::Size(v) => tool.size = *v,
                BrushOption::Flow(v) => tool.flow = *v,
                BrushOption::Opacity(v) => tool.opacity = *v,
                BrushOption::Hardness(v) => tool.hardness = *v,
                BrushOption::Spacing(v) => tool.spacing = *v,
                BrushOption::Angle(v) => tool.angle = v.to_radians(),
                BrushOption::Smoothing(v) => tool.smoothing = *v,
                BrushOption::MaskValue(v) => tool.mask_value = *v,
                BrushOption::Alpha(id) => {
                    tool.alpha = match id {
                        None => None,
                        Some(id) => match patine_paint::alphas::find(id) {
                            Some(alpha) => Some(alpha.id),
                            None => anyhow::bail!("unknown alpha {id}"),
                        },
                    };
                }
                BrushOption::Color([r, g, b]) => tool.set_channel(
                    &ChannelDef::standard(Channel::BaseColor),
                    Some([*r, *g, *b, 1.0]),
                ),
                BrushOption::Scalar(channel, v) => {
                    tool.set_channel(&ChannelDef::standard(*channel), Some([*v; 4]));
                }
                BrushOption::Off(channel) => {
                    tool.set_channel(&ChannelDef::standard(*channel), None);
                }
            }
        }
        for panel in &options.tabs {
            if let Some(path) = self.ui.dock.find_tab(panel) {
                // The path was just found in the same dock state.
                let _ = self.ui.dock.set_active_tab(path);
            }
        }
        let view = &mut self.ui.view;
        let settings = &mut view.settings;
        if let Some(tone_mapper) = options.tone_mapper {
            settings.tone_mapper = tone_mapper;
        }
        if let Some(background) = options.background {
            settings.background = background;
        }
        if let Some((mode, _)) = options.display {
            settings.display = mode;
        }
        settings.wireframe |= options.wireframe;
        settings.shadows &= !options.no_shadows;
        if let Some((_, Some(channel))) = options.display {
            view.inspect_channel = channel;
        }
        if let Some(projection) = options.projection {
            view.camera.projection = projection;
        }
        if let Some(fov) = options.fov {
            view.camera.fov_y = fov.to_radians();
        }
        if let Some([yaw, pitch]) = options.orbit {
            view.camera.yaw = yaw.to_radians();
            view.camera.pitch = pitch.to_radians();
        }
        match options.dialog {
            Some(StartupDialog::About) => self.ui.show_about = true,
            Some(StartupDialog::NewProject) => {
                let mut dialog = match &options.dialog_path {
                    Some(path) => NewProjectDialog::with_path(path),
                    None => NewProjectDialog::default(),
                };
                if options.dialog_create {
                    dialog.start_import(&self.egui_ctx);
                }
                self.ui.new_project = Some(dialog);
            }
            None => {}
        }
        Ok(())
    }

    fn action_context(&self) -> ActionContext {
        let view = &self.ui.view;
        ActionContext {
            profiler_enabled: self.profiler.is_enabled(),
            language: self.i18n.language(),
            has_project: self.document.is_some(),
            busy: self.file_jobs.busy(),
            can_undo: self
                .document
                .as_ref()
                .is_some_and(|document| document.history.can_undo()),
            can_redo: self
                .document
                .as_ref()
                .is_some_and(|document| document.history.can_redo()),
            channel_view: view.settings.display == DisplayMode::Channel,
            mask_view: view.settings.display == DisplayMode::Mask,
            layer_selected: self.ui.layers.selected.is_some() && self.document.is_some(),
            orthographic: view.camera.projection == Projection::Orthographic,
            wireframe: view.settings.wireframe,
            shadows: view.settings.shadows,
            eraser: self.ui.tool.tool == crate::paint_tool::Tool::Eraser,
        }
    }

    fn create_graphics(&self, event_loop: &ActiveEventLoop) -> anyhow::Result<Graphics> {
        let attributes = Window::default_attributes()
            .with_title(self.i18n.tr("app-title"))
            .with_inner_size(match self.startup.window {
                Some([width, height]) => PhysicalSize::new(width, height),
                None => PhysicalSize::new(1600, 900),
            })
            .with_min_inner_size(PhysicalSize::new(640, 360));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .context("failed to create the window")?,
        );

        let instance = GpuContext::create_instance(AdapterChoice::Hardware);
        let surface = instance
            .create_surface(Arc::clone(&window))
            .context("failed to create the window surface")?;
        let gpu = pollster::block_on(GpuContext::request(
            instance,
            AdapterChoice::Hardware,
            Some(&surface),
        ))?;
        let info = gpu.adapter_info();
        tracing::info!(capabilities = ?gpu.capabilities(), "GPU ready");

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&gpu.adapter, size.width.max(1), size.height.max(1))
            .context("the window surface is not supported by the adapter")?;
        // egui blends in gamma space and expects a non-sRGB surface. The 3D viewport
        // shades in linear space and encodes sRGB itself after tone mapping.
        let caps = surface.get_capabilities(&gpu.adapter);
        if let Some(format) = caps.formats.iter().find(|format| !format.is_srgb()) {
            config.format = *format;
        }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        // Window captures copy the presented image.
        if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
            config.usage |= wgpu::TextureUsages::COPY_SRC;
        }
        surface.configure(&gpu.device, &config);

        let max_texture_side = gpu.device.limits().max_texture_dimension_2d as usize;
        let egui_state = egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(max_texture_side),
        );
        let renderer = egui_wgpu::Renderer::new(
            &gpu.device,
            config.format,
            egui_wgpu::RendererOptions::default(),
        );
        let viewport = Viewport::new(&gpu).context("failed to create the viewport")?;
        let uv_view = UvView::new(&gpu).context("failed to create the 2D view")?;
        let compositor = Compositor::new(&gpu).context("failed to create the compositor")?;
        let white_mask = white_pixel(&gpu);
        let gpu_mesh = self
            .document
            .as_ref()
            .and_then(|document| upload_mesh(&gpu, &viewport, &document.project.mesh));

        Ok(Graphics {
            window,
            surface,
            config,
            gpu_name: info.name.clone(),
            gpu_backend: format!("{:?}", info.backend),
            gpu,
            egui_state,
            renderer,
            viewport,
            gpu_mesh,
            last_pick: None,
            pool: TexturePool::new(POOL_BUDGET),
            gpu_document: GpuDocument::new(),
            compositor,
            materials: Vec::new(),
            materials_inspect: None,
            white_mask,
            uv_view,
        })
    }

    /// The environment shown now: the document's, or the draft without a project.
    fn environment(&self) -> &EnvironmentSettings {
        match &self.document {
            Some(document) => &document.project.environment,
            None => &self.draft_environment.settings,
        }
    }

    /// The current environment as a draft, to give to a new project.
    fn environment_draft(&self) -> DraftEnvironment {
        match &self.document {
            Some(document) => {
                let settings = document.project.environment.clone();
                let resources = match &settings.source {
                    EnvironmentSource::Resource(hash) => document
                        .project
                        .resources
                        .get(hash)
                        .map(|resource| (*hash, resource.clone()))
                        .into_iter()
                        .collect(),
                    _ => Default::default(),
                };
                DraftEnvironment {
                    settings,
                    resources,
                }
            }
            None => self.draft_environment.clone(),
        }
    }

    /// Display name of the current environment.
    fn environment_name(&self) -> String {
        let resources = match &self.document {
            Some(document) => &document.project.resources,
            None => &self.draft_environment.resources,
        };
        match &self.environment().source {
            EnvironmentSource::Studio => self.i18n.tr("display-environment-studio").into_owned(),
            EnvironmentSource::Bundled(name) => name.clone(),
            EnvironmentSource::Resource(hash) => resources
                .get(hash)
                .map_or_else(|| hash.to_string(), |resource| resource.name.clone()),
        }
    }

    /// Applies an environment change: a command on the document, or a direct change of
    /// the draft without a project.
    fn change_environment(&mut self, change: EnvironmentChange) {
        match &mut self.document {
            Some(document) => {
                let command = environment_command(&document.project.environment, change);
                document.execute(command);
            }
            None => self.draft_environment.apply(change),
        }
    }

    /// Replaces the document by a new project, which takes the current environment. This
    /// is not an undoable command: the history starts over with the new document.
    fn open_project(&mut self, created: CreatedProject) {
        let CreatedProject {
            mut project,
            report,
        } = created;
        self.environment_draft().install(&mut project);
        let mesh = &project.mesh;
        tracing::info!(
            name = %mesh.name,
            format = report.format.name(),
            triangles = mesh.triangle_count(),
            vertices = mesh.vertex_count(),
            materials = mesh.materials.len(),
            objects = mesh.objects.len(),
            "project created"
        );
        let build = &report.build;
        let mut timings = format!("read {:.0} ms", report.read_time.as_secs_f64() * 1e3);
        for (step, duration) in &build.timings {
            timings.push_str(&format!(", {step} {:.0} ms", duration.as_secs_f64() * 1e3));
        }
        tracing::info!(%timings, "import timings");
        if build.degenerate_removed > 0 {
            tracing::warn!(
                count = build.degenerate_removed,
                "degenerate triangles removed"
            );
        }
        if build.invalid_normals_replaced > 0 {
            tracing::warn!(
                count = build.invalid_normals_replaced,
                "invalid normals replaced"
            );
        }
        if build.uvs_outside_unit_square > 0 {
            tracing::warn!(
                vertices = build.uvs_outside_unit_square,
                "UVs outside the unit square (UDIM tiles are not supported yet)"
            );
        }
        if !build.unused_materials.is_empty() {
            tracing::warn!(materials = ?build.unused_materials, "unused materials dropped");
        }
        for warning in &report.warnings {
            tracing::warn!("{warning}");
        }
        self.replace_document(Document::new(project));
    }

    /// Starts this instance's autosave session, and finds the projects left by crashes.
    fn start_autosave(&mut self) {
        let Some(root) = autosave::default_root() else {
            tracing::warn!("no application data folder: autosave disabled");
            return;
        };
        self.orphans = autosave::orphans(&root);
        let interval = self
            .startup
            .autosave_interval
            .map_or(autosave::DEFAULT_INTERVAL, std::time::Duration::from_secs);
        match Session::start(&root) {
            Ok(session) => self.autosaver = Some(Autosaver::new(session, interval)),
            Err(error) => tracing::warn!(%error, "autosave disabled"),
        }
    }

    /// Shows the recovery dialog for the next project left by a crash, if any.
    fn propose_recovery(&mut self) {
        self.ui.recovery_prompt = self
            .orphans
            .first()
            .map(|orphan| (orphan.info.name.clone(), orphan.age.as_secs() / 60));
    }

    /// Answer of the recovery dialog for the first orphan.
    fn answer_recovery(&mut self, recover: bool) {
        self.ui.recovery_prompt = None;
        if self.orphans.is_empty() {
            return;
        }
        let orphan = self.orphans.remove(0);
        if recover {
            match patine_core::file::load(&orphan.project) {
                Ok(project) => {
                    tracing::info!(name = %orphan.info.name, "project recovered");
                    let mut document = Document::new(project);
                    document.path = orphan.info.original_path.clone();
                    // Not saved anywhere the user knows of.
                    document.history.mark_unsaved();
                    self.replace_document(document);
                    if let Some(autosaver) = &mut self.autosaver {
                        autosaver.due_now();
                    }
                }
                Err(error) => {
                    tracing::error!(%error, "recovery failed");
                    self.ui.error_message = Some(project_io::error_message(
                        &self.i18n,
                        false,
                        &orphan.project,
                        &error,
                    ));
                }
            }
        }
        orphan.discard();
        self.propose_recovery();
    }

    /// Makes `document` the open document: uploads its mesh and frames it.
    fn replace_document(&mut self, document: Document) {
        self.document_generation += 1;
        if let Some(gfx) = self.graphics.as_mut() {
            gfx.gpu_mesh = upload_mesh(&gfx.gpu, &gfx.viewport, &document.project.mesh);
            gfx.last_pick = None;
            gfx.gpu_document.clear(&mut gfx.pool);
            gfx.materials.clear();
        }
        self.ui.view.hovered = None;
        self.ui.view.hovered_point = None;
        self.ui.view.painting = false;
        self.ui.view.paint_events.clear();
        self.stroke = None;
        // Framed at the next render, when the panel size is known.
        self.ui.view.frame_pending = true;
        self.ui.texture_sets = Default::default();
        self.ui.layers = Default::default();
        self.ui.view_2d.framed = false;
        self.document = Some(document);
    }

    /// Runs `action` now, or first asks what to do with unsaved changes.
    fn request(&mut self, action: PendingAction) {
        match &self.document {
            Some(document) if document.history.is_dirty() => {
                self.ui.unsaved_prompt = Some(document.name());
                self.pending = Some(action);
            }
            _ => self.perform(action),
        }
    }

    /// Runs an action that may discard the document.
    fn perform(&mut self, action: PendingAction) {
        match action {
            PendingAction::NewProject => {
                if self.ui.new_project.is_none() {
                    self.ui.new_project = Some(NewProjectDialog::default());
                }
            }
            PendingAction::Open(path) => {
                let filter = self.i18n.tr("file-filter").into_owned();
                self.file_jobs.open(&self.egui_ctx, path, filter);
            }
            PendingAction::Quit => self.quit_requested = true,
        }
    }

    /// Answer of the unsaved changes dialog.
    fn answer_unsaved(&mut self, choice: UnsavedChoice) {
        self.ui.unsaved_prompt = None;
        let Some(action) = self.pending.take() else {
            return;
        };
        match choice {
            UnsavedChoice::Save => {
                self.after_save = Some(action);
                self.save(false);
            }
            UnsavedChoice::Discard => self.perform(action),
            UnsavedChoice::Cancel => {}
        }
    }

    /// Saves the document in the background, to its file or to one picked in a dialog
    /// (always with `choose_path`).
    fn save(&mut self, choose_path: bool) {
        if self.file_jobs.busy() {
            return;
        }
        let thumbnail = self.thumbnail();
        let Some(document) = &mut self.document else {
            self.after_save = None;
            return;
        };
        let state = document.history.begin_save();
        let path = if choose_path {
            None
        } else {
            document.path.clone()
        };
        let filter = self.i18n.tr("file-filter").into_owned();
        self.file_jobs.save(
            &self.egui_ctx,
            document.project.clone(),
            state,
            path,
            thumbnail,
            filter,
            document.name(),
        );
    }

    /// The last viewport image, for the project thumbnail.
    fn thumbnail(&self) -> Option<image::RgbaImage> {
        let gfx = self.graphics.as_ref()?;
        let (texture, _) = gfx.viewport.output()?;
        let pixels = patine_gpu::readback::read_texture(&gfx.gpu, texture).ok()?;
        image::RgbaImage::from_raw(texture.width(), texture.height(), pixels)
    }

    /// Handles a finished open or save.
    fn finish_file_job(&mut self, result: JobResult) {
        match result {
            JobResult::Saved { path, state } => {
                if let Some(document) = &mut self.document {
                    document.path = Some(path);
                    document.history.mark_saved_as(state);
                }
                if let Some(action) = self.after_save.take() {
                    self.perform(action);
                }
            }
            JobResult::Opened { path, project } => {
                tracing::info!(path = %path.display(), "project opened");
                self.replace_document(Document::opened(*project, path));
            }
            JobResult::Failed {
                saving,
                path,
                error,
            } => {
                tracing::error!(path = %path.display(), %error, "project file error");
                self.ui.error_message =
                    Some(project_io::error_message(&self.i18n, saving, &path, &error));
                self.after_save = None;
            }
            JobResult::Cancelled => self.after_save = None,
        }
    }

    /// Window title: application, document name, and `*` when modified.
    fn window_title(&self) -> String {
        let app = self.i18n.tr("app-title");
        match &self.document {
            Some(document) => {
                let modified = if document.history.is_dirty() { "*" } else { "" };
                format!("{}{modified} — {app}", document.name())
            }
            None => app.into_owned(),
        }
    }

    fn execute(&mut self, action: ActionId) {
        tracing::debug!(action = self.actions.get(action).name, "action");
        match action {
            ActionId::NewProject => self.request(PendingAction::NewProject),
            ActionId::Open => self.request(PendingAction::Open(None)),
            ActionId::Save => self.save(false),
            ActionId::SaveAs => self.save(true),
            ActionId::Quit => self.request(PendingAction::Quit),
            ActionId::ResetLayout => self.ui.dock = ui::default_layout(),
            ActionId::ToggleProfiler => {
                let enabled = !self.profiler.is_enabled();
                self.profiler.set_enabled(enabled);
            }
            ActionId::LanguageEnglish => self.i18n = I18n::new(Language::English),
            ActionId::LanguageFrench => self.i18n = I18n::new(Language::French),
            ActionId::About => self.ui.show_about = true,
            ActionId::FrameMesh if self.ui.view_2d.hovered => self.ui.view_2d.frame(),
            ActionId::FrameMesh => {
                if let Some(bounds) = self
                    .document
                    .as_ref()
                    .and_then(|document| document.project.mesh.bounds())
                {
                    let aspect = self.ui.view.aspect();
                    self.ui.view.camera.frame(&bounds, aspect);
                }
            }
            ActionId::ToggleProjection => {
                let camera = &mut self.ui.view.camera;
                camera.projection = match camera.projection {
                    Projection::Perspective => Projection::Orthographic,
                    Projection::Orthographic => Projection::Perspective,
                };
            }
            ActionId::ToggleWireframe => {
                let settings = &mut self.ui.view.settings;
                settings.wireframe = !settings.wireframe;
            }
            ActionId::ToggleShadows => {
                let settings = &mut self.ui.view.settings;
                settings.shadows = !settings.shadows;
            }
            ActionId::ToggleMaskView => {
                let settings = &mut self.ui.view.settings;
                settings.display = if settings.display == DisplayMode::Mask {
                    DisplayMode::Lit
                } else {
                    DisplayMode::Mask
                };
            }
            ActionId::DeleteLayer | ActionId::GroupLayer => {
                let (Some(document), Some(selected)) =
                    (&mut self.document, self.ui.layers.selected)
                else {
                    return;
                };
                let Some(set) = document.project.texture_set(self.ui.texture_sets.selected) else {
                    return;
                };
                let part = self.ui.layers.part;
                let command: Option<Box<dyn patine_core::command::Command>> = match action {
                    ActionId::DeleteLayer => crate::masks::delete_selection(set, selected, part),
                    _ => crate::layers_panel::group(&self.i18n, &document.project, set, selected)
                        .map(|c| Box::new(c) as Box<dyn patine_core::command::Command>),
                };
                if let Some(command) = command {
                    document.history.seal();
                    document.execute(command);
                    document.history.seal();
                    if action == ActionId::DeleteLayer {
                        if part == crate::masks::LayerPart::Content {
                            self.ui.layers.selected = None;
                        }
                        self.ui.layers.part = crate::masks::LayerPart::Content;
                    }
                }
            }
            ActionId::ToggleChannelView => {
                let channels = self.current_channels();
                let view = &mut self.ui.view;
                if view.settings.display == DisplayMode::Channel {
                    view.settings.display = DisplayMode::Lit;
                } else if let Some(first) = channels.first() {
                    if !channels.contains(&view.inspect_channel) {
                        view.inspect_channel = *first;
                    }
                    view.settings.display = DisplayMode::Channel;
                }
            }
            ActionId::NextChannel => {
                let channels = self.current_channels();
                let view = &mut self.ui.view;
                if let Some(first) = channels.first() {
                    view.inspect_channel = if view.settings.display == DisplayMode::Channel {
                        let index = channels.iter().position(|c| *c == view.inspect_channel);
                        index.map_or(*first, |i| channels[(i + 1) % channels.len()])
                    } else if channels.contains(&view.inspect_channel) {
                        view.inspect_channel
                    } else {
                        *first
                    };
                    view.settings.display = DisplayMode::Channel;
                }
            }
            ActionId::NewPaintLayer => {
                let Some(document) = &mut self.document else {
                    return;
                };
                let Some(set) = document.project.texture_set(self.ui.texture_sets.selected) else {
                    return;
                };
                let insert = crate::layers_panel::new_paint(
                    &self.i18n,
                    &document.project,
                    set,
                    self.ui.layers.selected,
                );
                self.ui.layers.selected = Some(LayerId(document.project.next_id));
                self.ui.layers.part = crate::masks::LayerPart::Content;
                document.history.seal();
                document.execute(Box::new(insert));
                document.history.seal();
            }
            ActionId::ToolBrush => self.ui.tool.tool = crate::paint_tool::Tool::Brush,
            ActionId::ToolEraser => self.ui.tool.tool = crate::paint_tool::Tool::Eraser,
            ActionId::BrushSmaller => self.ui.tool.scale_size(1.0 / 1.25),
            ActionId::BrushLarger => self.ui.tool.scale_size(1.25),
            ActionId::BrushSofter => self.ui.tool.add_hardness(-0.1),
            ActionId::BrushHarder => self.ui.tool.add_hardness(0.1),
            ActionId::Undo => {
                if let Some(document) = &mut self.document {
                    document.undo();
                }
            }
            ActionId::Redo => {
                if let Some(document) = &mut self.document {
                    document.redo();
                }
            }
        }
        if let Some(gfx) = &self.graphics {
            gfx.window.request_redraw();
        }
    }

    /// Executes what the interface asked for this frame: a new project, commands and
    /// environment changes.
    fn apply_ui_output(&mut self, output: ui::UiOutput) {
        if let Some(recover) = output.recovery_choice {
            self.answer_recovery(recover);
        }
        if let Some(choice) = output.unsaved_choice {
            self.answer_unsaved(choice);
        }
        if let Some(created) = output.created_project {
            self.open_project(created);
        }
        if let Some(picked) = output.picked_hdri {
            // Already decoded: the viewport uses it without decoding the file again.
            let hash = ResourceHash::of(&picked.bytes);
            self.environment_sync.preload(
                &self.egui_ctx,
                EnvironmentSource::Resource(hash),
                picked.image,
            );
        }
        if let Some(change) = output.environment {
            self.change_environment(change);
        }
        if let Some(document) = &mut self.document {
            for command in output.commands {
                document.execute(command);
            }
            if output.seal {
                document.history.seal();
            }
            if let Some(applied) = output.history_go_to
                && let Err(error) = document.history.go_to(&mut document.project, applied)
            {
                tracing::error!(%error, "history navigation failed; the history was cleared");
            }
        }
        if let Some(gfx) = &self.graphics {
            gfx.window.request_redraw();
        }
    }

    /// Channels of the current texture set.
    fn current_channels(&self) -> Vec<Channel> {
        self.document
            .as_ref()
            .and_then(|document| document.project.texture_set(self.ui.texture_sets.selected))
            .map(|set| set.channels.iter().map(|def| def.channel).collect())
            .unwrap_or_default()
    }

    /// Renders the 2D view of the current texture set when its panel is shown, and keeps
    /// its egui texture registered.
    /// Renders one missing thumbnail of the Library panel per frame, while the panel is
    /// visible: materials lit by their normal, noises and patterns in gray.
    fn render_thumbnails(&mut self) {
        use crate::library_panel::{THUMBNAIL_SIZE, thumbnail_pixels};
        use patine_procedural::def::Category;
        use patine_procedural::evaluate::{EvaluationUniforms, SurfaceInput};

        let visible = std::mem::take(&mut self.ui.library.visible);
        let Some(gfx) = self.graphics.as_ref() else {
            return;
        };
        let thumbnails = &mut self.ui.library.thumbnails;
        let Some(def) = patine_procedural::library::all()
            .iter()
            .find(|def| !thumbnails.contains_key(&def.id))
        else {
            return;
        };
        if !visible {
            return;
        }
        let size = THUMBNAIL_SIZE;
        // Every output of the procedural, in one pass, read back.
        let render = || -> Option<Vec<Vec<[f32; 4]>>> {
            let formats = patine_procedural::evaluate::output_formats(def);
            let textures: Vec<wgpu::Texture> = formats
                .iter()
                .map(|format| {
                    gfx.gpu.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("thumbnail"),
                        size: wgpu::Extent3d {
                            width: size,
                            height: size,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: *format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    })
                })
                .collect();
            let views: Vec<wgpu::TextureView> = textures
                .iter()
                .map(|texture| texture.create_view(&Default::default()))
                .collect();
            let targets: Vec<_> = views.iter().zip(formats).collect();
            let uniforms = EvaluationUniforms::new(
                &patine_core::layer::Projection::default(),
                size,
                [0.0; 3],
                1.0,
                true,
            );
            let mut encoder = gfx.gpu.device.create_command_encoder(&Default::default());
            let result = gfx.compositor.procedurals().evaluate(
                &gfx.gpu,
                &mut encoder,
                def,
                &def.defaults(),
                &uniforms,
                SurfaceInput::Flat,
                &targets,
            );
            if let Err(error) = result {
                tracing::error!(id = %def.id, %error, "thumbnail failed");
                return None;
            }
            gfx.gpu.queue.submit([encoder.finish()]);
            textures
                .iter()
                .map(|texture| match texture.format() {
                    wgpu::TextureFormat::Rgba16Float => {
                        patine_gpu::readback::read_rgba16f(&gfx.gpu, texture).ok()
                    }
                    _ => patine_gpu::readback::read_r16f_level(&gfx.gpu, texture, 0, 0)
                        .ok()
                        .map(|values| values.into_iter().map(|v| [v, 0.0, 0.0, 0.0]).collect()),
                })
                .collect()
        };
        let pixels = render().and_then(|outputs| {
            if def.category == Category::Material {
                // Outputs in the order of their index: base color 0, normal 3.
                Some(thumbnail_pixels(outputs.first()?, Some(outputs.get(3)?)))
            } else {
                Some(thumbnail_pixels(outputs.first()?, None))
            }
        });
        // A failed thumbnail stays gray instead of being rendered again every frame.
        let pixels = pixels.unwrap_or_else(|| vec![64; (size * size * 4) as usize]);
        let image = egui::ColorImage::from_rgba_unmultiplied([size as usize; 2], &pixels);
        let handle = self.egui_ctx.load_texture(
            format!("thumbnail {}", def.id),
            image,
            egui::TextureOptions::LINEAR,
        );
        thumbnails.insert(def.id.clone(), handle);
        self.egui_ctx.request_repaint();
    }

    fn render_view_2d(&mut self) {
        let state = &mut self.ui.view_2d;
        let visible = std::mem::take(&mut state.visible);
        let (Some(gfx), Some(document)) = (self.graphics.as_mut(), &self.document) else {
            return;
        };
        let Some(mesh) = gfx.gpu_mesh.as_ref() else {
            return;
        };
        if !visible || state.size[0] <= 1 || state.size[1] <= 1 {
            return;
        }
        if !state.framed {
            state.frame();
        }
        let project = &document.project;
        let Some(set) = project
            .texture_set(self.ui.texture_sets.selected)
            .or(project.texture_sets.first())
        else {
            return;
        };
        // The lit and checker modes show the base color.
        let inspect = inspect_of(
            self.ui.view.settings.display,
            self.ui.view.inspect_channel,
            self.ui.layers.selected,
        )
        .unwrap_or(Inspect::Channel(Channel::BaseColor));
        let texture = inspected(inspect, set, &gfx.gpu_document, &gfx.white_mask);
        let scene = UvScene {
            mesh,
            material: set.material_index,
            texture,
            transform: state.transform,
            wireframe: true,
        };
        let reallocated = gfx
            .uv_view
            .render(&gfx.gpu, &scene, state.size[0], state.size[1]);
        if !reallocated && state.texture.is_some() {
            return;
        }
        let Some((_, output)) = gfx.uv_view.output() else {
            return;
        };
        let device = &gfx.gpu.device;
        let filter = wgpu::FilterMode::Linear;
        match state.texture {
            Some(id) => gfx
                .renderer
                .update_egui_texture_from_wgpu_texture(device, output, filter, id),
            None => {
                state.texture = Some(gfx.renderer.register_native_texture(device, output, filter));
                gfx.window.request_redraw();
            }
        }
    }

    /// Renders the 3D viewport and keeps its egui texture registered.
    fn render_viewport(&mut self) {
        let Some(gfx) = self.graphics.as_mut() else {
            return;
        };
        // The GPU follows the document: environment image, rotation and exposure.
        let (wanted, resources) = match &self.document {
            Some(document) => (&document.project.environment, &document.project.resources),
            None => (
                &self.draft_environment.settings,
                &self.draft_environment.resources,
            ),
        };
        let decoded = self
            .environment_sync
            .update(&self.egui_ctx, &wanted.source, resources);
        let view = &mut self.ui.view;
        view.settings.environment_rotation = wanted.rotation;
        view.settings.exposure_ev = wanted.exposure_ev;
        // Frame once the panel size has settled (the same two frames in a row): the first
        // layouts of a new window or dock can report intermediate sizes.
        let size_settled = view.size == view.previous_size && view.size[0] > 1 && view.size[1] > 1;
        view.previous_size = view.size;
        if view.frame_pending && !size_settled {
            gfx.window.request_redraw();
        }
        if view.frame_pending && size_settled {
            view.frame_pending = false;
            if let Some(bounds) = self
                .document
                .as_ref()
                .and_then(|document| document.project.mesh.bounds())
            {
                let aspect = view.aspect();
                view.camera.frame(&bounds, aspect);
                if let Some(zoom) = self.startup.zoom.take() {
                    view.camera.zoom(zoom);
                }
            }
        }
        if let Some(image) = decoded {
            puffin::profile_scope!("environment");
            let start = Instant::now();
            match gfx.viewport.set_environment(&gfx.gpu, &image) {
                Ok(()) => tracing::info!(
                    width = image.width,
                    height = image.height,
                    ms = start.elapsed().as_millis() as u64,
                    "environment processed"
                ),
                Err(error) => {
                    tracing::error!(%error, "environment processing failed");
                    self.environment_sync.error = Some(error.to_string());
                }
            }
        }
        // The document's channel textures, and the materials binding them.
        let inspect = inspect_of(
            view.settings.display,
            view.inspect_channel,
            self.ui.layers.selected,
        );
        match &self.document {
            Some(document) => {
                let project = &document.project;
                let changed =
                    gfx.gpu_document
                        .sync(&gfx.gpu, &mut gfx.pool, &gfx.compositor, project);
                if !changed.is_empty()
                    || gfx.materials_inspect != inspect
                    || gfx.materials.len() != project.mesh.materials.len()
                {
                    gfx.materials = build_materials(
                        &gfx.gpu,
                        &gfx.viewport,
                        &gfx.gpu_document,
                        project,
                        inspect,
                        &gfx.white_mask,
                    );
                    gfx.materials_inspect = inspect;
                }
            }
            None if !gfx.materials.is_empty() => {
                gfx.gpu_document.clear(&mut gfx.pool);
                gfx.materials.clear();
            }
            None => {}
        }

        // Strokes given on the command line, once the view has settled: one per frame,
        // each with its tool.
        if !view.frame_pending && view.texture.is_some() && !self.startup.strokes.is_empty() {
            let stroke = self.startup.strokes.remove(0);
            if stroke.eraser {
                self.ui.tool.tool = crate::paint_tool::Tool::Eraser;
            } else if self.startup.eraser != Some(true) {
                self.ui.tool.tool = crate::paint_tool::Tool::Brush;
            }
            let mut points = stroke.points.iter();
            if let Some(first) = points.next() {
                view.paint_events.push(PaintEvent::Press(*first));
            }
            view.paint_events
                .extend(points.map(|p| PaintEvent::Move(*p)));
            view.paint_events.push(PaintEvent::Release);
            gfx.window.request_redraw();
        }
        // Painting gestures of this frame, on the synced document.
        let mut finished = None;
        if let Some(document) = &self.document {
            let project = &document.project;
            for event in std::mem::take(&mut view.paint_events) {
                match event {
                    PaintEvent::Press(position) => {
                        let Some(set) = project.texture_set(self.ui.texture_sets.selected) else {
                            continue;
                        };
                        let Some(target) = crate::paint_tool::paint_target(
                            set,
                            Some(self.ui.layers.part),
                            self.ui.layers.selected,
                        ) else {
                            view.notice = Some((
                                self.i18n.tr("paint-not-paintable").into_owned(),
                                self.egui_ctx.input(|input| input.time),
                            ));
                            continue;
                        };
                        let (brush, tool, alpha) = self.ui.tool.snapshot(set, target);
                        let stroke = patine_core::stroke::Stroke {
                            brush,
                            tool,
                            camera: patine_core::stroke::StrokeCamera {
                                view_proj: view
                                    .camera
                                    .view_projection(view.aspect())
                                    .to_cols_array_2d(),
                                viewport: view.size,
                            },
                            space: patine_core::stroke::StrokeSpace::View,
                            symmetry: patine_core::stroke::Symmetry::default(),
                            seed: 0,
                            dabs: Vec::new(),
                        };
                        let started = gfx.gpu_document.begin_stroke(
                            &gfx.gpu,
                            &mut gfx.pool,
                            &gfx.compositor,
                            project,
                            target,
                            stroke,
                            alpha.as_ref().map(|(_, resource)| resource),
                        );
                        match started {
                            Ok(()) => {
                                let mut sampler = patine_paint::sampler::StrokeSampler::new(
                                    self.ui.tool.sampler(),
                                );
                                let dabs = sampler.push(position, 1.0, 0.0);
                                gfx.gpu_document.paint(
                                    &gfx.gpu,
                                    &mut gfx.pool,
                                    &gfx.compositor,
                                    project,
                                    &dabs,
                                );
                                let name = set
                                    .stack
                                    .find(target.layer())
                                    .map(|layer| layer.name.clone())
                                    .unwrap_or_default();
                                self.stroke = Some(StrokeInProgress {
                                    sampler,
                                    start: Instant::now(),
                                    target,
                                    alpha,
                                    name,
                                });
                            }
                            Err(error) => {
                                let key = match error {
                                    patine_compositor::gpu_document::PaintError::NoChannel => {
                                        "paint-no-channel"
                                    }
                                    _ => "paint-not-paintable",
                                };
                                tracing::debug!(%error, "stroke refused");
                                view.notice = Some((
                                    self.i18n.tr(key).into_owned(),
                                    self.egui_ctx.input(|input| input.time),
                                ));
                            }
                        }
                    }
                    PaintEvent::Move(position) => {
                        if let Some(stroke) = &mut self.stroke {
                            let time = stroke.start.elapsed().as_secs_f32();
                            let dabs = stroke.sampler.push(position, 1.0, time);
                            if !dabs.is_empty() {
                                gfx.gpu_document.paint(
                                    &gfx.gpu,
                                    &mut gfx.pool,
                                    &gfx.compositor,
                                    project,
                                    &dabs,
                                );
                            }
                        }
                    }
                    PaintEvent::Release => {
                        if let Some(stroke) = self.stroke.take()
                            && let Some(painted) =
                                gfx.gpu_document.end_stroke(&mut gfx.pool, &gfx.compositor)
                        {
                            finished = Some(patine_core::commands::PaintStroke::new(
                                stroke.target,
                                painted,
                                stroke.alpha,
                                stroke.name,
                            ));
                        }
                    }
                }
            }
        } else {
            view.paint_events.clear();
        }
        if let (Some(command), Some(document)) = (finished, self.document.as_mut()) {
            document.history.seal();
            document.execute(Box::new(command));
            document.history.seal();
        }
        let scene = Scene {
            mesh: gfx.gpu_mesh.as_ref(),
            materials: (!gfx.materials.is_empty()).then_some(gfx.materials.as_slice()),
            camera: &view.camera,
            settings: &view.settings,
        };
        let reallocated = gfx
            .viewport
            .render(&gfx.gpu, &scene, view.size[0], view.size[1]);

        // Picking under the cursor: the result of the previous request arrives now, and a
        // new one is sent for the current cursor position.
        if let Some(result) = gfx.viewport.poll_pick(&gfx.gpu) {
            view.hovered = result.triangle;
            view.hovered_point = match (result.triangle, result.position, &self.document) {
                (Some(triangle), Some(position), Some(document)) => {
                    let mesh = &document.project.mesh;
                    mesh.triangles.get(triangle as usize).map(|corners| {
                        let [a, b, c] =
                            corners.map(|v| glam::Vec3::from(mesh.positions[v as usize]));
                        crate::paint_tool::SurfacePoint {
                            position: glam::Vec3::new(position.x, position.y, position.z),
                            normal: (b - a).cross(c - a).normalize_or(glam::Vec3::Y),
                        }
                    })
                }
                _ => None,
            };
        }
        // The brush cursor, at the size of the brush, gray when nothing can be painted.
        view.cursor = match (view.hovered_point, &self.document) {
            (Some(point), Some(document)) if view.hover_pixel.is_some() => {
                let paintable = document
                    .project
                    .texture_set(self.ui.texture_sets.selected)
                    .and_then(|set| {
                        crate::paint_tool::paint_target(
                            set,
                            Some(self.ui.layers.part),
                            self.ui.layers.selected,
                        )
                    })
                    .is_some();
                let view_proj = view.camera.view_projection(view.aspect());
                let right = view.camera.view().row(0).truncate();
                let outline = crate::paint_tool::cursor_outline(
                    glam::Mat4::from_cols_array_2d(&view_proj.to_cols_array_2d()),
                    view.size,
                    point,
                    self.ui.tool.size * 0.5,
                    glam::Vec3::new(right.x, right.y, right.z),
                );
                Some((outline, paintable))
            }
            _ => None,
        };
        match view.hover_pixel {
            Some(pixel) if scene.mesh.is_some() => {
                let key = (pixel, view.size, view.camera);
                if gfx.last_pick != Some(key)
                    && gfx.viewport.request_pick(
                        &gfx.gpu,
                        &scene,
                        view.size[0],
                        view.size[1],
                        pixel,
                    )
                {
                    gfx.last_pick = Some(key);
                }
                if gfx.last_pick != Some(key) || gfx.viewport.pick_pending() {
                    // Retry next frame, or show the result as soon as it is read back.
                    gfx.window.request_redraw();
                }
            }
            _ => {
                view.hovered = None;
                view.hovered_point = None;
                gfx.last_pick = None;
            }
        }

        if !reallocated && view.texture.is_some() {
            return;
        }
        let Some((_, output)) = gfx.viewport.output() else {
            return;
        };
        let device = &gfx.gpu.device;
        let filter = wgpu::FilterMode::Linear;
        match view.texture {
            Some(id) => gfx
                .renderer
                .update_egui_texture_from_wgpu_texture(device, output, filter, id),
            None => {
                view.texture = Some(gfx.renderer.register_native_texture(device, output, filter));
                // The panel only knows the texture from the next frame on.
                gfx.window.request_redraw();
            }
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        puffin::profile_function!();
        if let Some(result) = self.file_jobs.poll() {
            self.finish_file_job(result);
        }
        let action_context = self.action_context();
        let environment_name = self.environment_name();
        let Some(gfx) = self.graphics.as_mut() else {
            return;
        };
        if gfx.config.width == 0 || gfx.config.height == 0 {
            return; // Minimized.
        }

        let raw_input = gfx.egui_state.take_egui_input(&gfx.window);
        let mut triggered = Vec::new();
        let mut outputs = Vec::new();
        let info = FrameInfo {
            i18n: &self.i18n,
            actions: &self.actions,
            action_context,
            logs: &self.logs,
            gpu_name: &gfx.gpu_name,
            gpu_backend: &gfx.gpu_backend,
            project: self.document.as_ref().map(|document| &document.project),
            hovered: self.ui.view.hovered,
            environment: EnvironmentView {
                settings: match &self.document {
                    Some(document) => &document.project.environment,
                    None => &self.draft_environment.settings,
                },
                name: environment_name,
                loading: self.environment_sync.loading(),
                error: self.environment_sync.error.as_deref(),
            },
            history: self
                .document
                .as_ref()
                .map(|document| history_panel::HistoryView {
                    labels: document.history.labels(),
                    applied: document.history.applied(),
                    truncated: document.history.truncated(),
                }),
            gpu_memory: self
                .document
                .as_ref()
                .map(|_| gfx.gpu_document.memory() + gfx.gpu_document.mask_memory()),
            file_status: if self.file_jobs.saving() {
                Some("file-busy-saving")
            } else if self.file_jobs.opening() {
                Some("file-busy-opening")
            } else {
                None
            },
        };
        let ui_state = &mut self.ui;
        let actions = &self.actions;
        let mut full_output = self.egui_ctx.run_ui(raw_input, |ui| {
            puffin::profile_scope!("ui");
            let text_focus = ui.ctx().egui_wants_keyboard_input();
            triggered.extend(ui.ctx().input_mut(|input| {
                actions.triggered_by_shortcuts(input, &action_context, text_focus)
            }));
            let mut output = ui::show(ui, ui_state, &info);
            triggered.append(&mut output.actions);
            outputs.push(output);
        });
        if let Some(pixel) = self.startup.hover {
            self.ui.view.hover_pixel = Some(pixel);
        }
        gfx.egui_state.handle_platform_output(
            &gfx.window,
            std::mem::take(&mut full_output.platform_output),
        );

        let repaint_delay = full_output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|viewport| viewport.repaint_delay);

        // Document changes of this frame, before rendering it.
        for output in outputs {
            self.apply_ui_output(output);
        }
        if let Some(autosaver) = &mut self.autosaver {
            autosaver.tick(
                &self.egui_ctx,
                self.document.as_ref(),
                self.document_generation,
            );
        }

        // The 3D image of this frame, with the camera moved by this frame's input.
        self.render_viewport();
        self.render_view_2d();
        self.render_thumbnails();
        let Some(gfx) = self.graphics.as_mut() else {
            return;
        };
        let capture_now = match &mut self.capture {
            Some((path, 0)) => Some(path.clone()),
            Some((_, frames)) => {
                *frames -= 1;
                gfx.window.request_redraw();
                None
            }
            None => None,
        };
        let captured = Self::paint(
            gfx,
            &self.egui_ctx,
            &mut full_output,
            capture_now.as_deref(),
        );
        if let Some(result) = captured {
            if let Err(error) = result {
                tracing::error!("{error:#}");
                self.error = Some(error);
            }
            event_loop.exit();
        }
        self.profiler.new_frame();

        for action in triggered {
            self.execute(action);
        }
        if self.quit_requested {
            // A save in progress finishes before the application exits.
            if let Some(result) = self.file_jobs.wait() {
                self.finish_file_job(result);
            }
            event_loop.exit();
        }
        let title = self.window_title();
        if let Some(gfx) = &self.graphics
            && title != self.title
        {
            gfx.window.set_title(&title);
            self.title = title;
        }

        match repaint_delay {
            Some(delay) if delay.is_zero() => {
                if let Some(gfx) = &self.graphics {
                    gfx.window.request_redraw();
                }
            }
            Some(delay) => {
                if let Some(deadline) = Instant::now().checked_add(delay) {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
                }
            }
            None => {}
        }
    }

    /// Draws egui to the window. With `capture`, the image is also saved to that PNG and
    /// the result of the capture is returned.
    fn paint(
        gfx: &mut Graphics,
        egui_ctx: &egui::Context,
        full_output: &mut egui::FullOutput,
        capture: Option<&Path>,
    ) -> Option<anyhow::Result<()>> {
        let mut captured = None;
        puffin::profile_function!();
        let Graphics {
            window,
            surface,
            config,
            gpu,
            renderer,
            ..
        } = gfx;
        let (device, queue) = (&gpu.device, &gpu.queue);

        // Texture updates must reach the renderer even when no frame is drawn.
        let textures = &mut full_output.textures_delta;
        for (id, deltas) in textures.set.drain() {
            for delta in deltas {
                renderer.update_texture(device, queue, id, &delta);
            }
        }

        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Some(frame),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                tracing::debug!("surface outdated or lost, reconfiguring");
                surface.configure(device, config);
                window.request_redraw();
                None
            }
            other => {
                tracing::debug!(?other, "skipping frame");
                None
            }
        };

        if let Some(frame) = frame {
            let primitives = egui_ctx.tessellate(
                std::mem::take(&mut full_output.shapes),
                full_output.pixels_per_point,
            );
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [config.width, config.height],
                pixels_per_point: full_output.pixels_per_point,
            };
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("egui"),
            });
            let mut command_buffers =
                renderer.update_buffers(device, queue, &mut encoder, &primitives, &screen);
            let view = frame
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            {
                let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                renderer.render(&mut pass.forget_lifetime(), &primitives, &screen);
            }
            command_buffers.push(encoder.finish());
            queue.submit(command_buffers);
            if let Some(path) = capture {
                captured = Some(save_capture(gpu, &frame.texture, path));
            }
            window.pre_present_notify();
            queue.present(frame);
        }

        for id in textures.free.drain() {
            renderer.free_texture(&id);
        }
        captured
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if let Some(gfx) = self.graphics.as_mut() {
            gfx.config.width = size.width;
            gfx.config.height = size.height;
            if size.width > 0 && size.height > 0 {
                gfx.surface.configure(&gfx.gpu.device, &gfx.config);
                gfx.window.request_redraw();
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_some() {
            return;
        }
        match self.create_graphics(event_loop) {
            Ok(graphics) => {
                graphics.window.request_redraw();
                self.graphics = Some(graphics);
            }
            Err(error) => {
                tracing::error!("{error:#}");
                self.error = Some(error);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let Some(gfx) = self.graphics.as_mut() {
            let response = gfx.egui_state.on_window_event(&gfx.window, &event);
            if response.repaint {
                gfx.window.request_redraw();
            }
        }
        match event {
            WindowEvent::CloseRequested => {
                self.request(PendingAction::Quit);
                if let Some(gfx) = &self.graphics {
                    gfx.window.request_redraw();
                }
            }
            WindowEvent::Resized(size) => self.resize(size),
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // A WaitUntil deadline (egui's or the autosave's) has passed: draw a frame, which
        // also runs a due autosave.
        if let ControlFlow::WaitUntil(deadline) = event_loop.control_flow()
            && Instant::now() >= deadline
        {
            event_loop.set_control_flow(ControlFlow::Wait);
            if let Some(gfx) = &self.graphics {
                gfx.window.request_redraw();
            }
        }
        // Wake up for the next autosave even when nothing happens.
        let autosave = self.autosaver.as_ref().and_then(|autosaver| {
            autosaver.deadline(self.document.as_ref(), self.document_generation)
        });
        if let Some(autosave) = autosave {
            match event_loop.control_flow() {
                ControlFlow::WaitUntil(deadline) if deadline <= autosave => {}
                _ => event_loop.set_control_flow(ControlFlow::WaitUntil(autosave)),
            }
        }
    }
}

/// Uploads `mesh` with the picking buffers the viewport needs on this device.
fn upload_mesh(
    gpu: &GpuContext,
    viewport: &Viewport,
    mesh: &patine_core::mesh::MeshAsset,
) -> Option<GpuMesh> {
    if viewport.needs_flat_pick_buffers() {
        GpuMesh::with_flat_picking(gpu, mesh)
    } else {
        GpuMesh::new(gpu, mesh)
    }
}
