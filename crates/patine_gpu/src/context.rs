use std::fmt;

use crate::{GpuError, shaders};

/// Which kind of adapter to request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdapterChoice {
    /// The real, high-performance GPU.
    Hardware,
    /// The software rasterizer: WARP (DX12) on Windows, lavapipe (Vulkan) on Linux.
    ///
    /// Used by tests so that golden images are identical locally and in CI.
    Software,
}

impl fmt::Display for AdapterChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdapterChoice::Hardware => f.write_str("hardware adapter"),
            AdapterChoice::Software => f.write_str("software adapter"),
        }
    }
}

impl AdapterChoice {
    fn backends(self) -> wgpu::Backends {
        match self {
            AdapterChoice::Hardware => wgpu::Backends::PRIMARY,
            // WARP is only exposed through DX12, lavapipe through Vulkan.
            AdapterChoice::Software if cfg!(windows) => wgpu::Backends::DX12,
            AdapterChoice::Software => wgpu::Backends::VULKAN,
        }
    }
}

/// Optional capabilities of the device that later milestones depend on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuCapabilities {
    /// `R16Float` can be used as a write-only storage texture (compositor scalar channels).
    pub r16float_storage: bool,
    /// 32-bit float textures can be sampled with linear filtering.
    pub float32_filterable: bool,
}

/// Optional features requested whenever the adapter supports them.
const OPTIONAL_FEATURES: wgpu::Features = wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
    .union(wgpu::Features::FLOAT32_FILTERABLE);

/// A wgpu instance, adapter, device and queue, shared by every GPU subsystem.
#[derive(Debug)]
pub struct GpuContext {
    /// The wgpu instance the adapter comes from.
    pub instance: wgpu::Instance,
    /// The selected adapter.
    pub adapter: wgpu::Adapter,
    /// The logical device.
    pub device: wgpu::Device,
    /// The device's queue.
    pub queue: wgpu::Queue,
}

impl GpuContext {
    /// Creates an instance suitable for the given adapter choice.
    ///
    /// `WGPU_BACKEND` and the other wgpu environment variables still apply.
    pub fn create_instance(choice: AdapterChoice) -> wgpu::Instance {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = choice.backends();
        wgpu::Instance::new(desc.with_env())
    }

    /// Creates a context without any surface, for tests and the command line. Blocks.
    pub fn new_headless(choice: AdapterChoice) -> Result<Self, GpuError> {
        pollster::block_on(Self::request(Self::create_instance(choice), choice, None))
    }

    /// Requests an adapter and a device from `instance`.
    ///
    /// When `compatible_surface` is given, the adapter is guaranteed to present to it.
    pub async fn request(
        instance: wgpu::Instance,
        choice: AdapterChoice,
        compatible_surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<Self, GpuError> {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: choice == AdapterChoice::Software,
                compatible_surface,
                ..Default::default()
            })
            .await
            .map_err(|source| GpuError::NoAdapter {
                choice: choice.to_string(),
                source,
            })?;

        let info = adapter.get_info();
        tracing::info!(
            name = %info.name,
            backend = ?info.backend,
            device_type = ?info.device_type,
            driver = %info.driver,
            "GPU adapter selected"
        );

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("patine"),
                required_features: adapter.features() & OPTIONAL_FEATURES,
                required_limits: wgpu::Limits::default(),
                ..Default::default()
            })
            .await?;

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    /// Information about the selected adapter.
    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.adapter.get_info()
    }

    /// Optional capabilities actually enabled on the device.
    pub fn capabilities(&self) -> GpuCapabilities {
        let features = self.device.features();
        let r16 = self
            .adapter
            .get_texture_format_features(wgpu::TextureFormat::R16Float);
        GpuCapabilities {
            r16float_storage: features
                .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
                && r16
                    .allowed_usages
                    .contains(wgpu::TextureUsages::STORAGE_BINDING)
                && r16
                    .flags
                    .contains(wgpu::TextureFormatFeatureFlags::STORAGE_WRITE_ONLY),
            float32_filterable: features.contains(wgpu::Features::FLOAT32_FILTERABLE),
        }
    }

    /// Loads the shader `name` (see [`shaders::load`]) and compiles it on the device.
    ///
    /// Validation errors are returned instead of reaching the uncaptured-error handler,
    /// so a broken shader in hot-reload mode does not bring the application down.
    pub fn create_shader_module(&self, name: &str) -> Result<wgpu::ShaderModule, GpuError> {
        let source = shaders::load(name)?;
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(name),
                source: wgpu::ShaderSource::Wgsl(source),
            });
        match pollster::block_on(scope.pop()) {
            Some(error) => Err(GpuError::ShaderInvalid {
                name: name.to_owned(),
                message: error.to_string(),
            }),
            None => Ok(module),
        }
    }
}
