//! E10-S6 tests: the 6 `specs(...)` oracles ported verbatim from
//! `Tests/PalmierProTests/Captions/CaptionBuilderTests.swift` (these test `specs`,
//! which is E10-S6 scope — E10-S5 deferred them), plus the orchestration helpers
//! (visible-source union, dominant-speech-track, phrase→clip assignment) and the
//! caption-track placement + undo-group-name parity.

use super::*;
use palmier_history::History;
use palmier_model::{Clip, ClipType, TextStyle, Timeline, Track, Transform};
use palmier_text::{FontRegistry, Phrase, TextLayout};
use palmier_transcribe::{TranscriptionResult, TranscriptionSegment, TranscriptionWord};
use std::ops::RangeInclusive;

/// The shared reference fixture clip: `Clip(mediaRef:"m", startFrame:30, durationFrames:120)`.
fn clip() -> Clip {
    Clip::new("m", 30, 120)
}

fn phrase(text: &str, start: f64, end: f64) -> Phrase {
    Phrase::new(text, start, end)
}

// ===================================================================
// The 6 `specs(...)` tests — ported verbatim from CaptionBuilderTests.swift
// ===================================================================

/// `mapsSecondsThroughClipPlacement`: phrase 1.0..2.0s on a clip at frame 30,
/// duration 120, fps 30 → start 60, duration 30, group "g1".
#[test]
fn maps_seconds_through_clip_placement() {
    let p = phrase("hi", 1.0, 2.0);
    let s = specs(&[p], &clip(), 0, 30, &TextStyle::default(), Some("g1"), |_| None, 1);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].start_frame, 60);
    assert_eq!(s[0].duration_frames, 30);
    assert_eq!(s[0].caption_group_id.as_deref(), Some("g1"));
}

/// `clampsPhraseRunningPastClipEnd`: phrase 1.0..10.0s → start 60, clamped duration 90.
#[test]
fn clamps_phrase_running_past_clip_end() {
    let p = phrase("long", 1.0, 10.0);
    let s = specs(&[p], &clip(), 0, 30, &TextStyle::default(), None, |_| None, 1);
    assert_eq!(s[0].start_frame, 60);
    assert_eq!(s[0].duration_frames, 90);
}

/// `clampsPhraseSpanningTrimmedClip`: trim_start 60, phrase 0.0..10.0s → start 30,
/// duration 120 (full clip).
#[test]
fn clamps_phrase_spanning_trimmed_clip() {
    let mut trimmed = clip();
    trimmed.trim_start_frame = 60;
    let p = phrase("full", 0.0, 10.0);
    let s = specs(&[p], &trimmed, 0, 30, &TextStyle::default(), None, |_| None, 1);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].start_frame, 30);
    assert_eq!(s[0].duration_frames, 120);
}

/// `transformForResolvesEachBox`: `transform_for` resolves each box's transform.
#[test]
fn transform_for_resolves_each_box() {
    let p = phrase("hi", 1.0, 2.0);
    // Reference `Transform(center: (0.5, 0.85), width: 0.4, height: 0.1)`.
    let box_t = Transform {
        center_x: 0.5,
        center_y: 0.85,
        width: 0.4,
        height: 0.1,
        ..Transform::default()
    };
    let s = specs(
        &[p],
        &clip(),
        0,
        30,
        &TextStyle::default(),
        None,
        |_| Some(box_t),
        1,
    );
    assert_eq!(s[0].transform, Some(box_t));
}

/// `dropsPhraseEntirelyBeforeTrimIn`: trim_start 60, phrase 0.5..1.0s falls entirely
/// before the trim-in → dropped (no specs).
#[test]
fn drops_phrase_entirely_before_trim_in() {
    let mut trimmed = clip();
    trimmed.trim_start_frame = 60;
    let p = phrase("gone", 0.5, 1.0);
    let s = specs(&[p], &trimmed, 0, 30, &TextStyle::default(), None, |_| None, 1);
    assert!(s.is_empty());
}

/// Center-based Transform parity (ruling #7): a `None`-transform spec auto-fits +
/// centers the box; the emitted clip transform is center-based (never top-left).
#[test]
fn placed_none_transform_is_centered_and_center_based() {
    let mut timeline = Timeline::new();
    timeline.tracks.push(Track::new(ClipType::Video));
    let mut registry = FontRegistry::bundled_only();
    let mut layout = TextLayout::new();
    let spec = TextClipSpec {
        track_index: 0,
        start_frame: 10,
        duration_frames: 20,
        content: "hi".to_string(),
        style: TextStyle::default(),
        transform: None,
        caption_group_id: Some("g".to_string()),
    };
    let ids = place_text_clips(&mut timeline, &[spec], &mut registry, &mut layout);
    assert_eq!(ids.len(), 1);
    let placed = &timeline.tracks[0].clips[0];
    // The auto-fit box is centered: centerX/centerY ≈ 0.5.
    assert!((placed.transform.center_x - 0.5).abs() < 1e-9);
    assert!((placed.transform.center_y - 0.5).abs() < 1e-9);
    assert_eq!(placed.media_type, ClipType::Text);
    assert_eq!(placed.text_content.as_deref(), Some("hi"));
    assert_eq!(placed.caption_group_id.as_deref(), Some("g"));
}

// ===================================================================
// Orchestration helper tests
// ===================================================================

fn segment(text: &str, start: f64, end: f64) -> TranscriptionSegment {
    TranscriptionSegment { text: text.to_string(), start, end }
}

fn word(text: &str, start: f64, end: f64) -> TranscriptionWord {
    TranscriptionWord { text: text.to_string(), start: Some(start), end: Some(end) }
}

/// `visible_source_union` pads ±1.0 s and clamps the lower bound at 0.
#[test]
fn visible_source_union_pads_and_clamps() {
    // One clip on track t, ref "m", start 30 dur 120 trim 0 speed 1 → visible source
    // [0, 120] frames. fps 30 → [0/30 - 1, 120/30 + 1] = [-1→0 clamped, 5.0].
    let targets = vec![CaptionTarget {
        id: "c1".into(),
        track_id: "t1".into(),
        clip: clip(),
    }];
    let union = visible_source_union("m", &targets, 30).expect("union");
    assert!((*union.start() - 0.0).abs() < 1e-9, "lower clamped at 0");
    assert!((*union.end() - 5.0).abs() < 1e-9, "upper = 120/30 + 1");
}

/// `dominant_speech_track` picks the track with the most spoken words (midpoint in
/// the visible window); skips `None`-timestamp words.
#[test]
fn dominant_speech_track_picks_max_words() {
    // Two clips, same media_ref, on two tracks. Clip A visible [0,120]f → at 30fps a
    // word at 1.0..1.2s (mid 1.1s → 33f) counts. Clip B visible [0,30]f → only words
    // before ~0.5s count.
    let mut a = Clip::new("m", 0, 120); // visible source [0,120]
    a.id = "a".into();
    let mut b = Clip::new("m", 0, 30); // visible source [0,30]
    b.id = "b".into();
    let targets = vec![
        CaptionTarget { id: "a".into(), track_id: "trackA".into(), clip: a },
        CaptionTarget { id: "b".into(), track_id: "trackB".into(), clip: b },
    ];
    // Result with 3 words: mids at 1.1s(33f), 2.0s(60f), 3.0s(90f) — all inside A's
    // [0,120] window, none inside B's [0,30].
    let result = TranscriptionResult {
        text: "x".into(),
        language: Some("en".into()),
        words: vec![word("one", 1.0, 1.2), word("two", 1.9, 2.1), word("three", 2.9, 3.1)],
        segments: vec![],
    };
    let mut results = std::collections::HashMap::new();
    results.insert("m".to_string(), result);
    let winner = dominant_speech_track(&targets, &results, 30);
    // Both tracks reference the SAME result; A's window captures all 3 words, B none.
    assert_eq!(winner.as_deref(), Some("trackA"));
}

/// `best_clip` assigns a phrase to the most-overlapping clip only when overlap > 0
/// AND >= phrase_len/2.
#[test]
fn best_clip_requires_half_overlap() {
    // Clip visible source [0,120] frames (start 0 dur 120). fps 30.
    let mut c = Clip::new("m", 0, 120);
    c.id = "c".into();
    let clips = vec![CaptionTarget { id: "c".into(), track_id: "t".into(), clip: c }];

    // Phrase fully inside: 1.0..2.0s → [30,60]f, len 30, overlap 30 ≥ 15 → owned.
    let inside = phrase("p", 1.0, 2.0);
    assert!(best_clip(&inside, &clips, 30).is_some());

    // Phrase mostly outside: 3.5..4.5s → [105,135]f vs visible [0,120] → overlap 15,
    // len 30, half = 15 → 15 >= 15 → still owned (boundary inclusive).
    let edge = phrase("p", 3.5, 4.5);
    assert!(best_clip(&edge, &clips, 30).is_some());

    // Phrase almost entirely past the end: 3.9..4.9s → [117,147]f → overlap 3 < 15 → none.
    let outside = phrase("p", 3.9, 4.9);
    assert!(best_clip(&outside, &clips, 30).is_none());
}

/// Full `generate_captions` happy path: one audio clip, an injected transcript with
/// one short segment → one caption clip placed on a NEW video track at index 0, under
/// the exact "Generate Captions" undo group.
#[test]
fn generate_captions_places_track_with_undo_group() {
    let mut timeline = Timeline::new();
    // One audio track with one clip (media_ref "m", start 0 dur 120).
    let mut audio = Track::new(ClipType::Audio);
    audio.id = "audio-track".into();
    let mut clip = Clip::new("m", 0, 120);
    clip.media_type = ClipType::Audio;
    clip.id = "clip-1".into();
    audio.clips.push(clip);
    timeline.tracks.push(audio);

    let mut history: History<Timeline> = History::new();
    let mut registry = FontRegistry::bundled_only();
    let mut layout = TextLayout::new();

    // Inject a transcript: one segment "hello there" 0.0..1.0s (within visible window).
    let transcribe = |_ref: &str, _range: Option<&RangeInclusive<f64>>, _is_video: bool| {
        Ok::<_, std::convert::Infallible>(TranscriptionResult {
            text: "hello there".into(),
            language: Some("en".into()),
            words: vec![word("hello", 0.0, 0.5), word("there", 0.5, 1.0)],
            segments: vec![segment("hello there", 0.0, 1.0)],
        })
    };

    let request = CaptionRequest::default();
    let mut counter = 0;
    let new_group = || {
        counter += 1;
        format!("group-{counter}")
    };

    let result = generate_captions(
        &mut timeline,
        &mut history,
        &request,
        &NoAssetInfo,
        &mut registry,
        &mut layout,
        transcribe,
        new_group,
    )
    .expect("generate");

    // One caption clip placed.
    assert_eq!(result.clip_ids.len(), 1, "expected one caption clip");
    // A NEW video track was inserted at index 0.
    assert_eq!(timeline.tracks.len(), 2);
    assert_eq!(timeline.tracks[0].track_type, ClipType::Video);
    let caption = &timeline.tracks[0].clips[0];
    assert_eq!(caption.media_type, ClipType::Text);
    assert_eq!(caption.text_content.as_deref(), Some("hello there"));
    assert_eq!(caption.caption_group_id.as_deref(), Some("group-1"));
    // The undo entry is named exactly "Generate Captions" (agent-undo parity).
    assert_eq!(history.current_undo_action_name(), Some(GENERATE_CAPTIONS_UNDO_NAME));
}

/// `generate_captions` with no captionable clips → `NoSource`.
#[test]
fn generate_captions_no_source_errors() {
    let mut timeline = Timeline::new();
    let mut history: History<Timeline> = History::new();
    let mut registry = FontRegistry::bundled_only();
    let mut layout = TextLayout::new();
    let transcribe = |_: &str, _: Option<&RangeInclusive<f64>>, _: bool| {
        Ok::<_, std::convert::Infallible>(TranscriptionResult {
            text: String::new(),
            language: None,
            words: vec![],
            segments: vec![],
        })
    };
    let err = generate_captions(
        &mut timeline,
        &mut history,
        &CaptionRequest::default(),
        &NoAssetInfo,
        &mut registry,
        &mut layout,
        transcribe,
        || "g".to_string(),
    )
    .unwrap_err();
    assert_eq!(err, CaptionError::NoSource);
}

/// Casing is applied to caption text (upper) before placement.
#[test]
fn casing_is_applied_to_caption_text() {
    let mut timeline = Timeline::new();
    let mut audio = Track::new(ClipType::Audio);
    let mut clip = Clip::new("m", 0, 120);
    clip.media_type = ClipType::Audio;
    audio.clips.push(clip);
    timeline.tracks.push(audio);

    let mut history: History<Timeline> = History::new();
    let mut registry = FontRegistry::bundled_only();
    let mut layout = TextLayout::new();
    let transcribe = |_: &str, _: Option<&RangeInclusive<f64>>, _: bool| {
        Ok::<_, std::convert::Infallible>(TranscriptionResult {
            text: "hi".into(),
            language: Some("en".into()),
            words: vec![word("hi", 0.0, 0.5)],
            segments: vec![segment("hi", 0.0, 0.5)],
        })
    };
    let mut req = CaptionRequest::default();
    req.text_case = CaptionCase::Upper;
    let result = generate_captions(
        &mut timeline,
        &mut history,
        &req,
        &NoAssetInfo,
        &mut registry,
        &mut layout,
        transcribe,
        || "g".to_string(),
    )
    .expect("generate");
    assert_eq!(result.clip_ids.len(), 1);
    assert_eq!(timeline.tracks[0].clips[0].text_content.as_deref(), Some("HI"));
}
