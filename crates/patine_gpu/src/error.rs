use thiserror::Error;

/// Errors raised by the GPU layer.
#[derive(Debug, Error)]
pub enum GpuError {
    /// No adapter matches the requested [`crate::AdapterChoice`].
    #[error("no suitable GPU adapter found ({choice}): {source}")]
    NoAdapter {
        /// Human-readable description of the requested adapter.
        choice: String,
        /// Underlying wgpu error.
        #[source]
        source: wgpu::RequestAdapterError,
    },
    /// The adapter refused to create a device.
    #[error("failed to create the GPU device: {0}")]
    RequestDevice(#[from] wgpu::RequestDeviceError),
    /// No shader with this name exists under `shaders/`.
    #[error("unknown shader `{0}`")]
    ShaderNotFound(String),
    /// A shader file could not be read from disk (hot-reload mode).
    #[error("failed to read shader `{name}`: {source}")]
    ShaderIo {
        /// Shader name, relative to `shaders/` and without extension.
        name: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The device rejected a shader module.
    #[error("invalid shader `{name}`: {message}")]
    ShaderInvalid {
        /// Shader name, relative to `shaders/` and without extension.
        name: String,
        /// Validation message reported by wgpu.
        message: String,
    },
    /// A texture with this format cannot be read back.
    #[error("texture format {0:?} cannot be read back")]
    UnsupportedReadbackFormat(wgpu::TextureFormat),
    /// Mapping a staging buffer failed.
    #[error("failed to map a GPU buffer: {0}")]
    BufferMap(String),
    /// Waiting for the device failed.
    #[error("failed to wait for the GPU: {0}")]
    Poll(#[from] wgpu::PollError),
}
