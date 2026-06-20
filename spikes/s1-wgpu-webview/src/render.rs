//! Headless wgpu render — the producer side of the spike.
//!
//! Produces a `wgpu::Texture` we own, exactly as `palmier-engine`'s compositor (E5-S8)
//! will. The spike renders an animated clear-colour (frame-dependent) so a "moving"
//! frame can be observed via readback; production swaps this for the real textured-quad
//! composition pass. The point being proven here is *ownership*: the frame lives in a
//! Rust-owned wgpu texture, which is precisely why presenting it into a webview-owned
//! GPU context (candidate b) is hard and a native composited surface (a/c) is the answer.

use crate::RenderedFrame;

/// A minimal wgpu device/queue pair. In production this is created once and shared with
/// the presentation surface so the rendered texture and the swapchain are on the SAME
/// device (required for the zero-copy DComp/GTK paths — a cross-device texture would
/// force a copy).
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl GpuContext {
    /// Create a headless context. On Windows this comes up on the dx12 backend (D3D12),
    /// on Linux on Vulkan — the same backends the GPU floor (FOUNDATION §3) requires and
    /// the same ones the presentation seam targets.
    pub fn new_headless() -> Result<Self, String> {
        // NOTE: for the *real* Windows presentation path, this instance must be built
        // with `Dx12SwapchainKind::DxgiFromVisual` so its surfaces can be bound to a
        // DirectComposition visual. See `present::windows`. Here (headless) we use the
        // default instance because we never create a swapchain surface.
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .map_err(|e| format!("no suitable GPU adapter: {e:?}"))?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("s1-spike-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| format!("request_device failed: {e:?}"))?;

        Ok(Self { instance, adapter, device, queue })
    }

    /// Report which backend/adapter actually came up — lets the proof bin print whether
    /// we're on D3D12 (Windows target) or Vulkan (Linux target).
    pub fn adapter_summary(&self) -> String {
        let info = self.adapter.get_info();
        format!("{:?} | {} | {:?}", info.backend, info.name, info.device_type)
    }
}

/// Render one frame to an owned offscreen texture. `frame_index` animates the clear
/// colour so readback can show motion. Format is the premultiplied-alpha-friendly
/// BGRA8 the webview compositor expects (FOUNDATION §6.5 / risk #3).
pub fn render_frame(ctx: &GpuContext, width: u32, height: u32, frame_index: u32) -> RenderedFrame {
    let format = wgpu::TextureFormat::Bgra8UnormSrgb;

    let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("s1-composited-frame"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        // RENDER_ATTACHMENT: we draw into it. COPY_SRC: candidate-(c) readback can copy
        // it out. In the real DComp path the swapchain's own texture is the attachment;
        // here we render to a standalone texture to keep the spike headless.
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    // Animate clear colour by frame so "movement" is observable in readback output.
    let t = (frame_index % 120) as f64 / 120.0;
    let clear = wgpu::Color { r: t, g: 1.0 - t, b: 0.25, a: 1.0 };

    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("s1-encoder") });
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("s1-clear-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        // Production: bind the textured-quad pipeline and draw bottom->top LayerRenders
        // here (E5-S8). The clear alone is enough to prove the produce->present seam.
    }
    ctx.queue.submit(std::iter::once(encoder.finish()));

    RenderedFrame { texture, width, height, format }
}
