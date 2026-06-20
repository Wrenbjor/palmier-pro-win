//! Candidate (c) — IPC readback fallback.
//!
//! Copies the Rust-owned wgpu texture to a CPU buffer. In production the resulting RGBA
//! bytes would be pushed over a Tauri IPC channel (or a shared-memory fast path) to a
//! `<canvas>` `putImageData`/`texSubImage` in the webview. This module exists to MEASURE
//! the GPU->CPU->(IPC) cost so the spike can state, with a real number, when the fallback
//! is affordable vs. when it busts the SM-2 FPS floors (4K >= 30, 1080p60 >= 60).
//!
//! The known cliff: a full-resolution RGBA readback per frame stalls the GPU pipeline
//! (copy_texture_to_buffer + map_async + device.poll round-trip) and then the IPC/
//! serialization adds more. 4K BGRA8 = 3840*2160*4 = ~33.2 MB/frame; at 30 fps that's
//! ~1.0 GB/s of bus traffic + per-frame map stalls before IPC even starts.

use crate::RenderedFrame;
use crate::render::GpuContext;

/// wgpu requires buffer rows to be aligned to `COPY_BYTES_PER_ROW_ALIGNMENT` (256).
fn padded_bytes_per_row(width: u32, bytes_per_pixel: u32) -> u32 {
    let unpadded = width * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    unpadded.div_ceil(align) * align
}

/// Result of a readback: the tightly-packed RGBA bytes + how long the GPU->CPU round
/// trip took. The IPC leg is NOT included (that's frontend-bound) — this is the floor.
pub struct ReadbackResult {
    pub width: u32,
    pub height: u32,
    pub bytes_per_pixel: u32,
    pub pixels: Vec<u8>,
    pub gpu_to_cpu: std::time::Duration,
}

/// Copy the rendered frame to CPU memory and time the round trip.
pub fn read_frame_to_cpu(ctx: &GpuContext, frame: &RenderedFrame) -> ReadbackResult {
    let bpp = 4u32; // BGRA8
    let padded = padded_bytes_per_row(frame.width, bpp);
    let buffer_size = (padded * frame.height) as wgpu::BufferAddress;

    let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("s1-readback-staging"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let start = std::time::Instant::now();

    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("s1-readback-enc") });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &frame.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(frame.height),
            },
        },
        wgpu::Extent3d { width: frame.width, height: frame.height, depth_or_array_layers: 1 },
    );
    ctx.queue.submit(std::iter::once(encoder.finish()));

    // Map + poll round trip — this is the stall that makes the fallback expensive.
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        let _ = tx.send(res);
    });
    // On native, progress only advances when the device is polled.
    let _ = ctx.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    rx.recv().expect("map channel closed").expect("map_async failed");

    let mapped = slice.get_mapped_range();

    // Strip row padding into tightly-packed pixels (what we'd hand to the canvas).
    let unpadded = (frame.width * bpp) as usize;
    let mut pixels = Vec::with_capacity(unpadded * frame.height as usize);
    for row in 0..frame.height as usize {
        let begin = row * padded as usize;
        pixels.extend_from_slice(&mapped[begin..begin + unpadded]);
    }
    drop(mapped);
    staging.unmap();

    let gpu_to_cpu = start.elapsed();

    ReadbackResult { width: frame.width, height: frame.height, bytes_per_pixel: bpp, pixels, gpu_to_cpu }
}
