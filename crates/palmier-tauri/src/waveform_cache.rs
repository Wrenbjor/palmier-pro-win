//! Per-asset audio-waveform cache for the timeline canvas (`editor_get_timeline`).
//!
//! ## The product gap this closes
//! The timeline renderer (`src-ui/src/editor/renderer.ts` `drawWaveform`) already
//! draws an audio waveform whenever a `ClipView.waveform: number[]` is present, but
//! the live `editor_get_timeline` path serialized the timeline WITHOUT waveform data
//! (the full-fidelity serializer omits it — "computed at render time"). So audio
//! clips rendered as flat placeholder bars. This module computes the per-asset peak
//! array (via the tested `palmier_media::waveform` pipeline) and the command layer
//! injects it onto each audio clip's JSON.
//!
//! ## Why per-asset, not per-clip
//! The renderer takes the **full source** waveform and slices it to the clip's
//! visible/trimmed window itself (`trimStartFrame / sourceDurationFrames` →
//! `sampleStart..sampleEnd`). So we cache ONE peak array per media asset (keyed by
//! its resolved absolute path) and hand the same array to every clip that references
//! that asset. The renderer does the trim slicing.
//!
//! ## Caching + cost
//! `editor_get_timeline` fires often (every `timeline://changed`), so the waveform
//! must NOT be recomputed per call:
//! - **Hot path** — an in-memory `HashMap<path, Vec<f32>>` answers synchronously.
//! - **Warm path** — the disk-backed [`palmier_media::WaveformCache`] (`.waveform`
//!   blobs under the media-visual cache dir) is read once and promoted to memory.
//! - **Cold path** — a miss spawns a background decode (symphonia → downsample) on
//!   the Tauri async runtime; the command returns WITHOUT a waveform for that asset
//!   this call, and when generation finishes we emit `timeline://changed` so the UI
//!   refetches and picks up the now-cached peaks. In-flight keys are de-duped so a
//!   burst of refetches kicks off at most one decode per asset.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tauri::{AppHandle, Emitter, Runtime};

use palmier_media::WaveformCache;

use crate::commands::TIMELINE_CHANGED_EVENT;

/// Managed state: the disk-backed per-asset waveform cache + an in-memory promotion
/// layer + the in-flight de-dup set. Built once at boot.
pub struct WaveformState {
    /// Disk cache (`.waveform` blobs under the media-visual cache dir). `None` when
    /// the platform cache root can't be resolved (degrades to in-memory only).
    disk: Option<WaveformCache>,
    /// In-memory promoted peaks, keyed by resolved absolute path string. The hot
    /// synchronous path the command reads on every `editor_get_timeline`.
    mem: Arc<Mutex<HashMap<String, Vec<f32>>>>,
    /// Paths with a background generation in flight — so a burst of refetches kicks
    /// off at most one decode per asset.
    in_flight: Arc<Mutex<HashSet<String>>>,
}

impl WaveformState {
    /// Build the cache rooted at the media-visual cache dir (parity with the
    /// thumbnail/audio caches). Falls back to in-memory only if that dir is
    /// unavailable.
    #[must_use]
    pub fn new() -> Self {
        let disk = palmier_media::cache::media_visual_cache_dir().map(WaveformCache::new);
        WaveformState {
            disk,
            mem: Arc::new(Mutex::new(HashMap::new())),
            in_flight: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Synchronous lookup: in-memory hit, else a disk-blob read promoted to memory,
    /// else `None`. NEVER decodes (the cold path is [`Self::spawn_generate`]).
    pub fn get(&self, path: &Path) -> Option<Vec<f32>> {
        let key = path.to_string_lossy().into_owned();
        if let Some(hit) = self.mem.lock().expect("waveform mem mutex").get(&key) {
            return Some(hit.clone());
        }
        // Warm path: the on-disk `.waveform` blob (cheap read + parse, no decode).
        let samples = self.disk.as_ref().and_then(|c| c.load(path))?;
        self.mem
            .lock()
            .expect("waveform mem mutex")
            .insert(key, samples.clone());
        Some(samples)
    }

    /// Synchronous generate + store (decode → downsample → cache). Used by tests and
    /// any caller that already runs on a worker; the live command path uses the async
    /// [`Self::spawn_generate`] instead so it never blocks `editor_get_timeline`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn generate_blocking(&self, path: &Path, duration: f64) -> Option<Vec<f32>> {
        let samples = palmier_media::generate_waveform(path, duration).ok()?;
        if samples.is_empty() {
            return None;
        }
        self.mem
            .lock()
            .expect("waveform mem mutex")
            .insert(path.to_string_lossy().into_owned(), samples.clone());
        Some(samples)
    }

    /// Cold path: kick off a background decode+downsample for `path` (of source
    /// `duration` seconds) if one isn't already in flight. On completion, store the
    /// peaks in memory and emit `timeline://changed` so the UI refetches and the next
    /// `editor_get_timeline` returns the waveform synchronously.
    pub fn spawn_generate<R: Runtime>(&self, app: &AppHandle<R>, path: PathBuf, duration: f64) {
        let key = path.to_string_lossy().into_owned();
        {
            let mut flight = self.in_flight.lock().expect("waveform in-flight mutex");
            if flight.contains(&key)
                || self.mem.lock().expect("waveform mem mutex").contains_key(&key)
            {
                return;
            }
            flight.insert(key.clone());
        }

        // Clone the disk cache (it's `Clone`, sharing the gate state) so the task owns
        // its own handle. If there's no disk cache we still decode (uncached on disk,
        // cached in memory) so a no-cache-dir box still shows waveforms.
        let disk = self.disk.clone();
        let mem = Arc::clone(&self.mem);
        let in_flight = Arc::clone(&self.in_flight);
        let app = app.clone();

        tauri::async_runtime::spawn(async move {
            let result = match &disk {
                Some(cache) => cache.generate(&path, duration).await,
                None => {
                    let p = path.clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        palmier_media::generate_waveform(&p, duration)
                    })
                    .await
                    .unwrap_or_else(|e| Err(palmier_media::WaveformError::Io(e.to_string())))
                }
            };

            match result {
                Ok(samples) if !samples.is_empty() => {
                    mem.lock()
                        .expect("waveform mem mutex")
                        .insert(key.clone(), samples);
                    in_flight
                        .lock()
                        .expect("waveform in-flight mutex")
                        .remove(&key);
                    // Nudge the UI to refetch — the next read returns the waveform.
                    if let Err(err) = app.emit(TIMELINE_CHANGED_EVENT, ()) {
                        tracing::warn!(target: "app", error = %err, "waveform: failed to emit timeline://changed");
                    }
                }
                Ok(_) => {
                    // Empty (silent/no audio) — record nothing but clear in-flight so a
                    // later refetch can retry if the asset is replaced.
                    in_flight
                        .lock()
                        .expect("waveform in-flight mutex")
                        .remove(&key);
                }
                Err(err) => {
                    tracing::debug!(target: "app", path = %key, error = %err, "waveform generation failed");
                    in_flight
                        .lock()
                        .expect("waveform in-flight mutex")
                        .remove(&key);
                }
            }
        });
    }
}

impl Default for WaveformState {
    fn default() -> Self {
        WaveformState::new()
    }
}

/// A resolved audio asset: the absolute path + source duration (seconds) needed to
/// generate (and key) its waveform.
pub struct AudioAsset {
    pub path: PathBuf,
    pub duration: f64,
}

/// Inject `waveform` peaks onto every **audio** clip in a `full_timeline_json` value.
///
/// `assets` maps a clip's `mediaRef` → its resolved audio source (path + duration).
/// For each audio clip whose asset resolves: a cache hit injects the peaks; a miss
/// calls `on_miss(media_ref, asset)` (the live command kicks off a background decode
/// there; the clip is left without a waveform this call so the renderer falls back to
/// placeholder bars until the refetch).
///
/// Only clips with `mediaType == "audio"` get a waveform: the renderer only draws the
/// waveform for audio clips (video clips show a thumbnail), so computing peaks for
/// video audio would be wasted work.
///
/// The `on_miss` callback (rather than a hard `AppHandle` dependency) keeps this core
/// unit-testable without a live Tauri runtime — the command supplies the spawn, tests
/// supply a synchronous generate.
pub fn inject_waveforms(
    state: &WaveformState,
    assets: &HashMap<String, AudioAsset>,
    mut timeline: Value,
    mut on_miss: impl FnMut(&str, &AudioAsset),
) -> Value {
    let Some(tracks) = timeline.get_mut("tracks").and_then(Value::as_array_mut) else {
        return timeline;
    };
    for track in tracks.iter_mut() {
        let Some(clips) = track.get_mut("clips").and_then(Value::as_array_mut) else {
            continue;
        };
        for clip in clips.iter_mut() {
            if clip.get("mediaType").and_then(Value::as_str) != Some("audio") {
                continue;
            }
            let Some(media_ref) = clip.get("mediaRef").and_then(Value::as_str).map(str::to_owned)
            else {
                continue;
            };
            let Some(asset) = assets.get(&media_ref) else {
                continue;
            };
            match state.get(&asset.path) {
                Some(samples) => {
                    let arr = Value::Array(
                        samples
                            .into_iter()
                            .map(|s| Value::from(f64::from(s)))
                            .collect(),
                    );
                    if let Some(obj) = clip.as_object_mut() {
                        obj.insert("waveform".to_string(), arr);
                    }
                }
                None => on_miss(&media_ref, asset),
            }
        }
    }
    timeline
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A timeline JSON with one audio clip referencing `asset-a` and a video clip
    /// referencing `asset-v`, shaped like `full_timeline_json` output.
    fn timeline_json() -> Value {
        json!({
            "fps": 30,
            "tracks": [
                { "clips": [
                    { "id": "c1", "mediaRef": "asset-a", "mediaType": "audio",
                      "startFrame": 0, "durationFrames": 90 },
                    { "id": "c2", "mediaRef": "asset-v", "mediaType": "video",
                      "startFrame": 0, "durationFrames": 90 },
                ]}
            ]
        })
    }

    #[test]
    fn injects_waveform_on_audio_clip_when_cached() {
        let state = WaveformState::new();
        // Seed the in-memory cache for the audio asset's path directly.
        let path = PathBuf::from("/audio/a.wav");
        state
            .mem
            .lock()
            .unwrap()
            .insert(path.to_string_lossy().into_owned(), vec![1.0, 0.5, 0.0]);

        let mut assets = HashMap::new();
        assets.insert(
            "asset-a".to_string(),
            AudioAsset { path: path.clone(), duration: 3.0 },
        );

        let mut missed: Vec<String> = Vec::new();
        let out = inject_waveforms(&state, &assets, timeline_json(), |r, _| {
            missed.push(r.to_string())
        });

        // The audio clip carries the peaks; the video clip does not.
        let clips = out["tracks"][0]["clips"].as_array().unwrap();
        let audio = &clips[0];
        let video = &clips[1];
        let w = audio["waveform"].as_array().expect("audio clip has waveform");
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].as_f64(), Some(1.0));
        assert!(video.get("waveform").is_none(), "video clip gets no waveform");
        assert!(missed.is_empty(), "cached → no miss callback");
    }

    /// END-TO-END: a real audio-bearing clip (an ffmpeg-generated tone) decodes to a
    /// non-empty waveform, and the SERIALIZED `editor_get_timeline` JSON carries it on
    /// the audio clip. Self-skips when ffmpeg isn't on PATH (under the MSVC+ffmpeg-env
    /// wrapper it runs for real and is the GREEN gate). Mirrors the fixture pattern in
    /// `palmier-media/tests/decode_audio_pcm.rs`.
    #[test]
    fn real_audio_clip_yields_nonempty_waveform_in_serialized_output() {
        use std::process::Command;

        let dir = tempfile::tempdir().expect("tempdir");
        let clip = dir.path().join("tone.wav");
        let status = Command::new("ffmpeg")
            .arg("-y")
            .args([
                "-f", "lavfi", "-i",
                "sine=frequency=440:duration=2:sample_rate=44100",
                "-ac", "1",
            ])
            .arg(&clip)
            .status();
        match status {
            Ok(s) if s.success() && clip.exists() => {}
            _ => {
                eprintln!("ffmpeg not available — skipping real-clip waveform test");
                return;
            }
        }

        // Generate + cache the waveform for the real clip (decode → downsample).
        let state = WaveformState::new();
        let peaks = state
            .generate_blocking(&clip, 2.0)
            .expect("real audio clip generates a non-empty waveform");
        assert!(!peaks.is_empty(), "decoded peaks non-empty");
        assert!(
            peaks.iter().all(|&v| (0.0..=1.0).contains(&v)),
            "peaks are dB-normalised into [0,1]"
        );

        // A timeline JSON whose audio clip references the real fixture path.
        let timeline = json!({
            "fps": 30,
            "tracks": [
                { "clips": [
                    { "id": "c1", "mediaRef": "asset-a", "mediaType": "audio",
                      "startFrame": 0, "durationFrames": 60 },
                ]}
            ]
        });
        let mut assets = HashMap::new();
        assets.insert(
            "asset-a".to_string(),
            AudioAsset { path: clip.clone(), duration: 2.0 },
        );

        // Inject + serialize exactly as `editor_get_timeline` does.
        let out = inject_waveforms(&state, &assets, timeline, |_, _| {
            panic!("cached → must not miss")
        });
        let serialized = serde_json::to_string(&out).expect("serializes");
        let reparsed: Value = serde_json::from_str(&serialized).expect("reparses");

        let w = reparsed["tracks"][0]["clips"][0]["waveform"]
            .as_array()
            .expect("serialized audio clip carries a waveform array");
        assert!(!w.is_empty(), "serialized waveform is non-empty");
        assert_eq!(w.len(), peaks.len(), "serialized peak count matches generation");
    }

    #[test]
    fn cold_miss_calls_callback_and_leaves_no_waveform() {
        let state = WaveformState::new();
        let mut assets = HashMap::new();
        assets.insert(
            "asset-a".to_string(),
            AudioAsset { path: PathBuf::from("/audio/missing.wav"), duration: 3.0 },
        );

        let mut missed: Vec<String> = Vec::new();
        let out = inject_waveforms(&state, &assets, timeline_json(), |r, _| {
            missed.push(r.to_string())
        });

        assert_eq!(missed, vec!["asset-a".to_string()], "uncached audio → one miss");
        let audio = &out["tracks"][0]["clips"][0];
        assert!(audio.get("waveform").is_none(), "miss → no waveform this call");
    }
}
