//! # palmier-gen
//!
//! The AI generation lifecycle for the Palmier Pro Windows/Linux port (FOUNDATION
//! §4 / §6.11; Epic 9). Fetches the live model catalog, validates a generation
//! request per-model, creates placeholder `MediaAsset`s, uploads reference files
//! via the 3-step Convex contract, submits a job, subscribes to it reactively
//! until it settles, downloads results into `<project>/media`, and gates the UI
//! on credit budget (the advisory `can_generate`).
//!
//! ## Architecture (transport-agnostic lifecycle)
//!
//! Everything sits ABOVE a single [`GenerationTransport`](transport::GenerationTransport)
//! trait so the lifecycle is **unit-testable without a live Convex**:
//!
//! - [`transport`] — the wire types (`BackendGenerationJob`,
//!   `BackendGenerationStatus`, the typed `GenerationError` + error envelope), the
//!   `GenerationTransport` trait, the shared 3-step upload POST, and the test
//!   [`MockTransport`](transport::MockTransport).
//! - [`convex_ws`] — the **primary** WS transport over the official `convex` crate
//!   (ruling #25 / Spike S-2), feature-gated behind `convex-transport`. **Live
//!   round-trip GATED** on a deployment URL + Clerk account.
//! - [`http_poll`] — the **fallback** HTTP-polling transport (WS-blocked-by-proxy).
//! - [`catalog`] — `CatalogEntry` decode (custom, keyed on `kind`) + the partitioned
//!   `ModelCatalog` registry (`by_id`, `video()`/`image()`/…).
//! - [`cost`] — pure credit math (ceil rounding), per-kind + rerun `cost_for`.
//! - [`validate`] — per-model + reference-count validation (human-readable errors).
//! - [`params`] — the byte-faithful `*GenerationParams` wire contract.
//! - [`upload`] — the 3-step reference upload, the Content-Type map, the 6-day cache.
//! - [`gating`] — the advisory `can_generate` + affordability (reads `palmier-auth`).
//! - [`service`] — the `generate(...)` orchestrator: placeholders → submit →
//!   subscribe → finalize/download, reporting through a [`GenerationSink`](service::GenerationSink).
//! - [`preferences`] — user-disabled model ids (form filter).
//!
//! ## Gating (Wren / §13.9)
//! No live Convex deployment URL / test Clerk account is available yet (S-2 §5):
//! the WS transport compiles against the real `convex` 0.10 API but its socket is
//! never opened here; the live round-trip + the R-6 Date capture are E9-S1's first
//! task once that access lands. Every unit test drives the [`MockTransport`].

pub mod catalog;
pub mod cost;
pub mod gating;
pub mod http_poll;
pub mod params;
pub mod preferences;
pub mod service;
pub mod transport;
pub mod upload;
pub mod validate;

#[cfg(feature = "convex-transport")]
pub mod convex_ws;

// ── Re-exports (the crate's public API surface) ──────────────────────────────
pub use catalog::{
    AudioCaps, AudioModel, AudioPricing, Capabilities, CatalogEntry, ImageCaps, ImageModel,
    ModelCatalog, ModelCategory, UpscaleCaps, UpscaleModel, VideoCaps, VideoModel,
};
pub use cost::{audio_cost, cost_for, image_cost, upscale_cost, video_cost};
pub use gating::{can_afford, can_generate, GateBlock};
pub use http_poll::HttpPollingTransport;
pub use params::{
    AudioParams, BackendGenerationParams, ImageParams, UpscaleParams, VideoParams,
};
pub use preferences::ModelPreferences;
pub use service::{
    GenerateRequest, GenerationHandle, GenerationService, GenerationSink, StatusUpdate,
};
pub use transport::{
    BackendGenerationJob, BackendGenerationStatus, GenerationError, GenerationTransport, JobStream,
};
pub use upload::{content_type_for, upload_references, ReferenceUpload, UploadResult, UPLOAD_CACHE_TTL};
pub use validate::{
    clamp_num_images, validate_audio, validate_image, validate_upscale, validate_video,
    validate_video_references, ReferenceCounts,
};

#[cfg(feature = "convex-transport")]
pub use convex_ws::{ConvexWsTransport, JwtProvider};

#[cfg(any(test, feature = "test-mock"))]
pub use transport::MockTransport;
