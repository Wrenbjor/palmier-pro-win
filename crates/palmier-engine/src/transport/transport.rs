//! Transport loop state machine — E5-S7.
//!
//! Port of the macOS reference `VideoEngine.swift` (`@MainActor` transport), minus
//! the `AVPlayer`. The reference owns one `AVPlayer`, seeks it, and observes its
//! clock; we replace the player with an explicit [`Transport`] state machine driven
//! by an injectable [`Clock`] (so playback is testable under a fake clock) that
//! emits **presentation-agnostic effects** ([`TransportEvent`]). The Tauri layer
//! turns those effects into events / decode requests / GPU draws — the engine never
//! touches the webview or wgpu directly (FOUNDATION §4 strict layering).
//!
//! ## What the transport emits (the E5-S8 / Tauri contract)
//!
//! Each transport action returns a `Vec<TransportEvent>`:
//! - [`TransportEvent::CurrentFrameChanged`] — the reactive `current_frame`
//!   (timeline tab) or `source_playhead_frame` (asset tab); the Tauri layer streams
//!   it as an event (FR-19 "current_frame reactive via Tauri events").
//! - [`TransportEvent::Render`] — a finalized [`RenderFrame`] for E5-S8 to draw and
//!   present. The transport says *what* to render, never *how*.
//! - [`TransportEvent::SeekDecode`] — ask `palmier-media`'s `FrameSource` to land on
//!   `frame` under a [`SeekMode`] (Exact cancels pending; InteractiveScrub serves
//!   nearest + queues a precise decode, throttled). The transport carries the
//!   reference's tolerance/throttle decision; the decode owner does the fetch.
//! - [`TransportEvent::PlaybackStateChanged`] — play/pause edge for UI sync
//!   (reference `editor.isPlaying`).
//!
//! ## Two-tier rebuild (risk #8)
//!
//! [`Transport::rebuild`] does a full [`build_frame`] (structural change: clips
//! added / removed / retimed). [`Transport::refresh_visuals`] re-samples only
//! transform / opacity / crop on the **existing** frame graph (a transform/opacity/
//! volume edit) and must NOT re-decode — it preserves each layer's `FrameRef`. The
//! transport chooses between them; it never collapses a property edit into a full
//! rebuild.

use std::time::Duration;

use palmier_media::decode::interactive_tolerance_frames;
use palmier_media::{ScrubThrottle, SeekMode};
use palmier_model::Timeline;

use crate::composition::{build_frame, refresh_visuals, SourceResolver};
use crate::preview::{Canvas, PreviewTab, PreviewTabState, QualityTarget, RenderFrame};

use super::clock::{active_video_layer_count, Clock};

/// A presentation-agnostic effect the transport emits for the Tauri / E5-S8 layer.
///
/// The transport produces these; it does not perform them. The orchestrator (Tauri
/// command layer) maps each to a webview event, a `FrameSource` call, or a GPU draw.
#[derive(Debug, Clone, PartialEq)]
pub enum TransportEvent {
    /// The reactive playhead changed — `current_frame` (timeline tab) or
    /// `source_playhead_frame` (asset tab). Streamed to the frontend as an event.
    CurrentFrameChanged {
        /// The new playhead frame.
        frame: i32,
        /// Whether this is the timeline tab's `current_frame` (vs. an asset tab's
        /// `source_playhead_frame`).
        is_timeline: bool,
    },
    /// Render + present this finalized frame (E5-S8 consumes the [`RenderFrame`]).
    Render(RenderFrame),
    /// Ask the decode owner to land on `frame` under `mode`. For
    /// [`SeekMode::InteractiveScrub`] the `tolerance_frames` window is the reference's
    /// `min(0.75, 0.15·layers)` s converted to frames; the decode owner may serve a
    /// cached frame within that window and queue the precise decode.
    SeekDecode {
        /// Target source/timeline frame to seek the decoder to.
        frame: i32,
        /// Exact vs. interactive-scrub semantics.
        mode: SeekMode,
        /// Acceptable nearest-frame window for InteractiveScrub (0 for Exact).
        tolerance_frames: u64,
    },
    /// Playback started/stopped (reference `editor.isPlaying`) — UI play/pause sync.
    PlaybackStateChanged(bool),
}

/// The transport's playback state machine. Holds the active tab, per-tab playheads,
/// the playback clock + the structural [`CompositionFrame`] it drives, and the
/// interactive-scrub throttle. Generic over the [`Clock`] so tests inject a fake one.
///
/// The transport does **not** own the timeline or the [`SourceResolver`] — those are
/// passed per call so the engine's single source of truth (the project model) stays
/// external, exactly as the reference reads `editor.timeline` each time.
#[derive(Debug)]
pub struct Transport<C: Clock> {
    clock: C,
    /// The active preview tab (reference `editor.activePreviewTab`).
    active_tab: PreviewTab,
    /// Per-tab playhead state, keyed by [`PreviewTab::id`]. The timeline tab and each
    /// asset tab keep their own playhead across switches.
    tab_state: std::collections::HashMap<String, PreviewTabState>,
    /// Whether playback is running (reference `editor.isPlaying`).
    playing: bool,
    /// Whether the user is mid-scrub — the time observer does not advance the
    /// playhead while scrubbing (reference `editor.isScrubbing` guard).
    scrubbing: bool,
    /// The current structural composition frame (the skeleton `refresh_visuals`
    /// re-samples). `None` until the first `rebuild`/`seek`.
    current_composition: Option<crate::composition::CompositionFrame>,
    /// Interactive-scrub coalescing throttle (1/30 s — reference
    /// `interactiveSeekInterval`).
    throttle: ScrubThrottle,
    /// A pending coalesced interactive seek awaiting the throttle window
    /// (reference `pendingInteractiveSeek`).
    pending_scrub: Option<(i32, u64)>,
    /// The clock time (since origin) at which the current play span started, plus the
    /// frame it started from — used to convert elapsed time → frames advanced.
    play_anchor: Option<(Duration, i32)>,
}

impl<C: Clock> Transport<C> {
    /// A new transport on the **timeline tab**, paused at frame 0, with the given
    /// clock.
    pub fn new(clock: C) -> Self {
        let mut tab_state = std::collections::HashMap::new();
        tab_state.insert(PreviewTab::Timeline.id(), PreviewTabState::default());
        Transport {
            clock,
            active_tab: PreviewTab::Timeline,
            tab_state,
            playing: false,
            scrubbing: false,
            current_composition: None,
            throttle: ScrubThrottle::default(),
            pending_scrub: None,
            play_anchor: None,
        }
    }

    /// The active tab.
    pub fn active_tab(&self) -> &PreviewTab {
        &self.active_tab
    }

    /// Whether playback is running.
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// The active tab's current playhead frame.
    pub fn current_frame(&self) -> i32 {
        self.tab_state
            .get(&self.active_tab.id())
            .map(|s| s.playhead_frame)
            .unwrap_or(0)
    }

    fn set_current_frame(&mut self, frame: i32) {
        self.tab_state
            .entry(self.active_tab.id())
            .or_default()
            .playhead_frame = frame;
    }

    /// Begin a scrub gesture — suppresses time-observer playhead advance until
    /// [`Transport::end_scrub`] (reference `editor.isScrubbing`).
    pub fn begin_scrub(&mut self) {
        self.scrubbing = true;
    }

    /// End a scrub gesture.
    pub fn end_scrub(&mut self) {
        self.scrubbing = false;
    }

    /// Start playback. Mirrors `VideoEngine.play`: mark playing, seek **exactly** to
    /// the current frame (playback start is always exact), and anchor the clock.
    pub fn play<R: SourceResolver>(
        &mut self,
        timeline: &Timeline,
        resolver: &R,
    ) -> Vec<TransportEvent> {
        if self.playing {
            return Vec::new();
        }
        self.playing = true;
        let frame = self.current_frame();
        self.play_anchor = Some((self.clock.now(), frame));
        let mut events = vec![TransportEvent::PlaybackStateChanged(true)];
        // Playback start is an exact seek (reference seeks `.exact` in `play`).
        events.extend(self.seek(timeline, resolver, frame, SeekMode::Exact));
        events
    }

    /// Pause playback (reference `VideoEngine.pause`).
    pub fn pause(&mut self) -> Vec<TransportEvent> {
        if !self.playing {
            return Vec::new();
        }
        self.playing = false;
        self.play_anchor = None;
        vec![TransportEvent::PlaybackStateChanged(false)]
    }

    /// Toggle play/pause (reference `togglePlayback`).
    pub fn toggle_playback<R: SourceResolver>(
        &mut self,
        timeline: &Timeline,
        resolver: &R,
    ) -> Vec<TransportEvent> {
        if self.playing {
            self.pause()
        } else {
            self.play(timeline, resolver)
        }
    }

    /// Seek to `frame` under `mode` (reference `VideoEngine.seek(to:mode:)`).
    ///
    /// `Exact` cancels any pending interactive seek and dispatches immediately
    /// (tolerance 0, exact frame). `InteractiveScrub` computes the
    /// `min(0.75, 0.15·activeLayers)` s tolerance, coalesces under the 1/30 s
    /// throttle, and dispatches only when the throttle window has elapsed — exactly
    /// the reference's `enqueueInteractiveSeek` / `flushPendingInteractiveSeek` flow.
    pub fn seek<R: SourceResolver>(
        &mut self,
        timeline: &Timeline,
        resolver: &R,
        frame: i32,
        mode: SeekMode,
    ) -> Vec<TransportEvent> {
        // The playhead reflects the requested frame immediately (the reference sets
        // the player target and the displayed frame follows; for the engine the
        // playhead is authoritative).
        let frame = frame.max(0);
        let was = self.current_frame();
        self.set_current_frame(frame);
        // Re-anchor the play clock so elapsed time counts from the new position.
        if self.playing {
            self.play_anchor = Some((self.clock.now(), frame));
        }

        let mut events = Vec::new();
        if frame != was {
            events.push(TransportEvent::CurrentFrameChanged {
                frame,
                is_timeline: self.active_tab.is_timeline(),
            });
        }

        match mode {
            SeekMode::Exact => {
                // Cancel any pending interactive seek; dispatch precisely now.
                self.throttle.reset();
                self.pending_scrub = None;
                events.push(TransportEvent::SeekDecode {
                    frame,
                    mode: SeekMode::Exact,
                    tolerance_frames: 0,
                });
                // Exact seeks rebuild structurally (a precise frame is decoded).
                events.extend(self.rebuild(timeline, resolver, frame));
            }
            SeekMode::InteractiveScrub => {
                let layers =
                    active_video_layer_count(timeline, frame, self.active_tab.is_timeline());
                let tol = interactive_tolerance_frames(layers, timeline.fps.max(1) as f64);
                self.pending_scrub = Some((frame, tol));
                let now = self.clock.now_instant_like();
                if self.throttle.can_dispatch(now) {
                    self.throttle.record_dispatch(now);
                    if let Some((f, t)) = self.pending_scrub.take() {
                        events.push(TransportEvent::SeekDecode {
                            frame: f,
                            mode: SeekMode::InteractiveScrub,
                            tolerance_frames: t,
                        });
                        // A scrub re-samples visuals only on the existing graph
                        // (cheap edit path); if there's no graph yet, build one.
                        events.extend(self.refresh_or_build(timeline, resolver, f));
                    }
                }
                // else: coalesced — the next `flush_pending_scrub` (driven by the
                // throttle timer) will dispatch it. The pending seek is retained.
            }
        }
        events
    }

    /// Flush a coalesced interactive seek if the throttle window has elapsed
    /// (reference `flushPendingInteractiveSeek`, called by the throttle timer). The
    /// orchestrator calls this when the `SCRUB_THROTTLE` delay returned earlier
    /// fires. No-op if nothing is pending or the window hasn't elapsed.
    pub fn flush_pending_scrub<R: SourceResolver>(
        &mut self,
        timeline: &Timeline,
        resolver: &R,
    ) -> Vec<TransportEvent> {
        let now = self.clock.now_instant_like();
        if !self.throttle.can_dispatch(now) {
            return Vec::new();
        }
        let Some((frame, tol)) = self.pending_scrub.take() else {
            return Vec::new();
        };
        self.throttle.record_dispatch(now);
        let mut events = vec![TransportEvent::SeekDecode {
            frame,
            mode: SeekMode::InteractiveScrub,
            tolerance_frames: tol,
        }];
        events.extend(self.refresh_or_build(timeline, resolver, frame));
        events
    }

    /// Step the playhead by `delta` frames (reference frame-stepping — always an
    /// **exact** seek). Clamped to `≥ 0`.
    pub fn step<R: SourceResolver>(
        &mut self,
        timeline: &Timeline,
        resolver: &R,
        delta: i32,
    ) -> Vec<TransportEvent> {
        let target = (self.current_frame() + delta).max(0);
        self.seek(timeline, resolver, target, SeekMode::Exact)
    }

    /// Advance the playhead from the playback clock (reference periodic time observer
    /// at `1/fps`). Called by the orchestrator on each clock tick. While playing and
    /// not scrubbing, computes the frame from elapsed time since the play anchor and,
    /// if it changed, emits a `CurrentFrameChanged` + a structural `Render` for the
    /// new frame (and drives the audio mixer at the orchestrator boundary).
    ///
    /// Returns no events when paused, scrubbing, or still on the same frame.
    pub fn tick<R: SourceResolver>(
        &mut self,
        timeline: &Timeline,
        resolver: &R,
    ) -> Vec<TransportEvent> {
        if !self.playing || self.scrubbing {
            return Vec::new();
        }
        let Some((anchor_time, anchor_frame)) = self.play_anchor else {
            return Vec::new();
        };
        let fps = timeline.fps.max(1);
        let elapsed = self.clock.now().saturating_sub(anchor_time);
        // secondsToFrame = Int(seconds * fps) — truncation toward zero (reference
        // `TimeFormatting.secondsToFrame`). A tiny epsilon absorbs binary-float dust
        // (e.g. `1/30 s × 30` materializing as 0.99999…) so a clock advanced by an
        // exact frame count lands on the whole frame rather than truncating one short.
        let advanced = (elapsed.as_secs_f64() * fps as f64 + 1e-6) as i32;
        let frame = anchor_frame + advanced;
        if frame == self.current_frame() {
            return Vec::new();
        }
        self.set_current_frame(frame);
        let mut events = vec![TransportEvent::CurrentFrameChanged {
            frame,
            is_timeline: self.active_tab.is_timeline(),
        }];
        // Per visible frame during playback: a full structural build (a new source
        // frame is needed). The orchestrator additionally advances the audio mixer.
        events.extend(self.rebuild(timeline, resolver, frame));
        events
    }

    /// Full structural rebuild (risk #8): build a fresh [`CompositionFrame`] for
    /// `frame` via [`build_frame`] and emit it as a [`RenderFrame`]. Replaces the
    /// retained composition skeleton. Use when the structure changed (clips
    /// added/removed/retimed) or a precise frame is needed (playback/step/exact).
    pub fn rebuild<R: SourceResolver>(
        &mut self,
        timeline: &Timeline,
        resolver: &R,
        frame: i32,
    ) -> Vec<TransportEvent> {
        let composition = build_frame(timeline, frame, resolver);
        self.current_composition = Some(composition.clone());
        let canvas = Canvas::new(timeline.width.max(1) as u32, timeline.height.max(1) as u32);
        let quality = QualityTarget::Full;
        vec![TransportEvent::Render(RenderFrame::new(
            composition,
            canvas,
            quality,
        ))]
    }

    /// Visuals-only fast path (risk #8 / reference `refreshVisuals`): re-sample
    /// transform / opacity / crop on the **existing** composition skeleton without a
    /// decode or structural rebuild, then emit the updated [`RenderFrame`]. Falls
    /// back to [`Transport::rebuild`] when there is no retained composition yet
    /// (reference `refreshVisuals` rebuilds if `trackMappings` is empty).
    ///
    /// `quality` lets the caller render an interactive scrub at a reduced backing
    /// resolution (the explicit version of AVFoundation's scrub downscaling).
    pub fn refresh_visuals<R: SourceResolver>(
        &mut self,
        timeline: &Timeline,
        resolver: &R,
        frame: i32,
        quality: QualityTarget,
    ) -> Vec<TransportEvent> {
        let Some(mut composition) = self.current_composition.clone() else {
            return self.rebuild(timeline, resolver, frame);
        };
        composition.frame_index = frame;
        refresh_visuals(&mut composition, timeline, resolver);
        self.current_composition = Some(composition.clone());
        let canvas = Canvas::new(timeline.width.max(1) as u32, timeline.height.max(1) as u32);
        vec![TransportEvent::Render(RenderFrame::new(
            composition,
            canvas,
            quality,
        ))]
    }

    /// During a scrub, prefer the cheap visuals refresh (scrub-scaled quality); if no
    /// composition skeleton exists yet, build one. Internal helper for `seek`/flush.
    fn refresh_or_build<R: SourceResolver>(
        &mut self,
        timeline: &Timeline,
        resolver: &R,
        frame: i32,
    ) -> Vec<TransportEvent> {
        if self.current_composition.is_some() {
            // A scrub only changes which frame is shown, not the structure — but the
            // *source frame* per layer changes, so a scrub still needs the structural
            // build to pick the right FrameRef. We use rebuild for frame correctness
            // but at scrub quality. (refresh_visuals alone would keep the old
            // FrameRef, which is wrong when the playhead moves.)
            let composition = build_frame(timeline, frame, resolver);
            self.current_composition = Some(composition.clone());
            let canvas =
                Canvas::new(timeline.width.max(1) as u32, timeline.height.max(1) as u32);
            vec![TransportEvent::Render(RenderFrame::new(
                composition,
                canvas,
                QualityTarget::Scaled(0.5),
            ))]
        } else {
            self.rebuild(timeline, resolver, frame)
        }
    }

    /// Activate `tab`, saving the outgoing tab's playhead and restoring the
    /// incoming one (reference `VideoEngine.activateTab`: pause, invalidate seek
    /// state, swap). Returns the `CurrentFrameChanged` for the restored playhead.
    pub fn activate_tab(&mut self, tab: PreviewTab) -> Vec<TransportEvent> {
        // Pause + invalidate scrub state on tab switch (reference).
        let mut events = Vec::new();
        if self.playing {
            events.extend(self.pause());
        }
        self.throttle.reset();
        self.pending_scrub = None;
        self.current_composition = None; // structure differs per tab.

        self.active_tab = tab;
        let restored = self
            .tab_state
            .entry(self.active_tab.id())
            .or_default()
            .playhead_frame;
        events.push(TransportEvent::CurrentFrameChanged {
            frame: restored,
            is_timeline: self.active_tab.is_timeline(),
        });
        events
    }

    /// Close an asset tab's retained state (the timeline tab is never closable —
    /// reference `isCloseable`). No-op for the timeline id.
    pub fn close_tab(&mut self, tab: &PreviewTab) {
        if tab.is_closeable() {
            self.tab_state.remove(&tab.id());
        }
    }
}

#[cfg(test)]
impl Transport<crate::transport::clock::ManualClock> {
    /// Test helper: advance the transport's own [`ManualClock`] by `delta`.
    pub(crate) fn advance_clock(&mut self, delta: Duration) {
        self.clock.advance(delta);
    }

    /// Test helper: advance the transport's clock by `frames` at `fps`.
    pub(crate) fn advance_clock_frames(&mut self, frames: u32, fps: u32) {
        self.clock.advance_frames(frames, fps);
    }
}

/// A tiny shim so the transport can hand the [`ScrubThrottle`] (which speaks
/// [`std::time::Instant`]) a monotonic instant derived from the injectable clock,
/// keeping the throttle deterministic under the fake clock.
trait ClockInstant {
    fn now_instant_like(&self) -> std::time::Instant;
}

impl<C: Clock> ClockInstant for C {
    fn now_instant_like(&self) -> std::time::Instant {
        // The throttle only compares deltas, so anchoring a fixed base + the clock's
        // monotonic `now()` keeps `delay_until_next` correct under both the wall and
        // manual clocks. A process-lifetime base keeps it monotonic.
        base_instant() + self.now()
    }
}

fn base_instant() -> std::time::Instant {
    use std::sync::OnceLock;
    static BASE: OnceLock<std::time::Instant> = OnceLock::new();
    *BASE.get_or_init(std::time::Instant::now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composition::SourceInfo;
    use crate::transport::clock::ManualClock;
    use palmier_model::{Clip, ClipType, Track};

    fn resolver() -> impl SourceResolver {
        |_r: &str| Some(SourceInfo::upright((1920.0, 1080.0)))
    }

    fn timeline_one_clip() -> Timeline {
        let mut tl = Timeline::new();
        tl.fps = 30;
        tl.width = 1920;
        tl.height = 1080;
        let mut t = Track::new(ClipType::Video);
        let mut c = Clip::new("m", 0, 300);
        c.id = "c".into();
        t.clips.push(c);
        tl.tracks = vec![t];
        tl
    }

    fn render_events(events: &[TransportEvent]) -> Vec<&RenderFrame> {
        events
            .iter()
            .filter_map(|e| match e {
                TransportEvent::Render(rf) => Some(rf),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn play_pause_toggle_state_machine() {
        let tl = timeline_one_clip();
        let r = resolver();
        let mut tr = Transport::new(ManualClock::new());
        assert!(!tr.is_playing());

        let ev = tr.play(&tl, &r);
        assert!(tr.is_playing());
        assert!(ev.contains(&TransportEvent::PlaybackStateChanged(true)));
        // Play does an exact seek → a SeekDecode + a Render.
        assert!(ev.iter().any(|e| matches!(e, TransportEvent::SeekDecode { mode: SeekMode::Exact, .. })));
        assert_eq!(render_events(&ev).len(), 1);

        // play() while already playing is a no-op.
        assert!(tr.play(&tl, &r).is_empty());

        let ev = tr.pause();
        assert!(!tr.is_playing());
        assert!(ev.contains(&TransportEvent::PlaybackStateChanged(false)));

        // toggle resumes.
        let ev = tr.toggle_playback(&tl, &r);
        assert!(tr.is_playing());
        assert!(ev.contains(&TransportEvent::PlaybackStateChanged(true)));
        // toggle again pauses.
        tr.toggle_playback(&tl, &r);
        assert!(!tr.is_playing());
    }

    #[test]
    fn current_frame_advances_under_fake_clock() {
        let tl = timeline_one_clip();
        let r = resolver();
        // The transport owns its clock; we advance it via the test-only
        // `advance_clock*` helpers so playback timing is fully deterministic.
        let mut tr = Transport::new(ManualClock::new());
        tr.play(&tl, &r);
        assert_eq!(tr.current_frame(), 0);

        // No time elapsed yet → tick emits nothing.
        assert!(tr.tick(&tl, &r).is_empty());

        // Advance the transport's clock by 1 frame (1/30 s) and tick.
        tr.advance_clock_frames(1, 30);
        let ev = tr.tick(&tl, &r);
        assert_eq!(tr.current_frame(), 1);
        assert!(ev.iter().any(|e| matches!(
            e,
            TransportEvent::CurrentFrameChanged { frame: 1, is_timeline: true }
        )));
        assert_eq!(render_events(&ev).len(), 1, "playback tick renders the new frame");

        // Advance 5 more frames at once → jumps to frame 6.
        tr.advance_clock_frames(5, 30);
        tr.tick(&tl, &r);
        assert_eq!(tr.current_frame(), 6);

        // Paused → tick does nothing even as the clock advances.
        tr.pause();
        tr.advance_clock_frames(10, 30);
        assert!(tr.tick(&tl, &r).is_empty());
        assert_eq!(tr.current_frame(), 6);
    }

    #[test]
    fn scrubbing_suppresses_tick_advance() {
        let tl = timeline_one_clip();
        let r = resolver();
        let mut tr = Transport::new(ManualClock::new());
        tr.play(&tl, &r);
        tr.begin_scrub();
        tr.advance_clock_frames(5, 30);
        assert!(tr.tick(&tl, &r).is_empty(), "no advance while scrubbing");
        tr.end_scrub();
    }

    #[test]
    fn step_is_exact_and_clamps_to_zero() {
        let tl = timeline_one_clip();
        let r = resolver();
        let mut tr = Transport::new(ManualClock::new());
        let ev = tr.step(&tl, &r, 5);
        assert_eq!(tr.current_frame(), 5);
        assert!(ev.iter().any(|e| matches!(e, TransportEvent::SeekDecode { mode: SeekMode::Exact, tolerance_frames: 0, .. })));
        // Step back below zero clamps.
        tr.step(&tl, &r, -100);
        assert_eq!(tr.current_frame(), 0);
    }

    #[test]
    fn exact_seek_emits_exact_decode_and_full_render() {
        let tl = timeline_one_clip();
        let r = resolver();
        let mut tr = Transport::new(ManualClock::new());
        let ev = tr.seek(&tl, &r, 42, SeekMode::Exact);
        assert_eq!(tr.current_frame(), 42);
        let decode = ev.iter().find_map(|e| match e {
            TransportEvent::SeekDecode { frame, mode, tolerance_frames } => {
                Some((*frame, *mode, *tolerance_frames))
            }
            _ => None,
        });
        assert_eq!(decode, Some((42, SeekMode::Exact, 0)), "exact lands on the exact frame, tolerance 0");
        let rf = render_events(&ev);
        assert_eq!(rf.len(), 1);
        assert_eq!(rf[0].quality, QualityTarget::Full);
        assert_eq!(rf[0].frame_index(), 42);
    }

    #[test]
    fn interactive_scrub_tolerance_wired_by_layer_count() {
        // FR-19: tolerance window must follow min(0.75, 0.15·activeLayerCount) at
        // layer counts 1/3/6. Build timelines with 1, 3, 6 active video layers.
        let r = resolver();
        for (layers, expected_tol) in [(1u32, 5u64), (3, 14), (6, 23)] {
            let mut tl = Timeline::new();
            tl.fps = 30;
            tl.width = 1920;
            tl.height = 1080;
            let mut tracks = Vec::new();
            for i in 0..layers {
                let mut t = Track::new(ClipType::Video);
                let mut c = Clip::new(format!("m{i}"), 0, 300);
                c.id = format!("c{i}");
                t.clips.push(c);
                tracks.push(t);
            }
            tl.tracks = tracks;

            let mut tr = Transport::new(ManualClock::new());
            let ev = tr.seek(&tl, &r, 50, SeekMode::InteractiveScrub);
            let tol = ev.iter().find_map(|e| match e {
                TransportEvent::SeekDecode { mode: SeekMode::InteractiveScrub, tolerance_frames, .. } => {
                    Some(*tolerance_frames)
                }
                _ => None,
            });
            assert_eq!(
                tol,
                Some(expected_tol),
                "layer count {layers} must yield tolerance {expected_tol} frames"
            );
        }
    }

    #[test]
    fn interactive_scrub_throttle_coalesces_within_window() {
        let tl = timeline_one_clip();
        let r = resolver();
        let mut tr = Transport::new(ManualClock::new());

        // First scrub dispatches immediately.
        let ev1 = tr.seek(&tl, &r, 10, SeekMode::InteractiveScrub);
        assert!(ev1.iter().any(|e| matches!(e, TransportEvent::SeekDecode { mode: SeekMode::InteractiveScrub, .. })));

        // A second scrub within the 1/30 s window is coalesced — no immediate decode.
        let ev2 = tr.seek(&tl, &r, 11, SeekMode::InteractiveScrub);
        assert!(!ev2.iter().any(|e| matches!(e, TransportEvent::SeekDecode { .. })), "coalesced, no dispatch");

        // After the throttle window elapses, flushing dispatches the pending target.
        tr.advance_clock(Duration::from_millis(40));
        let flushed = tr.flush_pending_scrub(&tl, &r);
        let frame = flushed.iter().find_map(|e| match e {
            TransportEvent::SeekDecode { frame, mode: SeekMode::InteractiveScrub, .. } => Some(*frame),
            _ => None,
        });
        assert_eq!(frame, Some(11), "the latest coalesced target is dispatched");
    }

    #[test]
    fn structural_vs_property_rebuild_selection() {
        // rebuild() = full build_frame; refresh_visuals() = re-sample, FrameRef kept.
        let mut tl = timeline_one_clip();
        // Give the clip a linear fade so opacity changes with frame.
        tl.tracks[0].clips[0].fade_in_frames = 10;
        tl.tracks[0].clips[0].fade_in_interpolation = palmier_model::Interpolation::Linear;
        let r = resolver();
        let mut tr = Transport::new(ManualClock::new());

        // Structural build at frame 0.
        let ev = tr.rebuild(&tl, &r, 0);
        let rf = render_events(&ev);
        assert_eq!(rf.len(), 1);
        let frame_ref0 = rf[0].composition.layers[0].visual().unwrap().frame.clone();
        let op0 = rf[0].composition.layers[0].visual().unwrap().opacity;
        assert!(op0.abs() < 1e-9, "frame 0 fade-in opacity is 0");

        // Property refresh at frame 5: opacity re-samples to 0.5, SAME source frame
        // (no decode/structural change — risk #8).
        let ev = tr.refresh_visuals(&tl, &r, 5, QualityTarget::Full);
        let rf = render_events(&ev);
        let v = rf[0].composition.layers[0].visual().unwrap();
        assert!((v.opacity - 0.5).abs() < 1e-9, "opacity re-sampled to 0.5: {}", v.opacity);
        assert_eq!(v.frame, frame_ref0, "refresh_visuals must NOT change the source FrameRef");
    }

    #[test]
    fn refresh_visuals_falls_back_to_rebuild_without_skeleton() {
        let tl = timeline_one_clip();
        let r = resolver();
        let mut tr = Transport::new(ManualClock::new());
        // No prior rebuild → refresh_visuals must build (reference falls back when
        // trackMappings is empty).
        let ev = tr.refresh_visuals(&tl, &r, 3, QualityTarget::Full);
        assert_eq!(render_events(&ev).len(), 1);
        assert_eq!(render_events(&ev)[0].frame_index(), 3);
    }

    #[test]
    fn tab_activation_saves_and_restores_playhead() {
        let tl = timeline_one_clip();
        let r = resolver();
        let mut tr = Transport::new(ManualClock::new());
        // Move the timeline playhead to 100.
        tr.seek(&tl, &r, 100, SeekMode::Exact);
        assert_eq!(tr.current_frame(), 100);

        // Switch to an asset tab — its playhead starts at 0.
        let asset = PreviewTab::media_asset("a1", "Clip.mp4", ClipType::Video);
        let ev = tr.activate_tab(asset.clone());
        assert_eq!(tr.current_frame(), 0);
        assert!(ev.iter().any(|e| matches!(
            e,
            TransportEvent::CurrentFrameChanged { frame: 0, is_timeline: false }
        )));
        // Move the asset playhead to 25.
        tr.seek(&tl, &r, 25, SeekMode::Exact);
        assert_eq!(tr.current_frame(), 25);

        // Back to the timeline → playhead 100 restored.
        tr.activate_tab(PreviewTab::Timeline);
        assert_eq!(tr.current_frame(), 100);
        // Back to the asset → 25 restored.
        tr.activate_tab(asset);
        assert_eq!(tr.current_frame(), 25);
    }
}
