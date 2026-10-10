//! Main window: winit event loop, wgpu surface and egui rendering.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context as _;
use patine_gpu::{AdapterChoice, GpuContext, wgpu};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use patine_core::project::{Project, ProjectSettings};
use patine_mesh::BuildOptions;
use patine_render::camera::Projection;
use patine_render::mesh_gpu::GpuMesh;
use patine_render::viewport::{Scene, Viewport};

use crate::actions::{ActionContext, ActionId, ActionRegistry};
use crate::i18n::{I18n, Language};
use crate::logging::LogBuffer;
use crate::new_project::{CreatedProject, NewProjectDialog};
use crate::profiling::Profiler;
use crate::ui::{self, FrameInfo, UiState};

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
}

/// Application state.
pub struct App {
    logs: LogBuffer,
    profiler: Profiler,
    i18n: I18n,
    actions: ActionRegistry,
    ui: UiState,
    /// The document. Replaced as a whole by New Project; edited through commands (M3).
    project: Option<Project>,
    egui_ctx: egui::Context,
    graphics: Option<Graphics>,
    /// First fatal error; the event loop exits and `run` returns it.
    error: Option<anyhow::Error>,
}

/// Opens the main window and runs until it is closed.
///
/// With `mesh`, a project is created from it at startup with default settings.
pub fn run(logs: LogBuffer, profiler: Profiler, mesh: Option<&Path>) -> anyhow::Result<()> {
    let event_loop = EventLoop::new().context("failed to create the event loop")?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let egui_ctx = egui::Context::default();
    ui::setup_style(&egui_ctx);
    let mut app = App {
        logs,
        profiler,
        i18n: I18n::new(Language::system()),
        actions: ActionRegistry::default(),
        ui: UiState::default(),
        project: None,
        egui_ctx,
        graphics: None,
        error: None,
    };
    if let Some(path) = mesh {
        let imported = patine_mesh::import(path, BuildOptions::default())
            .with_context(|| format!("cannot import {}", path.display()))?;
        app.open_project(CreatedProject {
            project: Project::new(imported.mesh, path.to_owned(), ProjectSettings::default()),
            report: imported.report,
        });
    }
    event_loop.run_app(&mut app).context("event loop failed")?;
    match app.error.take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

impl App {
    fn action_context(&self) -> ActionContext {
        let view = &self.ui.view;
        ActionContext {
            profiler_enabled: self.profiler.is_enabled(),
            language: self.i18n.language(),
            has_project: self.project.is_some(),
            orthographic: view.camera.projection == Projection::Orthographic,
            wireframe: view.settings.wireframe,
            shadows: view.settings.shadows,
        }
    }

    fn create_graphics(&self, event_loop: &ActiveEventLoop) -> anyhow::Result<Graphics> {
        let attributes = Window::default_attributes()
            .with_title(self.i18n.tr("app-title"))
            .with_inner_size(PhysicalSize::new(1600, 900))
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
        let gpu_mesh = self
            .project
            .as_ref()
            .and_then(|project| GpuMesh::new(&gpu, &project.mesh));

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
        })
    }

    /// Replaces the document by a new project. This is not an undoable command: the
    /// history starts over with the new document.
    fn open_project(&mut self, created: CreatedProject) {
        let CreatedProject { project, report } = created;
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
        if let Some(gfx) = self.graphics.as_mut() {
            gfx.gpu_mesh = GpuMesh::new(&gfx.gpu, &project.mesh);
        }
        if let Some(bounds) = project.mesh.bounds() {
            let aspect = self.ui.view.aspect();
            self.ui.view.camera.frame(&bounds, aspect);
        }
        self.project = Some(project);
    }

    fn execute(&mut self, action: ActionId, event_loop: &ActiveEventLoop) {
        tracing::debug!(action = self.actions.get(action).name, "action");
        match action {
            ActionId::NewProject => {
                if self.ui.new_project.is_none() {
                    self.ui.new_project = Some(NewProjectDialog::default());
                }
            }
            ActionId::Quit => event_loop.exit(),
            ActionId::ResetLayout => self.ui.dock = ui::default_layout(),
            ActionId::ToggleProfiler => {
                let enabled = !self.profiler.is_enabled();
                self.profiler.set_enabled(enabled);
            }
            ActionId::LanguageEnglish => self.i18n = I18n::new(Language::English),
            ActionId::LanguageFrench => self.i18n = I18n::new(Language::French),
            ActionId::About => self.ui.show_about = true,
            ActionId::FrameMesh => {
                if let Some(bounds) = self.project.as_ref().and_then(|p| p.mesh.bounds()) {
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
        }
        if let Some(gfx) = &self.graphics {
            gfx.window.request_redraw();
        }
    }

    /// Renders the 3D viewport and keeps its egui texture registered.
    fn render_viewport(&mut self) {
        let Some(gfx) = self.graphics.as_mut() else {
            return;
        };
        let view = &mut self.ui.view;
        if let Some(image) = view.environment.pending.take() {
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
                    view.environment.error = Some(error.to_string());
                }
            }
        }
        let scene = Scene {
            mesh: gfx.gpu_mesh.as_ref(),
            camera: &view.camera,
            settings: &view.settings,
        };
        let reallocated = gfx
            .viewport
            .render(&gfx.gpu, &scene, view.size[0], view.size[1]);
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
        let action_context = self.action_context();
        let Some(gfx) = self.graphics.as_mut() else {
            return;
        };
        if gfx.config.width == 0 || gfx.config.height == 0 {
            return; // Minimized.
        }

        let raw_input = gfx.egui_state.take_egui_input(&gfx.window);
        let mut triggered = Vec::new();
        let mut created_project = None;
        let info = FrameInfo {
            i18n: &self.i18n,
            actions: &self.actions,
            action_context,
            logs: &self.logs,
            gpu_name: &gfx.gpu_name,
            gpu_backend: &gfx.gpu_backend,
            project: self.project.as_ref(),
        };
        let ui_state = &mut self.ui;
        let actions = &self.actions;
        let mut full_output = self.egui_ctx.run_ui(raw_input, |ui| {
            puffin::profile_scope!("ui");
            let text_focus = ui.ctx().egui_wants_keyboard_input();
            triggered.extend(ui.ctx().input_mut(|input| {
                actions.triggered_by_shortcuts(input, &action_context, text_focus)
            }));
            let output = ui::show(ui, ui_state, &info);
            triggered.extend(output.actions);
            created_project = created_project.take().or(output.created_project);
        });
        gfx.egui_state.handle_platform_output(
            &gfx.window,
            std::mem::take(&mut full_output.platform_output),
        );

        let repaint_delay = full_output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|viewport| viewport.repaint_delay);

        // The 3D image of this frame, with the camera moved by this frame's input.
        self.render_viewport();
        let Some(gfx) = self.graphics.as_mut() else {
            return;
        };
        Self::paint(gfx, &self.egui_ctx, &mut full_output);
        self.profiler.new_frame();

        if let Some(created) = created_project {
            self.open_project(created);
        }
        for action in triggered {
            self.execute(action, event_loop);
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

    fn paint(gfx: &mut Graphics, egui_ctx: &egui::Context, full_output: &mut egui::FullOutput) {
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
            window.pre_present_notify();
            queue.present(frame);
        }

        for id in textures.free.drain() {
            renderer.free_texture(&id);
        }
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
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => self.resize(size),
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // A WaitUntil deadline from egui has passed: draw the requested frame.
        if let ControlFlow::WaitUntil(deadline) = event_loop.control_flow()
            && Instant::now() >= deadline
        {
            event_loop.set_control_flow(ControlFlow::Wait);
            if let Some(gfx) = &self.graphics {
                gfx.window.request_redraw();
            }
        }
    }
}
