//! Playback transport — E5-S7.
//!
//! Port of the macOS reference `VideoEngine.swift` transport, minus `AVPlayer`. The
//! [`Transport`] is the play/pause/seek/step state machine that drives the
//! composition graph + audio mixer and emits a reactive `current_frame` as
//! presentation-agnostic [`TransportEvent`]s the Tauri layer streams (FOUNDATION
//! §6.5, FR-19).
//!
//! - [`clock`] — the injectable [`Clock`] (so playback is testable under a fake
//!   clock) + the reference `activeVideoLayerCount` used to size the scrub tolerance.
//! - [`transport`] — the [`Transport`] state machine + [`TransportEvent`] effects.
//!
//! ## Presentation-agnostic boundary
//!
//! The transport emits *what* to do — "current_frame changed", "render this
//! [`RenderFrame`]", "seek the decoder to frame F under mode M". It never touches
//! wgpu (E5-S8 draws the `RenderFrame`), the webview (the Tauri layer emits the
//! events), or FFmpeg (`palmier-media`'s `FrameSource` does the decode). The
//! [`SeekMode`](palmier_media::SeekMode), tolerance, and throttle are reused verbatim
//! from `palmier-media` (E5-S2) so the decode owner and the transport agree on the
//! contract.

pub mod clock;
#[allow(clippy::module_inception)]
pub mod transport;

pub use clock::{active_video_layer_count, Clock, ManualClock, WallClock};
pub use transport::{Transport, TransportEvent};
