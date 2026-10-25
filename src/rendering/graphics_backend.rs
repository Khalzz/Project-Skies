//! Which graphics API the renderer draws with - picked once, when the app
//! starts (wgpu chooses it as it creates the GPU device), so changing it
//! takes a restart:
//!
//!     let mut app = App::new_with_backend(GraphicsBackend::Vulkan).await?;
//!     app.renderer.backend()          // what it ended up with
//!     GraphicsBackend::available()    // what this machine can use
//!
//! `Auto` lets wgpu pick the best one this machine has.

use std::fmt;
use std::sync::OnceLock;

/// A graphics API wgpu can draw with - see the module doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GraphicsBackend {
    /// Whichever wgpu thinks best here.
    #[default]
    Auto,
    Vulkan,
    /// DirectX 12 - Windows.
    Dx12,
    /// Metal - macOS.
    Metal,
    /// OpenGL (or OpenGL ES) - the most compatible, the least capable.
    OpenGl,
}

impl GraphicsBackend {
    /// Every backend that can be asked for, `Auto` first.
    pub const ALL: [GraphicsBackend; 5] = [
        GraphicsBackend::Auto,
        GraphicsBackend::Vulkan,
        GraphicsBackend::Dx12,
        GraphicsBackend::Metal,
        GraphicsBackend::OpenGl,
    ];

    /// The ones this machine can draw with - a GPU answers for each - with
    /// `Auto` first. Worked out once, when the app starts (before it makes its
    /// own GPU device - see App::new_with_backend), and kept: asking each
    /// backend means creating and dropping a wgpu instance per backend, and
    /// doing that beside a live device can lose the device (seen on AMD's
    /// Vulkan driver). Any later call just reads the kept list.
    pub fn available() -> Vec<GraphicsBackend> {
        static AVAILABLE: OnceLock<Vec<GraphicsBackend>> = OnceLock::new();
        AVAILABLE.get_or_init(|| {
            let mut usable = vec![GraphicsBackend::Auto];
            for backend in GraphicsBackend::ALL.into_iter().skip(1) {
                let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor { backends: backend.wgpu_backends(), ..Default::default() });
                if !instance.enumerate_adapters(backend.wgpu_backends()).is_empty() {
                    usable.push(backend);
                }
            }
            usable
        }).clone()
    }

    /// Its name for showing, "Vulkan", "DirectX 12"...
    pub fn name(self) -> &'static str {
        match self {
            GraphicsBackend::Auto => "Auto",
            GraphicsBackend::Vulkan => "Vulkan",
            GraphicsBackend::Dx12 => "DirectX 12",
            GraphicsBackend::Metal => "Metal",
            GraphicsBackend::OpenGl => "OpenGL",
        }
    }

    /// A short lowercase key for saving it - "auto", "vulkan", "dx12",
    /// "metal", "opengl" - read back with `from_key`.
    pub fn key(self) -> &'static str {
        match self {
            GraphicsBackend::Auto => "auto",
            GraphicsBackend::Vulkan => "vulkan",
            GraphicsBackend::Dx12 => "dx12",
            GraphicsBackend::Metal => "metal",
            GraphicsBackend::OpenGl => "opengl",
        }
    }

    pub fn from_key(key: &str) -> Option<GraphicsBackend> {
        GraphicsBackend::ALL.into_iter().find(|backend| backend.key().eq_ignore_ascii_case(key.trim()))
    }

    /// The wgpu backends to create an instance with.
    pub(crate) fn wgpu_backends(self) -> wgpu::Backends {
        match self {
            GraphicsBackend::Auto => wgpu::Backends::PRIMARY | wgpu::Backends::GL,
            GraphicsBackend::Vulkan => wgpu::Backends::VULKAN,
            GraphicsBackend::Dx12 => wgpu::Backends::DX12,
            GraphicsBackend::Metal => wgpu::Backends::METAL,
            GraphicsBackend::OpenGl => wgpu::Backends::GL,
        }
    }

    /// The one wgpu ended up with.
    pub(crate) fn from_wgpu(backend: wgpu::Backend) -> GraphicsBackend {
        match backend {
            wgpu::Backend::Vulkan => GraphicsBackend::Vulkan,
            wgpu::Backend::Dx12 => GraphicsBackend::Dx12,
            wgpu::Backend::Metal => GraphicsBackend::Metal,
            wgpu::Backend::Gl => GraphicsBackend::OpenGl,
            _ => GraphicsBackend::Auto,
        }
    }
}

impl fmt::Display for GraphicsBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
