//! S-2 WRY-integration spike — the wgpu producer that draws into a swapchain on the
//! SAME window a transparent WRY webview is parented over.
//!
//! S-1 proved the produce seam headlessly and pinned the DirectComposition call path
//! in `spikes/s1-wgpu-webview/src/present.rs`. This spike's job is the un-executed
//! integration: a LIVE window where the wgpu frame actually shows through a transparent
//! webview. The compositing here is done by the OS (DWM on Windows) merging the wgpu
//! swapchain on the window's HWND with the WebView2 child HWND that WRY parents on top.
//!
//! This is the same architectural shape S-1 recommended ("native wgpu surface
//! composited UNDER a transparent webview by the OS compositor") — realized through
//! WRY's own `build_as_child` child-window path rather than a hand-wired DComp visual
//! tree. See FINDINGS.md for the A/B/C plan mapping.

use std::sync::Arc;
use winit::window::Window;

/// wgpu device/queue/surface bound to a real window. In production (`palmier-engine`)
/// the surface is the compositor's present target and the textured-quad pass draws the
/// `CompositionFrame` layers; here we draw a clear-color + a triangle, which is all that
/// is needed to prove the produce -> present -> OS-composite seam visually.
pub struct GfxState {
    pub surface: wgpu::Surface<'static>,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub pipeline: wgpu::RenderPipeline,
    /// What the swapchain reported for composite-alpha — load-bearing for whether the
    /// wgpu layer can be transparent where the shader writes alpha < 1 (so the webview
    /// chrome and desktop can show through). Reported by the proof for the verdict.
    pub alpha_mode: wgpu::CompositeAlphaMode,
}

impl GfxState {
    /// Build the wgpu surface ON the window. `force_dx12` exercises the Windows
    /// production backend explicitly — S-1 came up Vulkan/AMD on this box, and the
    /// DirectComposition production path depends on dx12, so we want to know if the
    /// D3D12 backend cooperates here.
    pub fn new(window: Arc<Window>, force_dx12: bool) -> Result<Self, String> {
        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));

        let backends = if force_dx12 {
            wgpu::Backends::DX12
        } else {
            wgpu::Backends::all()
        };

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });

        // create_surface takes the window by Arc (it implements HasWindowHandle +
        // HasDisplayHandle via raw-window-handle 0.6) and yields a 'static surface.
        let surface = instance
            .create_surface(window)
            .map_err(|e| format!("create_surface failed: {e:?}"))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .map_err(|e| format!("no suitable GPU adapter (backends={backends:?}): {e:?}"))?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("s2-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| format!("request_device failed: {e:?}"))?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb))
            .unwrap_or(caps.formats[0]);

        // Prefer a non-opaque composite-alpha so wgpu pixels with alpha < 1 let the
        // webview/desktop behind show through (the "transparent hole" works in reverse
        // too: GPU layer can be partially transparent). If the platform only offers
        // Opaque, the GPU layer is fully opaque and the webview must sit OVER it (which
        // is exactly our layout: chrome on top, GPU behind).
        let alpha_mode = caps
            .alpha_modes
            .iter()
            .copied()
            .find(|m| {
                matches!(
                    m,
                    wgpu::CompositeAlphaMode::PreMultiplied
                        | wgpu::CompositeAlphaMode::PostMultiplied
                )
            })
            .unwrap_or(caps.alpha_modes[0]);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("s2-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("s2-layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("s2-pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        Ok(Self {
            surface,
            adapter,
            device,
            queue,
            config,
            pipeline,
            alpha_mode,
        })
    }

    pub fn adapter_summary(&self) -> String {
        let info = self.adapter.get_info();
        format!(
            "{:?} | {} | {:?} | swapchain={:?} | alpha={:?}",
            info.backend, info.name, info.device_type, self.config.format, self.alpha_mode
        )
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    /// Draw one frame: clear to a vivid magenta and draw a green triangle. Both are
    /// chosen to be unmistakable behind/around the semi-transparent webview chrome so a
    /// human (or a screenshot) can confirm the GPU layer is actually showing.
    pub fn render(&mut self, frame_index: u32) -> Result<(), wgpu::SurfaceError> {
        let frame = self.surface.get_current_texture()?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Animate the clear so the GPU layer is obviously live (pulsing), not a static
        // paint that could be mistaken for a webview background.
        let t = (frame_index % 180) as f64 / 180.0;
        let clear = wgpu::Color {
            r: 0.8,
            g: 0.05 + 0.4 * t,
            b: 0.8,
            a: 1.0,
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("s2-enc") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("s2-pass"),
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
            pass.set_pipeline(&self.pipeline);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
        Ok(())
    }
}

const SHADER: &str = r#"
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i) - 1) * 0.7;
    let y = f32(i32(i & 1u) * 2 - 1) * 0.7;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    // Opaque green triangle — premultiplied alpha so a=1 means fully opaque.
    return vec4<f32>(0.0, 1.0, 0.2, 1.0);
}
"#;
