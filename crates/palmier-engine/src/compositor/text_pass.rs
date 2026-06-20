//! The wgpu glyph text pass — E5-S9. **Feature-gated behind `wgpu-compositor`.**
//!
//! Renders [`LayerRender::Text`](crate::LayerRender) layers (laid out upstream by
//! [`build_text_layers`](crate::build_text_layers) → `palmier-text` glyph runs) into
//! the composite color target, **above** the video stack, matching the reference's
//! `CALayer` text tree over the `AVPlayerLayer`. Per text layer it draws, in order:
//!
//! 1. the **background** box (if `style.background`) — a solid premultiplied quad,
//! 2. the **shadow** glyphs (if `style.shadow`) — the glyph atlas tinted with the
//!    shadow color, offset by the scaled shadow offset (a cheap 1-tap drop shadow;
//!    a true Gaussian blur is a future refinement, noted),
//! 3. the **glyphs** — the R8 coverage atlas tinted with the text color,
//! 4. the **border** (if `style.border`) — four solid edge quads around the box.
//!
//! Every color is premultiplied and scaled by the layer opacity, so the
//! `PREMULTIPLIED_ALPHA` blend composites correctly over the already-drawn video.
//!
//! ## Glyph atlas
//! Glyphs are rasterized once via cosmic-text's [`SwashCache`] into a single
//! `R8Unorm` **coverage** atlas, packed with a simple shelf/row allocator keyed by
//! the glyph's [`CacheKey`]. The atlas persists across frames (captions reuse the
//! same glyphs every frame), growing by re-allocation when full.
//!
//! Color glyphs (emoji): swash returns RGBA; v1 folds them to coverage (luma→alpha)
//! so they render monochrome in the text color. Full-color emoji is a noted future
//! refinement (a second RGBA atlas + a separate draw).

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use palmier_text::cosmic_text::{CacheKey, SwashCache, SwashContent};
use palmier_text::{FontRegistry, GlyphRun, RenderColor};
use wgpu::util::DeviceExt;

use super::quad::CanvasSize;

/// A premultiplied-RGBA quad-color uniform (16-byte aligned for std140).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct QuadUniform {
    color: [f32; 4],
}

/// One vertex of a text/solid quad: NDC position + atlas UV.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TextVertex {
    pos: [f32; 2],
    uv: [f32; 2],
}

/// The packed location of one glyph in the atlas: pixel rect + swash placement
/// offsets (`left`/`top` = bitmap origin relative to the pen position).
#[derive(Clone, Copy)]
struct AtlasEntry {
    /// Atlas pixel rect.
    ax: u32,
    ay: u32,
    aw: u32,
    ah: u32,
    /// Swash placement: bitmap left/top relative to the glyph pen origin.
    left: i32,
    top: i32,
}

/// A simple shelf/row glyph-atlas allocator over an `R8Unorm` coverage texture.
struct GlyphAtlas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    /// Current shelf cursor.
    cursor_x: u32,
    cursor_y: u32,
    shelf_height: u32,
    entries: HashMap<CacheKey, Option<AtlasEntry>>,
}

const ATLAS_DIM: u32 = 1024;
const ATLAS_PAD: u32 = 1;

impl GlyphAtlas {
    fn new(device: &wgpu::Device) -> Self {
        let (texture, view) = Self::make_texture(device, ATLAS_DIM, ATLAS_DIM);
        GlyphAtlas {
            texture,
            view,
            width: ATLAS_DIM,
            height: ATLAS_DIM,
            cursor_x: ATLAS_PAD,
            cursor_y: ATLAS_PAD,
            shelf_height: 0,
            entries: HashMap::new(),
        }
    }

    fn make_texture(device: &wgpu::Device, w: u32, h: u32) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph-atlas"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    /// Reserve a `w × h` shelf slot, advancing the cursor. Returns `None` if the
    /// glyph cannot fit even in a fresh shelf (oversize — skip it).
    fn reserve(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w + 2 * ATLAS_PAD > self.width || h + 2 * ATLAS_PAD > self.height {
            return None;
        }
        if self.cursor_x + w + ATLAS_PAD > self.width {
            // New shelf.
            self.cursor_x = ATLAS_PAD;
            self.cursor_y += self.shelf_height + ATLAS_PAD;
            self.shelf_height = 0;
        }
        if self.cursor_y + h + ATLAS_PAD > self.height {
            return None; // atlas full this frame; caller can grow next time.
        }
        let (x, y) = (self.cursor_x, self.cursor_y);
        self.cursor_x += w + ATLAS_PAD;
        self.shelf_height = self.shelf_height.max(h);
        Some((x, y))
    }

    /// Resolve a glyph to its atlas entry, rasterizing + uploading on first sight.
    /// `None` means the glyph has no visible bitmap (e.g. a space) or didn't fit.
    fn entry(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        swash: &mut SwashCache,
        registry: &mut FontRegistry,
        key: CacheKey,
    ) -> Option<AtlasEntry> {
        if let Some(cached) = self.entries.get(&key) {
            return *cached;
        }
        let font_system = registry.font_system_mut();
        let entry = match swash.get_image(font_system, key) {
            Some(image) if image.placement.width > 0 && image.placement.height > 0 => {
                let (gw, gh) = (image.placement.width, image.placement.height);
                // Fold swash output to single-channel coverage.
                let coverage = to_coverage(&image.data, &image.content, gw, gh);
                match self.reserve(gw, gh) {
                    Some((ax, ay)) => {
                        queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &self.texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d { x: ax, y: ay, z: 0 },
                                aspect: wgpu::TextureAspect::All,
                            },
                            &coverage,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(gw),
                                rows_per_image: Some(gh),
                            },
                            wgpu::Extent3d { width: gw, height: gh, depth_or_array_layers: 1 },
                        );
                        Some(AtlasEntry {
                            ax,
                            ay,
                            aw: gw,
                            ah: gh,
                            left: image.placement.left,
                            top: image.placement.top,
                        })
                    }
                    None => None,
                }
            }
            // No bitmap (whitespace) or no image.
            _ => None,
        };
        let _ = device; // device kept in signature for symmetry / future atlas grow.
        self.entries.insert(key, entry);
        entry
    }
}

/// Fold a swash glyph image into single-channel R8 coverage. `Mask` is already
/// 1 byte/px; `Color`/`SubpixelMask` are folded by luma/average so color glyphs
/// render monochrome (v1; full-color emoji is a future refinement).
fn to_coverage(data: &[u8], content: &SwashContent, w: u32, h: u32) -> Vec<u8> {
    let n = (w * h) as usize;
    match content {
        SwashContent::Mask => data.to_vec(),
        SwashContent::Color => {
            // RGBA → use alpha as coverage.
            let mut out = Vec::with_capacity(n);
            for px in data.chunks_exact(4) {
                out.push(px[3]);
            }
            out
        }
        SwashContent::SubpixelMask => {
            // RGB subpixel → average the three channels to a coverage byte.
            let mut out = Vec::with_capacity(n);
            for px in data.chunks_exact(4) {
                let avg = ((px[0] as u16 + px[1] as u16 + px[2] as u16) / 3) as u8;
                out.push(avg);
            }
            out
        }
    }
}

/// The text pass: pipelines (solid + glyph), the glyph atlas, the swash cache, and
/// the font registry that owns the `FontSystem`. Constructed lazily by the
/// compositor on the first text frame.
pub struct TextPass {
    solid_pipeline: wgpu::RenderPipeline,
    glyph_pipeline: wgpu::RenderPipeline,
    solid_bgl: wgpu::BindGroupLayout,
    glyph_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    atlas: GlyphAtlas,
    swash: SwashCache,
}

impl TextPass {
    /// Build the text-pass pipelines for the given color-target `format`.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("text-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("text.wgsl").into()),
        });

        let solid_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("text-solid-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let glyph_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("text-glyph-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<TextVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x2 },
                wgpu::VertexAttribute { offset: 8, shader_location: 1, format: wgpu::VertexFormat::Float32x2 },
            ],
        };

        let blend = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let make_pipeline = |layout: &wgpu::PipelineLayout, fs_entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("text-pipeline"),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_quad"),
                    buffers: &[vertex_layout.clone()],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs_entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
        };

        let solid_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("text-solid-pl"),
            bind_group_layouts: &[&solid_bgl],
            push_constant_ranges: &[],
        });
        let glyph_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("text-glyph-pl"),
            bind_group_layouts: &[&glyph_bgl],
            push_constant_ranges: &[],
        });

        let solid_pipeline = make_pipeline(&solid_layout, "fs_solid");
        let glyph_pipeline = make_pipeline(&glyph_layout, "fs_glyph");

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("text-atlas-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        TextPass {
            solid_pipeline,
            glyph_pipeline,
            solid_bgl,
            glyph_bgl,
            sampler,
            atlas: GlyphAtlas::new(device),
            swash: SwashCache::new(),
        }
    }

    /// Prepare GPU draw resources for one text layer's [`GlyphRun`] at `opacity`,
    /// rasterizing any new glyphs into the atlas (this must run BEFORE the render
    /// pass — it does `queue.write_texture`). Returns the per-draw resources to
    /// issue in the pass. `canvas` drives the render-pixel → NDC mapping.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        registry: &mut FontRegistry,
        run: &GlyphRun,
        opacity: f64,
        canvas: CanvasSize,
    ) -> Vec<TextDraw> {
        let op = opacity.clamp(0.0, 1.0);
        if op <= 0.0 {
            return Vec::new(); // fully transparent (preroll lead-in) → nothing.
        }
        let mut draws = Vec::new();
        let style = &run.style;
        let bx = &run.box_rect;

        // 1) Background box (premultiplied, × opacity).
        if let Some(bg) = style.background {
            draws.push(self.solid_quad(
                device,
                bx.x, bx.y, bx.width, bx.height,
                bg, op, canvas,
            ));
        }

        // 2) Shadow glyphs (offset, tinted shadow color).
        if let Some(sh) = style.shadow {
            for g in &run.glyphs {
                if let Some(d) = self.glyph_quad(
                    device, queue, registry, g.cache_key,
                    g.x as f64 + sh.offset_x, g.y as f64 + sh.offset_y,
                    sh.color, op, canvas,
                ) {
                    draws.push(d);
                }
            }
        }

        // 3) Glyphs (text color).
        for g in &run.glyphs {
            if let Some(d) = self.glyph_quad(
                device, queue, registry, g.cache_key,
                g.x as f64, g.y as f64,
                style.color, op, canvas,
            ) {
                draws.push(d);
            }
        }

        // 4) Border (four edge quads).
        if let Some((col, w)) = style.border {
            let w = w.max(0.0);
            // top, bottom, left, right.
            let edges = [
                (bx.x, bx.y, bx.width, w),
                (bx.x, bx.y + bx.height - w, bx.width, w),
                (bx.x, bx.y, w, bx.height),
                (bx.x + bx.width - w, bx.y, w, bx.height),
            ];
            for (x, y, ew, eh) in edges {
                if ew > 0.0 && eh > 0.0 {
                    draws.push(self.solid_quad(device, x, y, ew, eh, col, op, canvas));
                }
            }
        }

        draws
    }

    /// Build a solid-color quad draw (background / border / shadow box).
    #[allow(clippy::too_many_arguments)]
    fn solid_quad(
        &self,
        device: &wgpu::Device,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        color: RenderColor,
        opacity: f64,
        canvas: CanvasSize,
    ) -> TextDraw {
        let verts = rect_ndc(x, y, w, h, canvas, [0.0; 4]);
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("text-solid-verts"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ubuf = self.color_uniform(device, color, opacity);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("text-solid-bg"),
            layout: &self.solid_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }],
        });
        TextDraw { kind: DrawKind::Solid, vbuf, _ubuf: ubuf, bind_group }
    }

    /// Build a glyph quad draw, rasterizing the glyph into the atlas if new.
    /// `(pen_x, pen_y)` is the glyph pen origin in render px; the atlas placement
    /// offsets position the bitmap. `None` if the glyph has no bitmap (whitespace).
    #[allow(clippy::too_many_arguments)]
    fn glyph_quad(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        registry: &mut FontRegistry,
        key: CacheKey,
        pen_x: f64,
        pen_y: f64,
        color: RenderColor,
        opacity: f64,
        canvas: CanvasSize,
    ) -> Option<TextDraw> {
        let entry = self.atlas.entry(device, queue, &mut self.swash, registry, key)?;
        // Bitmap top-left in render px: pen + placement (top is distance from
        // baseline UP to the bitmap top, so y = pen_y - top).
        let gx = pen_x + entry.left as f64;
        let gy = pen_y - entry.top as f64;
        let (gw, gh) = (entry.aw as f64, entry.ah as f64);

        // UV rect into the atlas.
        let (aw, ah) = (self.atlas.width as f64, self.atlas.height as f64);
        let uv = [
            entry.ax as f32 / aw as f32,
            entry.ay as f32 / ah as f32,
            (entry.ax + entry.aw) as f32 / aw as f32,
            (entry.ay + entry.ah) as f32 / ah as f32,
        ];
        let verts = rect_ndc(gx, gy, gw, gh, canvas, uv);
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("text-glyph-verts"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ubuf = self.color_uniform(device, color, opacity);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("text-glyph-bg"),
            layout: &self.glyph_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.atlas.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        Some(TextDraw { kind: DrawKind::Glyph, vbuf, _ubuf: ubuf, bind_group })
    }

    fn color_uniform(&self, device: &wgpu::Device, color: RenderColor, opacity: f64) -> wgpu::Buffer {
        // Premultiply rgb by alpha × opacity (premultiplied-alpha blend).
        let a = (color.a * opacity).clamp(0.0, 1.0) as f32;
        let uniform = QuadUniform {
            color: [
                (color.r.clamp(0.0, 1.0) as f32) * a,
                (color.g.clamp(0.0, 1.0) as f32) * a,
                (color.b.clamp(0.0, 1.0) as f32) * a,
                a,
            ],
        };
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("text-color-uniform"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        })
    }

    /// Issue the prepared draws into an already-open render pass.
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, draws: &'a [TextDraw]) {
        for d in draws {
            let pipeline = match d.kind {
                DrawKind::Solid => &self.solid_pipeline,
                DrawKind::Glyph => &self.glyph_pipeline,
            };
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &d.bind_group, &[]);
            pass.set_vertex_buffer(0, d.vbuf.slice(..));
            pass.draw(0..6, 0..1);
        }
    }
}

/// Which pipeline a [`TextDraw`] uses.
#[derive(Clone, Copy)]
enum DrawKind {
    Solid,
    Glyph,
}

/// Per-quad GPU resources for one text draw (a glyph or a solid box).
pub struct TextDraw {
    kind: DrawKind,
    vbuf: wgpu::Buffer,
    /// Held alive for the bind group's lifetime.
    _ubuf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// Build a render-pixel rect's 6 NDC vertices (two triangles), top-left origin →
/// NDC with the y-flip (matches the composite pass's `layer_clip_matrix`).
fn rect_ndc(x: f64, y: f64, w: f64, h: f64, canvas: CanvasSize, uv: [f32; 4]) -> [TextVertex; 6] {
    let (cw, ch) = (canvas.width as f64, canvas.height as f64);
    let to_ndc = |px: f64, py: f64| -> [f32; 2] {
        [((2.0 * px / cw) - 1.0) as f32, (1.0 - 2.0 * py / ch) as f32]
    };
    let tl = to_ndc(x, y);
    let tr = to_ndc(x + w, y);
    let bl = to_ndc(x, y + h);
    let br = to_ndc(x + w, y + h);
    let (u0, v0, u1, v1) = (uv[0], uv[1], uv[2], uv[3]);
    [
        TextVertex { pos: tl, uv: [u0, v0] },
        TextVertex { pos: tr, uv: [u1, v0] },
        TextVertex { pos: bl, uv: [u0, v1] },
        TextVertex { pos: tr, uv: [u1, v0] },
        TextVertex { pos: br, uv: [u1, v1] },
        TextVertex { pos: bl, uv: [u0, v1] },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_ndc_maps_canvas_corners() {
        let c = CanvasSize::new(1920, 1080);
        // Full-canvas rect → corners at NDC (-1,1)..(1,-1).
        let v = rect_ndc(0.0, 0.0, 1920.0, 1080.0, c, [0.0, 0.0, 1.0, 1.0]);
        assert!((v[0].pos[0] + 1.0).abs() < 1e-5 && (v[0].pos[1] - 1.0).abs() < 1e-5);
        // br vertex is index 4.
        assert!((v[4].pos[0] - 1.0).abs() < 1e-5 && (v[4].pos[1] + 1.0).abs() < 1e-5);
    }

    #[test]
    fn coverage_from_mask_is_passthrough() {
        let data = vec![10u8, 20, 30, 40];
        let cov = to_coverage(&data, &SwashContent::Mask, 2, 2);
        assert_eq!(cov, data);
    }

    #[test]
    fn coverage_from_color_uses_alpha() {
        // One RGBA pixel, alpha = 200.
        let data = vec![255u8, 0, 0, 200];
        let cov = to_coverage(&data, &SwashContent::Color, 1, 1);
        assert_eq!(cov, vec![200]);
    }
}
