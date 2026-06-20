//! The wgpu compositor — E5-S8. **Feature-gated behind `wgpu-compositor`.**
//!
//! Reproduces the macOS reference's per-frame `AVVideoComposition` render as a wgpu
//! textured-quad pass (preview-engine.md "Mapping → palmier-engine"): clear to black
//! (the opaque floor), then draw each [`LayerRender`] bottom→top as a textured quad
//! with its [`Mat3`] affine, [`CropRect`], opacity, and **premultiplied-alpha blend**
//! (premultiply on upload, `pixels.rs`). Color is a single BT.709 working space
//! (risk #5); straight-alpha sources are premultiplied so edges don't fringe (risk #3).
//!
//! ## Targets
//! - [`Compositor::new_for_surface`] — on-screen: a [`wgpu::Surface`] on a window's
//!   `HasWindowHandle` (the S-2 plan-A1 seam: wgpu draws on the window HWND, a
//!   transparent WebView2 child composites over it via DWM). The `palmier-tauri`
//!   preview module constructs this from the Tauri window handle.
//! - [`Compositor::new_headless`] — offscreen: renders into an owned `Rgba8Unorm`
//!   texture with no surface, so the pipeline + texture upload + a smoke-render are
//!   exercised in tests on a headless box (or skipped cleanly if no adapter).
//!
//! ## Frame resolution
//! [`Compositor::render`] takes a [`RenderFrame`] (the transport's
//! `TransportEvent::Render` payload) and a [`palmier_media::FrameSource`]. For each
//! visual layer it resolves the layer's [`FrameRef`] through the frame source
//! (`request_frame`), uploads/caches the decoded pixels as a premultiplied RGBA
//! texture (1.5 GB VRAM LRU, `texture_cache.rs`), and draws the quad. Text/Lottie
//! handling: see the per-variant notes in [`Compositor::render`] (Text + Lottie are
//! **stubbed** this story, deferred to E5-S9).

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use palmier_media::SeekMode;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use wgpu::util::DeviceExt;

use crate::composition::{CropRect, LayerRender};
use crate::preview::RenderFrame;
use crate::Mat3;

use super::pixels::{decoded_to_rgba, RgbaImage};
use super::provider::FrameProvider;
use super::quad::{crop_corners, crop_uv_rect, layer_clip_matrix, CanvasSize};
use super::text_pass::{TextDraw, TextPass};
use super::texture_cache::{TexKey, TextureCache};
use palmier_text::FontRegistry;

/// The single working pixel format. BGRA on a surface (the native swapchain format
/// DWM wants); RGBA for the headless offscreen target. Both are 8-bit, non-sRGB so
/// the BT.709 working space stays linear-ish for v1 (color-managed pass is future).
const HEADLESS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Per-layer uniform: the source-pixel→clip matrix + opacity. `#[repr(C)]` + `Pod`
/// so it uploads as raw bytes; the trailing pad matches the WGSL `LayerUniform`
/// 16-byte alignment.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LayerUniform {
    clip: [f32; 16],
    opacity: f32,
    _pad: [f32; 3],
}

/// One quad vertex: a source-pixel corner + its texture coordinate.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    corner: [f32; 2],
    uv: [f32; 2],
}

/// Where the compositor presents.
enum Target {
    /// On-screen swapchain on a window's handle (S-2 plan A1).
    Surface {
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    },
    /// Offscreen owned texture (headless tests / smoke render).
    Offscreen {
        texture: wgpu::Texture,
        view: wgpu::TextureView,
        width: u32,
        height: u32,
    },
}

/// Errors standing up or running the compositor.
#[derive(Debug)]
pub enum CompositorError {
    /// No GPU adapter could be acquired (headless CI with no device → tests skip).
    NoAdapter(String),
    /// Device request failed.
    Device(String),
    /// Surface creation failed (bad window handle).
    Surface(String),
    /// A decode request through the frame source failed.
    Decode(String),
    /// Acquiring the next swapchain texture failed.
    Present(wgpu::SurfaceError),
}

impl std::fmt::Display for CompositorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompositorError::NoAdapter(s) => write!(f, "no GPU adapter: {s}"),
            CompositorError::Device(s) => write!(f, "device request failed: {s}"),
            CompositorError::Surface(s) => write!(f, "surface creation failed: {s}"),
            CompositorError::Decode(s) => write!(f, "frame decode failed: {s}"),
            CompositorError::Present(e) => write!(f, "present failed: {e:?}"),
        }
    }
}

impl std::error::Error for CompositorError {}

/// The wgpu compositor: device/queue/pipeline + the VRAM texture cache + the present
/// target. Construct once per preview surface; call [`Compositor::render`] per
/// `TransportEvent::Render`.
pub struct Compositor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    texture_cache: TextureCache<wgpu::Texture>,
    target: Target,
    /// The swapchain/target format (BGRA for a surface, RGBA headless).
    format: wgpu::TextureFormat,
    adapter_info: wgpu::AdapterInfo,
    /// Font registry (bundled + system fonts) the text pass rasterizes through.
    /// Built once with the compositor so the `FontSystem` (expensive system scan)
    /// is shared across every frame's text layers (E5-S9).
    font_registry: FontRegistry,
    /// The wgpu glyph text pass (E5-S9), built lazily on the first text frame so a
    /// composition with no text never pays for the atlas/pipelines.
    text_pass: Option<TextPass>,
}

impl Compositor {
    /// Acquire an instance + adapter + device for `backends`, optionally compatible
    /// with `surface`. Shared by both constructors.
    fn acquire_device(
        backends: wgpu::Backends,
        surface: Option<&wgpu::Surface<'static>>,
    ) -> Result<(wgpu::Instance, wgpu::Adapter, wgpu::Device, wgpu::Queue), CompositorError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: surface,
        }))
        .map_err(|e| CompositorError::NoAdapter(format!("{e:?}")))?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("palmier-compositor-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| CompositorError::Device(format!("{e:?}")))?;

        // Keep `instance` alive for the caller (surface creation needs it before this).
        Ok((instance, adapter, device, queue))
    }

    /// Build the shared pipeline + bind-group layout + sampler for `format`.
    fn build_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("composite-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("composite.wgsl").into()),
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("composite-bgl"),
                entries: &[
                    // layer uniform
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // texture
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // sampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("composite-pl"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: 8,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("composite-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[vertex_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // Premultiplied-alpha blend: source RGBA is already premultiplied
                    // (pixels.rs), opacity-scaled in the shader → composites bottom→top.
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("composite-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        (pipeline, bind_group_layout, sampler)
    }

    /// On-screen compositor: create a wgpu surface on `window` (its
    /// `HasWindowHandle` + `HasDisplayHandle` — the S-2 plan-A1 seam) sized
    /// `width × height`. `force_dx12` pins the Windows production backend (DX12),
    /// matching the spike's proven path; pass `false` to let wgpu choose.
    pub fn new_for_surface<W>(
        window: Arc<W>,
        width: u32,
        height: u32,
        force_dx12: bool,
    ) -> Result<Self, CompositorError>
    where
        W: HasWindowHandle + HasDisplayHandle + Send + Sync + 'static,
    {
        let backends = if force_dx12 {
            wgpu::Backends::DX12
        } else {
            wgpu::Backends::all()
        };
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });
        let surface = instance
            .create_surface(window)
            .map_err(|e| CompositorError::Surface(format!("{e:?}")))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .map_err(|e| CompositorError::NoAdapter(format!("{e:?}")))?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("palmier-compositor-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| CompositorError::Device(format!("{e:?}")))?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm))
            .unwrap_or(caps.formats[0]);
        // Prefer a non-opaque composite-alpha so the wgpu layer can be partly
        // transparent (lets the webview/desktop show through where alpha < 1).
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

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let (pipeline, bind_group_layout, sampler) = Self::build_pipeline(&device, format);

        Ok(Compositor {
            adapter_info: adapter.get_info(),
            device,
            queue,
            pipeline,
            bind_group_layout,
            sampler,
            texture_cache: TextureCache::new(),
            target: Target::Surface { surface, config },
            format,
            font_registry: FontRegistry::with_bundled_fonts(),
            text_pass: None,
        })
    }

    /// Headless compositor rendering into an owned `width × height` `Rgba8Unorm`
    /// texture (no window/surface). Used by tests + a smoke render; returns
    /// [`CompositorError::NoAdapter`] cleanly when the box has no GPU so the caller
    /// can skip.
    pub fn new_headless(width: u32, height: u32) -> Result<Self, CompositorError> {
        let (_instance, adapter, device, queue) =
            Self::acquire_device(wgpu::Backends::all(), None)?;

        let format = HEADLESS_FORMAT;
        let (pipeline, bind_group_layout, sampler) = Self::build_pipeline(&device, format);
        let (texture, view) = Self::make_offscreen(&device, width, height, format);

        Ok(Compositor {
            adapter_info: adapter.get_info(),
            device,
            queue,
            pipeline,
            bind_group_layout,
            sampler,
            texture_cache: TextureCache::new(),
            target: Target::Offscreen {
                texture,
                view,
                width: width.max(1),
                height: height.max(1),
            },
            format,
            // Tests/headless use bundled-only fonts (deterministic, no system scan).
            font_registry: FontRegistry::bundled_only(),
            text_pass: None,
        })
    }

    fn make_offscreen(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("composite-offscreen"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    /// Human-readable adapter line (backend | name | type) for diagnostics/proof.
    pub fn adapter_summary(&self) -> String {
        format!(
            "{:?} | {} | {:?} | format={:?}",
            self.adapter_info.backend,
            self.adapter_info.name,
            self.adapter_info.device_type,
            self.format
        )
    }

    /// VRAM texture-cache occupancy snapshot.
    pub fn cache_stats(&self) -> super::texture_cache::TexCacheStats {
        self.texture_cache.stats()
    }

    /// Resize the present target (window resize / quality-scale change).
    pub fn resize(&mut self, width: u32, height: u32) {
        let (w, h) = (width.max(1), height.max(1));
        match &mut self.target {
            Target::Surface { surface, config } => {
                config.width = w;
                config.height = h;
                surface.configure(&self.device, config);
            }
            Target::Offscreen { texture, view, width, height } => {
                let (t, v) = Self::make_offscreen(&self.device, w, h, self.format);
                *texture = t;
                *view = v;
                *width = w;
                *height = h;
            }
        }
    }

    /// Drop every cached texture for `media_ref` (asset removed / source edited).
    pub fn evict_asset(&mut self, media_ref: &str) {
        self.texture_cache.evict_asset(media_ref);
    }

    /// Upload a decoded frame's pixels as a premultiplied-RGBA texture, caching it
    /// under its `(media_ref, source_frame)` key. Returns the cached texture's view.
    /// Honors the 1.5 GB VRAM ceiling (LRU eviction); an oversize frame is uploaded
    /// transiently (not cached).
    fn upload_layer_texture(&mut self, key: TexKey, img: &RgbaImage) -> Option<wgpu::Texture> {
        if img.width == 0 || img.height == 0 {
            return None;
        }
        // Fast path: already resident.
        if self.texture_cache.get(&key).is_some() {
            // The cache holds it; clone the handle (wgpu textures are cheap Arc clones).
            return self.texture_cache.get(&key).cloned();
        }

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("layer-texture"),
            size: wgpu::Extent3d {
                width: img.width,
                height: img.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &img.bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(img.row_bytes()),
                rows_per_image: Some(img.height),
            },
            wgpu::Extent3d {
                width: img.width,
                height: img.height,
                depth_or_array_layers: 1,
            },
        );

        let bytes = (img.width as u64) * (img.height as u64) * 4;
        match self.texture_cache.insert(key.clone(), texture, bytes) {
            Ok(()) => self.texture_cache.get(&key).cloned(),
            // Oversize for the whole ceiling: use it transiently (handed back here).
            Err(t) => Some(t),
        }
    }

    /// Resolve a layer's frame, upload/cache its texture, return the (texture, image
    /// size) to draw. `None` if the decode fails or the frame is empty (skip layer).
    fn resolve_layer_texture<P: FrameProvider>(
        &mut self,
        provider: &P,
        media_ref: &str,
        source_frame: u64,
        active_layers: u32,
    ) -> Result<Option<(wgpu::Texture, u32, u32)>, CompositorError> {
        let key = TexKey::new(media_ref, source_frame);
        // If cached, we still need the image size for the quad; we recover it from the
        // texture dimensions (cheap) rather than re-decoding.
        if let Some(tex) = self.texture_cache.get(&key).cloned() {
            let (w, h) = (tex.width(), tex.height());
            return Ok(Some((tex, w, h)));
        }
        // Miss: decode through the provider (one-decode-owner; engine never opens FFmpeg).
        let decoded = provider
            .provide_frame(media_ref, source_frame, SeekMode::Exact, active_layers)
            .map_err(|e| CompositorError::Decode(format!("{e:?}")))?;
        let img = decoded_to_rgba(&decoded);
        let (w, h) = (img.width, img.height);
        Ok(self.upload_layer_texture(key, &img).map(|t| (t, w, h)))
    }

    /// Render + present one [`RenderFrame`]. Clears to black, then draws each visual
    /// layer bottom→top as a textured quad. `frame_source` resolves each layer's
    /// pixels (one-decode-owner). On a surface this presents to the swapchain; on the
    /// headless target it renders into the owned texture (read back via
    /// [`Compositor::read_back`]).
    ///
    /// Text + Lottie layers are **stubbed** this story (E5-S9): Text is skipped
    /// (rendered by the text pass later); Lottie is treated as a normal texture layer
    /// **if** `palmier-media` already pre-rendered it to a frame (the build models it
    /// as a `FrameRef`), else skipped. See the per-variant match below.
    pub fn render<P: FrameProvider>(
        &mut self,
        frame: &RenderFrame,
        frame_source: &P,
    ) -> Result<(), CompositorError> {
        let canvas = CanvasSize::new(frame.canvas.width, frame.canvas.height);
        let active_layers = frame.composition.layers.len().max(1) as u32;

        // Acquire the target view (+ optional surface texture to present).
        let (view, surface_texture) = match &self.target {
            Target::Surface { surface, .. } => {
                let st = surface.get_current_texture().map_err(CompositorError::Present)?;
                let view = st.texture.create_view(&wgpu::TextureViewDescriptor::default());
                (view, Some(st))
            }
            Target::Offscreen { view, .. } => {
                let v = view.clone();
                (v, None)
            }
        };

        // Resolve + upload every layer's texture BEFORE opening the render pass
        // (write_texture + the pass can't interleave; and resolve borrows &mut self).
        let mut draws: Vec<DrawItem> = Vec::new();
        // Text layers are collected here (they composite ABOVE the video stack —
        // reference text `CALayer` tree over the `AVPlayerLayer`); their glyph
        // rasterization + atlas upload also happens before the pass.
        let text_layers: Vec<&crate::TextLayer> = frame
            .composition
            .layers
            .iter()
            .filter_map(|l| match l {
                LayerRender::Text(t) => Some(t),
                _ => None,
            })
            .collect();

        for layer in &frame.composition.layers {
            let (visual, transform, opacity, crop) = match layer {
                LayerRender::Video(v) | LayerRender::Image(v) => {
                    (v, v.transform, v.opacity, v.crop)
                }
                LayerRender::Lottie(v) => {
                    // Lottie is modeled as a FrameRef (palmier-media pre-renders it to
                    // a texture, #22). Treat it as a normal texture layer; if its
                    // source isn't resolvable yet it just skips (decode error → skip).
                    (v, v.transform, v.opacity, v.crop)
                }
                LayerRender::Text(_) => {
                    // Text is rendered by the glyph pass (collected above), above the
                    // video quads — not as a composition texture. Handled post-loop.
                    continue;
                }
            };

            let resolved = self.resolve_layer_texture(
                frame_source,
                &visual.frame.media_ref,
                visual.frame.source_frame,
                active_layers,
            );
            let (texture, tex_w, tex_h) = match resolved {
                Ok(Some(t)) => t,
                Ok(None) => continue, // empty/oversize-skip
                // A missing/offline source must not kill the whole frame — skip the layer.
                Err(_) => continue,
            };

            // The quad covers the cropped source region; the affine maps it to render
            // pixels then NDC. natural_size is the decoded display size; fall back to
            // the texture size if the build didn't carry it.
            let (nat_w, nat_h) = if visual.natural_size.0 > 0.0 && visual.natural_size.1 > 0.0 {
                visual.natural_size
            } else {
                (tex_w as f64, tex_h as f64)
            };
            let params = LayerDraw {
                transform,
                opacity,
                crop: clamp_crop(crop, nat_w, nat_h),
                nat_w,
                nat_h,
            };
            let item = self.build_draw_item(&params, &texture, canvas);
            draws.push(item);
        }

        // Prepare text-layer draws (E5-S9): rasterize any new glyphs into the atlas
        // (queue.write_texture — must run BEFORE the render pass) and build the
        // per-glyph/box quads. The text pass is built lazily on the first text frame.
        let mut text_draws: Vec<TextDraw> = Vec::new();
        if !text_layers.is_empty() {
            // Lazily stand up the text pass for this target's format.
            if self.text_pass.is_none() {
                self.text_pass = Some(TextPass::new(&self.device, self.format));
            }
            // Disjoint field borrows: text_pass + font_registry + device/queue.
            let pass = self.text_pass.as_mut().expect("text pass present");
            for t in &text_layers {
                let mut d = pass.prepare_layer(
                    &self.device,
                    &self.queue,
                    &mut self.font_registry,
                    &t.run,
                    t.opacity,
                    canvas,
                );
                text_draws.append(&mut d);
            }
        }

        // One render pass: clear to black (opaque floor), draw layers bottom→top.
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("composite-encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Black opaque floor (reference's black background track / risk #2).
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            for d in &draws {
                pass.set_bind_group(0, &d.bind_group, &[]);
                pass.set_vertex_buffer(0, d.vertex_buf.slice(..));
                pass.draw(0..4, 0..1); // triangle-strip quad
            }

            // Text pass: draw glyph/background/border/shadow quads ABOVE the video
            // (reference text tree over the player layer). Same render pass, same
            // premultiplied-alpha blend.
            if let Some(tp) = self.text_pass.as_ref() {
                tp.draw(&mut pass, &text_draws);
            }
        }
        self.queue.submit(std::iter::once(encoder.finish()));

        if let Some(st) = surface_texture {
            st.present(); // DWM composites under the transparent WebView2 child (A1).
        }
        Ok(())
    }

    /// Build the per-layer GPU resources (uniform + vertex buffer + bind group) from
    /// the layer's sampled geometry ([`LayerDraw`]), its uploaded texture, and the
    /// canvas size.
    fn build_draw_item(
        &self,
        params: &LayerDraw,
        texture: &wgpu::Texture,
        canvas: CanvasSize,
    ) -> DrawItem {
        let LayerDraw { transform, opacity, crop, nat_w, nat_h } = *params;
        let clip = layer_clip_matrix(transform, canvas);
        let uv = crop_uv_rect(crop, nat_w, nat_h);
        let corners = crop_corners(crop);

        // Triangle-strip order: TL, TR, BL, BR (corners are TL,TR,BL,BR already).
        let verts = [
            Vertex { corner: [corners[0].0 as f32, corners[0].1 as f32], uv: [uv.u0, uv.v0] },
            Vertex { corner: [corners[1].0 as f32, corners[1].1 as f32], uv: [uv.u1, uv.v0] },
            Vertex { corner: [corners[2].0 as f32, corners[2].1 as f32], uv: [uv.u0, uv.v1] },
            Vertex { corner: [corners[3].0 as f32, corners[3].1 as f32], uv: [uv.u1, uv.v1] },
        ];

        let uniform = LayerUniform {
            clip,
            opacity: opacity.clamp(0.0, 1.0) as f32,
            _pad: [0.0; 3],
        };

        let uniform_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("layer-uniform"),
                contents: bytemuck::bytes_of(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let vertex_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("layer-verts"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let tex_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("layer-bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&tex_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        DrawItem { uniform_buf, vertex_buf, bind_group }
    }

    /// Read back the headless offscreen target as premultiplied RGBA8 bytes (row-
    /// major, unpadded). Returns `None` on a surface target. Used by the smoke test
    /// to confirm pixels were produced (e.g. the black floor + a layer).
    pub fn read_back(&self) -> Option<RgbaImage> {
        let (texture, width, height) = match &self.target {
            Target::Offscreen { texture, width, height, .. } => (texture, *width, *height),
            Target::Surface { .. } => return None,
        };

        // 256-byte row alignment required for COPY (wgpu).
        let unpadded = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded = unpadded.div_ceil(align) * align;

        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (padded * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("readback-enc") });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit(std::iter::once(encoder.finish()));

        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self
            .device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        rx.recv().ok()?.ok()?;

        let data = slice.get_mapped_range();
        let mut bytes = Vec::with_capacity((unpadded * height) as usize);
        for row in 0..height {
            let start = (row * padded) as usize;
            bytes.extend_from_slice(&data[start..start + unpadded as usize]);
        }
        drop(data);
        buffer.unmap();
        Some(RgbaImage { width, height, bytes })
    }
}

/// The sampled per-layer geometry the quad draw needs: the source-pixel→render affine,
/// effective opacity, the (clamped) crop rect, and the source's natural size (for the
/// crop→UV divide). Grouped so `build_draw_item` stays a 3-arg call.
struct LayerDraw {
    transform: Mat3,
    opacity: f64,
    crop: CropRect,
    nat_w: f64,
    nat_h: f64,
}

/// Per-layer GPU resources for one quad draw (uniform + vertex buffer + bind group).
struct DrawItem {
    #[allow(dead_code)] // held to keep the buffer alive for the bind group's lifetime
    uniform_buf: wgpu::Buffer,
    vertex_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// Clamp a crop rect to the source frame bounds, flooring extent to ≥ 1 px (matches
/// the build/reference clamp so a degenerate crop never produces a zero-area quad).
fn clamp_crop(crop: CropRect, nat_w: f64, nat_h: f64) -> CropRect {
    let x = crop.x.clamp(0.0, (nat_w - 1.0).max(0.0));
    let y = crop.y.clamp(0.0, (nat_h - 1.0).max(0.0));
    let width = crop.width.clamp(1.0, nat_w - x);
    let height = crop.height.clamp(1.0, nat_h - y);
    CropRect { x, y, width, height }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_crop_keeps_in_bounds() {
        let c = clamp_crop(CropRect { x: -5.0, y: -5.0, width: 9999.0, height: 9999.0 }, 100.0, 80.0);
        assert_eq!(c.x, 0.0);
        assert_eq!(c.y, 0.0);
        assert_eq!(c.width, 100.0);
        assert_eq!(c.height, 80.0);
    }

    #[test]
    fn layer_uniform_is_pod_sized() {
        // 16 floats matrix + 4 floats (opacity+pad) = 80 bytes, 16-aligned.
        assert_eq!(std::mem::size_of::<LayerUniform>(), 80);
        assert_eq!(std::mem::size_of::<Vertex>(), 16);
    }
}
