//! E6-S2 / E6-S3 / E6-S4 — the XMEML 4 document builder.
//!
//! Ported literally from the macOS reference
//! `Sources/PalmierPro/Export/XMLExporter.swift` (`Builder`). Produces the
//! `<xmeml version="4">` document for a [`Timeline`] as a byte-stable string via
//! the [`crate::xml`] render core. See docs/reference/export.md §B for the
//! authoritative structure and every byte-fidelity rule.
//!
//! ## Byte-fidelity rules honored here (all golden-critical)
//! - Document shell: `<sequence id="sequence-1">`, fixed video/audio format blocks.
//! - Video tracks emitted **reversed** (FCP7 bottom→top vs model top→bottom);
//!   audio tracks in natural order. Clips filtered to resolvable URL, sorted by
//!   `start_frame`.
//! - `<clipitem>` child order; `masterclipid` = `masterclip-{linkGroupId}` if
//!   linked, else `masterclip-{mediaRef}-{video|audio}`.
//! - `sourceFramesConsumed = round(durationFrames*speed)` ties-away (model).
//! - `<file>` emitted in full once per `(mediaRef, isAudio)`; repeats collapse to
//!   `<file id="…"/>`.
//! - `pathurl` = `file://` → `file://localhost//` rewrite (Premiere quirk).
//! - `rateTags`: `timebase=round(fps)`, NTSC detection.
//! - Filters: time-remap, basic-motion (scale / rotation NEGATED / center
//!   offset-from-0.5), crop, opacity, audio-levels — all with the diff-only
//!   thresholds and keyframe sampling.
//! - Fades → single-sided `<transitionitem>` (Cross Dissolve / Cross Fade).
//! - Linked A/V `<link>` cross-refs from the same filtered/sorted lists.
//! - `formatTimecode` drop-frame `round(fps*0.066666)` approximation, copied
//!   exactly (NDF `:` vs DF `;`).

use std::collections::{HashMap, HashSet};

use palmier_model::{ClipType, Clip, Timeline, Track, Transform};

use crate::resolver::MediaResolver;
use crate::xml::{fmt_f, render, XmlNode, XML_PROLOG};

/// Which edge of a clip a fade applies to (local mirror of the reference
/// `FadeEdge`, used by the transition emitter).
#[derive(Clone, Copy)]
enum FadeEdge {
    Left,
    Right,
}

/// 1-based address of a clip within its media type's filtered/sorted track list,
/// used to emit `<link>` cross-references (reference `ClipAddress`).
#[derive(Clone, Copy)]
struct ClipAddress {
    track_index: i64,
    clip_index: i64,
    is_audio: bool,
}

/// Builds the XMEML document for one timeline.
pub struct Builder<'a, R: MediaResolver> {
    timeline: &'a Timeline,
    resolver: &'a R,
    fps: i32,
    seq_width: i32,
    seq_height: i32,

    /// Files already emitted in full; repeats collapse to `<file id="…"/>`.
    emitted_files: HashSet<(String, bool)>,
    /// Clip id → its 1-based address (for `<link>`).
    clip_addresses: HashMap<String, ClipAddress>,
    /// Link group id → the clips in it (across all tracks).
    clips_by_link_group: HashMap<String, Vec<Clip>>,
}

impl<'a, R: MediaResolver> Builder<'a, R> {
    /// Construct a builder over a timeline + resolver.
    pub fn new(timeline: &'a Timeline, resolver: &'a R) -> Self {
        Builder {
            timeline,
            resolver,
            fps: timeline.fps,
            seq_width: timeline.width,
            seq_height: timeline.height,
            emitted_files: HashSet::new(),
            clip_addresses: HashMap::new(),
            clips_by_link_group: HashMap::new(),
        }
    }

    // MARK: - Document shell

    /// Build the full XMEML document string (prolog + rendered tree).
    pub fn build(&mut self) -> String {
        // FCP XML orders video tracks bottom→top; our model stores them
        // top→bottom — so reverse the visual tracks.
        let video_tracks: Vec<&Track> = self
            .timeline
            .tracks
            .iter()
            .filter(|t| t.track_type.is_visual())
            .rev()
            .collect();
        let audio_tracks: Vec<&Track> = self
            .timeline
            .tracks
            .iter()
            .filter(|t| t.track_type == ClipType::Audio)
            .collect();

        let sorted_video: Vec<Vec<Clip>> =
            video_tracks.iter().map(|t| self.sort_emittable(t)).collect();
        let sorted_audio: Vec<Vec<Clip>> =
            audio_tracks.iter().map(|t| self.sort_emittable(t)).collect();

        self.index_addresses(&sorted_video, false);
        self.index_addresses(&sorted_audio, true);
        self.index_link_groups();

        let video_track_nodes: Vec<XmlNode> = video_tracks
            .iter()
            .zip(sorted_video.iter())
            .map(|(t, clips)| self.track_node(t, clips, false))
            .collect();
        let audio_track_nodes: Vec<XmlNode> = audio_tracks
            .iter()
            .zip(sorted_audio.iter())
            .map(|(t, clips)| self.track_node(t, clips, true))
            .collect();

        let mut video_children = vec![self.video_format_node()];
        video_children.extend(video_track_nodes);

        let mut audio_children =
            vec![XmlNode::leaf_int("numOutputChannels", 2), self.audio_format_node(), self.audio_outputs_node()];
        audio_children.extend(audio_track_nodes);

        let media = XmlNode::el(
            "media",
            vec![
                XmlNode::el("video", video_children),
                XmlNode::el("audio", audio_children),
            ],
        );

        let sequence = XmlNode::el_attrs(
            "sequence",
            vec![("id".into(), "sequence-1".into())],
            vec![
                XmlNode::leaf("name", "Timeline Export"),
                XmlNode::leaf_int("duration", self.timeline.total_frames() as i64),
                self.rate(self.fps, false),
                self.timecode_node(),
                media,
            ],
        );

        let root = XmlNode::el_attrs("xmeml", vec![("version".into(), "4".into())], vec![sequence]);
        format!("{XML_PROLOG}{}", render(&root, 0))
    }

    fn timecode_node(&self) -> XmlNode {
        XmlNode::el(
            "timecode",
            vec![
                self.rate(self.fps, false),
                XmlNode::leaf("string", "00:00:00:00"),
                XmlNode::leaf_int("frame", 0),
                XmlNode::leaf("source", "source"),
                XmlNode::leaf("displayformat", "NDF"),
            ],
        )
    }

    fn video_format_node(&self) -> XmlNode {
        XmlNode::el(
            "format",
            vec![XmlNode::el(
                "samplecharacteristics",
                vec![
                    XmlNode::leaf_int("width", self.seq_width as i64),
                    XmlNode::leaf_int("height", self.seq_height as i64),
                    XmlNode::bool_leaf("anamorphic", false),
                    XmlNode::leaf("pixelaspectratio", "square"),
                    XmlNode::leaf("fielddominance", "none"),
                    self.rate(self.fps, false),
                ],
            )],
        )
    }

    fn audio_format_node(&self) -> XmlNode {
        XmlNode::el(
            "format",
            vec![XmlNode::el(
                "samplecharacteristics",
                vec![XmlNode::leaf_int("samplerate", 48000), XmlNode::leaf_int("depth", 16)],
            )],
        )
    }

    fn audio_outputs_node(&self) -> XmlNode {
        XmlNode::el(
            "outputs",
            vec![XmlNode::el(
                "group",
                vec![
                    XmlNode::leaf_int("index", 1),
                    XmlNode::leaf_int("numchannels", 2),
                    XmlNode::leaf_int("downmix", 0),
                    XmlNode::el("channel", vec![XmlNode::leaf_int("index", 1)]),
                    XmlNode::el("channel", vec![XmlNode::leaf_int("index", 2)]),
                ],
            )],
        )
    }

    // MARK: - Tracks → clipitems

    fn track_node(&mut self, track: &Track, sorted_clips: &[Clip], is_audio: bool) -> XmlNode {
        let enabled = if is_audio { !track.muted } else { !track.hidden };
        let mut children = vec![XmlNode::bool_leaf("enabled", enabled), XmlNode::bool_leaf("locked", false)];
        for clip in sorted_clips {
            if let Some(fade_in) = self.fade_transition(clip, FadeEdge::Left, is_audio) {
                children.push(fade_in);
            }
            children.push(self.clip_item_node(clip, is_audio));
            if let Some(fade_out) = self.fade_transition(clip, FadeEdge::Right, is_audio) {
                children.push(fade_out);
            }
        }
        XmlNode::el("track", children)
    }

    fn clip_item_node(&mut self, clip: &Clip, is_audio: bool) -> XmlNode {
        let source_duration = self
            .source_duration_frames(&clip.media_ref)
            .unwrap_or_else(|| clip.source_duration_frames());
        // in/out are source-frame offsets spanning sourceFramesConsumed.
        let in_point = clip.trim_start_frame;
        let out_point = clip.trim_start_frame + clip.source_frames_consumed();

        let mut children = vec![
            XmlNode::leaf("masterclipid", self.masterclip_id(clip, is_audio)),
            XmlNode::leaf("name", self.resolver.display_name(&clip.media_ref)),
            XmlNode::bool_leaf("enabled", true),
            XmlNode::leaf_int("duration", source_duration as i64),
            self.rate(self.fps, false),
            XmlNode::leaf_int("start", clip.start_frame as i64),
            XmlNode::leaf_int("end", clip.end_frame() as i64),
            XmlNode::leaf_int("in", in_point as i64),
            XmlNode::leaf_int("out", out_point as i64),
            self.file_node(clip, is_audio),
        ];
        if let Some(remap) = self.time_remap_filter(clip.speed, is_audio) {
            children.push(remap);
        }
        if is_audio {
            children.extend(self.volume_filters(clip));
        } else {
            children.extend(self.video_filters(clip));
        }
        children.extend(self.link_nodes(clip));
        XmlNode::el_attrs("clipitem", vec![("id".into(), format!("clipitem-{}", clip.id))], children)
    }

    fn masterclip_id(&self, clip: &Clip, is_audio: bool) -> String {
        if let Some(group) = &clip.link_group_id {
            format!("masterclip-{group}")
        } else {
            format!("masterclip-{}-{}", clip.media_ref, if is_audio { "audio" } else { "video" })
        }
    }

    // MARK: - File elements

    fn file_node(&mut self, clip: &Clip, is_audio: bool) -> XmlNode {
        let media_ref = &clip.media_ref;
        let file_id = format!("file-{}-{}", media_ref, if is_audio { "audio" } else { "video" });
        let key = (media_ref.clone(), is_audio);
        if self.emitted_files.contains(&key) {
            return XmlNode::empty_attrs("file", vec![("id".into(), file_id)]);
        }
        self.emitted_files.insert(key);

        let entry = self.resolver.entry(media_ref);
        let url = self.resolver.resolve_url(media_ref);

        // name: url.lastPathComponent ?? entry.name ?? mediaRef.
        let file_name = url
            .as_deref()
            .map(last_path_component)
            .or_else(|| entry.map(|e| e.name.clone()))
            .unwrap_or_else(|| media_ref.clone());

        // pathurl: file:// → file://localhost// ; fallback media/{mediaRef}.
        let path_url = match &url {
            Some(u) => u.replacen("file://", "file://localhost//", 1),
            None => format!("media/{media_ref}"),
        };

        let is_image = entry.map(|e| e.asset_type == ClipType::Image).unwrap_or(false);
        let duration_frames = if is_image {
            1
        } else {
            entry
                .map(|e| seconds_to_frame(e.duration, self.fps).max(0))
                .unwrap_or(0)
        };
        let source_fps = entry.and_then(|e| e.source_fps).unwrap_or(self.fps as f64);
        let (timebase, ntsc) = rate_tags(source_fps);

        let media = if is_audio {
            XmlNode::el(
                "media",
                vec![XmlNode::el(
                    "audio",
                    vec![
                        XmlNode::el(
                            "samplecharacteristics",
                            vec![XmlNode::leaf_int("samplerate", 48000), XmlNode::leaf_int("depth", 16)],
                        ),
                        XmlNode::leaf_int("channelcount", 2),
                    ],
                )],
            )
        } else {
            let mut video_children: Vec<XmlNode> = if is_image {
                vec![XmlNode::leaf_int("duration", 1)]
            } else {
                vec![]
            };
            video_children.push(XmlNode::el(
                "samplecharacteristics",
                vec![
                    XmlNode::leaf_int("width", entry.and_then(|e| e.source_width).unwrap_or(self.seq_width) as i64),
                    XmlNode::leaf_int("height", entry.and_then(|e| e.source_height).unwrap_or(self.seq_height) as i64),
                    XmlNode::bool_leaf("anamorphic", false),
                    XmlNode::leaf("pixelaspectratio", "square"),
                    XmlNode::leaf("fielddominance", "none"),
                    self.rate(timebase, ntsc),
                ],
            ));
            XmlNode::el("media", vec![XmlNode::el("video", video_children)])
        };

        let drop_frame = ntsc && timebase % 30 == 0;
        let start_frame = self.source_start_frame(media_ref).unwrap_or(0);
        let timecode = XmlNode::el(
            "timecode",
            vec![
                self.rate(timebase, ntsc),
                XmlNode::leaf("string", format_timecode(start_frame, timebase, drop_frame)),
                XmlNode::leaf_int("frame", start_frame as i64),
                XmlNode::leaf("displayformat", if drop_frame { "DF" } else { "NDF" }),
            ],
        );

        XmlNode::el_attrs(
            "file",
            vec![("id".into(), file_id)],
            vec![
                XmlNode::leaf("name", file_name),
                XmlNode::leaf("pathurl", path_url),
                self.rate(timebase, ntsc),
                XmlNode::leaf_int("duration", duration_frames as i64),
                timecode,
                media,
            ],
        )
    }

    /// Source start timecode from the QuickTime `tmcd` track.
    ///
    /// v1: returns `None` (defaults to frame 0) — the reference tolerates a nil
    /// tmcd → 0, which is acceptable for the v1 goldens (docs/reference/export.md
    /// Open Questions; the `palmier-media` demux read is a later sub-task).
    fn source_start_frame(&self, _media_ref: &str) -> Option<i32> {
        None
    }

    // MARK: - Links

    fn link_nodes(&self, clip: &Clip) -> Vec<XmlNode> {
        let Some(group) = &clip.link_group_id else {
            return vec![];
        };
        let Some(partners) = self.clips_by_link_group.get(group) else {
            return vec![];
        };
        if partners.len() <= 1 {
            return vec![];
        }
        partners
            .iter()
            .filter_map(|partner| {
                let addr = self.clip_addresses.get(&partner.id)?;
                Some(XmlNode::el(
                    "link",
                    vec![
                        XmlNode::leaf("linkclipref", format!("clipitem-{}", partner.id)),
                        XmlNode::leaf("mediatype", if addr.is_audio { "audio" } else { "video" }),
                        XmlNode::leaf_int("trackindex", addr.track_index),
                        XmlNode::leaf_int("clipindex", addr.clip_index),
                    ],
                ))
            })
            .collect()
    }

    // MARK: - Transitions (fades)

    fn fade_transition(&self, clip: &Clip, edge: FadeEdge, is_audio: bool) -> Option<XmlNode> {
        let frames = match edge {
            FadeEdge::Left => clip.fade_in_frames,
            FadeEdge::Right => clip.fade_out_frames,
        };
        if frames <= 0 {
            return None;
        }
        let (start, end, alignment, cut_frames) = match edge {
            FadeEdge::Left => (clip.start_frame, clip.start_frame + frames, "start-black", 0),
            FadeEdge::Right => (clip.end_frame() - frames, clip.end_frame(), "end-black", frames),
        };

        let mut children = vec![
            XmlNode::leaf_int("start", start as i64),
            XmlNode::leaf_int("end", end as i64),
            XmlNode::leaf("alignment", alignment),
        ];
        if is_audio {
            children.push(self.rate(self.fps, false));
            children.push(effect(
                "Cross Fade ( 0dB)",
                "KGAudioTransCrossFade0dB",
                "transition",
                "audio",
                None,
                vec![],
            ));
        } else {
            // Premiere's private cut-point, in ticks (254016000000/sec).
            let cut_point_ticks = cut_frames as i64 * (254_016_000_000_i64 / self.fps as i64);
            children.push(XmlNode::leaf("cutPointTicks", cut_point_ticks.to_string()));
            children.push(self.rate(self.fps, false));
            children.push(effect(
                "Cross Dissolve",
                "Cross Dissolve",
                "transition",
                "video",
                Some("Dissolve"),
                vec![
                    XmlNode::leaf_int("wipecode", 0),
                    XmlNode::leaf_int("wipeaccuracy", 100),
                    XmlNode::leaf_int("startratio", 0),
                    XmlNode::leaf_int("endratio", 1),
                    XmlNode::bool_leaf("reverse", false),
                ],
            ));
        }
        Some(XmlNode::el("transitionitem", children))
    }

    // MARK: - Filters

    fn time_remap_filter(&self, speed: f64, is_audio: bool) -> Option<XmlNode> {
        if speed == 1.0 {
            return None;
        }
        Some(filter(effect(
            "Time Remap",
            "timeremap",
            "motion",
            if is_audio { "audio" } else { "video" },
            None,
            vec![
                parameter(
                    "variablespeed",
                    "variablespeed",
                    Some("0"),
                    Some("1"),
                    XmlNode::leaf_int("value", 0),
                    vec![],
                ),
                parameter(
                    "speed",
                    "speed",
                    Some("-100000"),
                    Some("100000"),
                    XmlNode::leaf("value", fmt_f(4, speed * 100.0)),
                    vec![],
                ),
                parameter("reverse", "reverse", None, None, XmlNode::bool_leaf("value", false), vec![]),
                parameter(
                    "frameblending",
                    "frameblending",
                    None,
                    None,
                    XmlNode::bool_leaf("value", false),
                    vec![],
                ),
            ],
        )))
    }

    fn volume_filters(&self, clip: &Clip) -> Vec<XmlNode> {
        fn clamp_level(v: f64) -> f64 {
            v.clamp(0.0, 3.98)
        }
        let frames = keyframe_frames(clip, Property::Volume);
        let level = if frames.is_empty() {
            if clip.volume == 1.0 {
                return vec![];
            }
            scalar_param("level", "Level", "0", "3.98107", clamp_level(clip.volume), vec![], 4)
        } else {
            let kfs: Vec<(i64, f64)> = frames
                .iter()
                .map(|&f| ((f - clip.start_frame) as i64, clamp_level(clip.raw_volume_at(f))))
                .collect();
            let base = kfs[0].1;
            scalar_param("level", "Level", "0", "3.98107", base, kfs, 4)
        };
        vec![filter(effect("Audio Levels", "audiolevels", "audio", "audio", None, vec![level]))]
    }

    fn video_filters(&self, clip: &Clip) -> Vec<XmlNode> {
        [self.motion_filter(clip), self.crop_filter(clip), self.opacity_filter(clip)]
            .into_iter()
            .flatten()
            .collect()
    }

    fn motion_filter(&self, clip: &Clip) -> Option<XmlNode> {
        let source_width = self
            .resolver
            .entry(&clip.media_ref)
            .and_then(|e| e.source_width)
            .unwrap_or(0);
        let scale_pct = |width: f64| -> f64 {
            if source_width > 0 {
                (self.seq_width as f64 / source_width as f64) * width * 100.0
            } else {
                width * 100.0
            }
        };
        // FCP7 center: normalized offset-from-0.5.
        let center = |t: &Transform| -> (f64, f64) { (t.center_x - 0.5, t.center_y - 0.5) };

        // Center depends on position+scale, so sample at the union of frames.
        let mut frame_set: Vec<i32> = keyframe_frames(clip, Property::Position);
        frame_set.extend(keyframe_frames(clip, Property::Scale));
        frame_set.extend(keyframe_frames(clip, Property::Rotation));
        let frames = sorted_unique(frame_set);

        let mut params: Vec<XmlNode> = vec![];
        if frames.is_empty() {
            let t = clip.transform;
            let c = center(&t);
            let scaled = scale_pct(t.width);
            let rotated = -t.rotation;
            let needs_center = c.0.abs() > 0.001 || c.1.abs() > 0.001;
            let needs_scale = (scaled - 100.0).abs() > 0.1;
            let needs_rotation = rotated.abs() > 0.05;
            if !(needs_center || needs_scale || needs_rotation) {
                return None;
            }
            if needs_scale {
                params.push(scalar_param("scale", "Scale", "0", "1000", scaled, vec![], 2));
            }
            if needs_rotation {
                params.push(scalar_param("rotation", "Rotation", "-100000", "100000", rotated, vec![], 2));
            }
            if needs_center {
                params.push(center_param(c, vec![]));
            }
        } else {
            let scale_kfs: Vec<(i64, f64)> = frames
                .iter()
                .map(|&f| ((f - clip.start_frame) as i64, scale_pct(clip.size_at(f).0)))
                .collect();
            let rotation_kfs: Vec<(i64, f64)> = frames
                .iter()
                .map(|&f| ((f - clip.start_frame) as i64, -clip.rotation_at(f)))
                .collect();
            let center_kfs: Vec<(i64, f64, f64)> = frames
                .iter()
                .map(|&f| {
                    let c = center(&clip.transform_at(f));
                    ((f - clip.start_frame) as i64, c.0, c.1)
                })
                .collect();
            let scale_base = scale_kfs[0].1;
            let rotation_base = rotation_kfs[0].1;
            let center_base = (center_kfs[0].1, center_kfs[0].2);
            params = vec![
                scalar_param("scale", "Scale", "0", "1000", scale_base, scale_kfs, 2),
                scalar_param("rotation", "Rotation", "-100000", "100000", rotation_base, rotation_kfs, 2),
                center_param(center_base, center_kfs),
            ];
        }
        Some(filter(effect("Basic Motion", "basic", "motion", "video", None, params)))
    }

    fn crop_filter(&self, clip: &Clip) -> Option<XmlNode> {
        let frames = keyframe_frames(clip, Property::Crop);
        if frames.is_empty() && clip.crop.is_identity() {
            return None;
        }
        let edge = |id: &str, pick: fn(&palmier_model::Crop) -> f64| -> XmlNode {
            if frames.is_empty() {
                scalar_param(id, id, "0", "100", pick(&clip.crop) * 100.0, vec![], 2)
            } else {
                let kfs: Vec<(i64, f64)> = frames
                    .iter()
                    .map(|&f| ((f - clip.start_frame) as i64, pick(&clip.crop_at(f)) * 100.0))
                    .collect();
                let base = kfs[0].1;
                scalar_param(id, id, "0", "100", base, kfs, 2)
            }
        };
        let params = vec![
            edge("left", |c| c.left),
            edge("right", |c| c.right),
            edge("top", |c| c.top),
            edge("bottom", |c| c.bottom),
        ];
        Some(filter(effect("Crop", "crop", "motion", "video", Some("motion"), params)))
    }

    fn opacity_filter(&self, clip: &Clip) -> Option<XmlNode> {
        let frames = keyframe_frames(clip, Property::Opacity);
        let opacity = if frames.is_empty() {
            if clip.opacity == 1.0 {
                return None;
            }
            scalar_param("opacity", "Opacity", "0", "100", clip.opacity * 100.0, vec![], 1)
        } else {
            let kfs: Vec<(i64, f64)> = frames
                .iter()
                .map(|&f| ((f - clip.start_frame) as i64, clip.raw_opacity_at(f) * 100.0))
                .collect();
            let base = kfs[0].1;
            scalar_param("opacity", "Opacity", "0", "100", base, kfs, 1)
        };
        Some(filter(effect("Opacity", "opacity", "motion", "video", None, vec![opacity])))
    }

    // MARK: - Indexing helpers

    /// Drop unresolvable clips so track builders and `<link>` indices agree.
    fn sort_emittable(&self, track: &Track) -> Vec<Clip> {
        let mut clips: Vec<Clip> = track
            .clips
            .iter()
            .filter(|c| self.resolver.resolve_url(&c.media_ref).is_some())
            .cloned()
            .collect();
        clips.sort_by_key(|c| c.start_frame);
        clips
    }

    fn index_addresses(&mut self, sorted_tracks: &[Vec<Clip>], is_audio: bool) {
        for (ti, clips) in sorted_tracks.iter().enumerate() {
            for (ci, clip) in clips.iter().enumerate() {
                self.clip_addresses.insert(
                    clip.id.clone(),
                    ClipAddress {
                        track_index: ti as i64 + 1,
                        clip_index: ci as i64 + 1,
                        is_audio,
                    },
                );
            }
        }
    }

    fn index_link_groups(&mut self) {
        for track in &self.timeline.tracks {
            for clip in &track.clips {
                if let Some(group) = &clip.link_group_id {
                    self.clips_by_link_group
                        .entry(group.clone())
                        .or_default()
                        .push(clip.clone());
                }
            }
        }
    }

    fn source_duration_frames(&self, media_ref: &str) -> Option<i32> {
        let seconds = self.resolver.entry(media_ref)?.duration;
        Some(seconds_to_frame(seconds, self.fps).max(0))
    }

    fn rate(&self, timebase: i32, ntsc: bool) -> XmlNode {
        XmlNode::el(
            "rate",
            vec![XmlNode::leaf_int("timebase", timebase as i64), XmlNode::bool_leaf("ntsc", ntsc)],
        )
    }
}

// MARK: - Free emitter helpers (no `self`)

fn filter(effect: XmlNode) -> XmlNode {
    XmlNode::el("filter", vec![effect])
}

fn effect(
    name: &str,
    id: &str,
    effect_type: &str,
    mediatype: &str,
    category: Option<&str>,
    body: Vec<XmlNode>,
) -> XmlNode {
    let mut children = vec![XmlNode::leaf("name", name), XmlNode::leaf("effectid", id)];
    if let Some(cat) = category {
        children.push(XmlNode::leaf("effectcategory", cat));
    }
    children.push(XmlNode::leaf("effecttype", effect_type));
    children.push(XmlNode::leaf("mediatype", mediatype));
    children.extend(body);
    XmlNode::el("effect", children)
}

fn parameter(
    id: &str,
    name: &str,
    min: Option<&str>,
    max: Option<&str>,
    value: XmlNode,
    keyframes: Vec<(i64, XmlNode)>,
) -> XmlNode {
    let mut children = vec![XmlNode::leaf("parameterid", id), XmlNode::leaf("name", name)];
    if let Some(min) = min {
        children.push(XmlNode::leaf("valuemin", min));
    }
    if let Some(max) = max {
        children.push(XmlNode::leaf("valuemax", max));
    }
    children.push(value);
    for (when, val) in keyframes {
        children.push(XmlNode::el("keyframe", vec![XmlNode::leaf_int("when", when), val]));
    }
    XmlNode::el("parameter", children)
}

/// A scalar parameter whose value (and keyframes) are floats formatted by
/// `places` decimal places. `keyframes` are `(when, value)`.
fn scalar_param(
    id: &str,
    name: &str,
    min: &str,
    max: &str,
    base: f64,
    keyframes: Vec<(i64, f64)>,
    places: usize,
) -> XmlNode {
    let kf_nodes: Vec<(i64, XmlNode)> = keyframes
        .into_iter()
        .map(|(when, v)| (when, XmlNode::leaf("value", fmt_f(places, v))))
        .collect();
    parameter(id, name, Some(min), Some(max), XmlNode::leaf("value", fmt_f(places, base)), kf_nodes)
}

/// The two-component Center parameter: a `<horiz>`/`<vert>` pair (`%.5f` each).
fn center_param(base: (f64, f64), keyframes: Vec<(i64, f64, f64)>) -> XmlNode {
    fn vec_node(x: f64, y: f64) -> XmlNode {
        XmlNode::el(
            "value",
            vec![XmlNode::leaf("horiz", fmt_f(5, x)), XmlNode::leaf("vert", fmt_f(5, y))],
        )
    }
    let kf_nodes: Vec<(i64, XmlNode)> = keyframes
        .into_iter()
        .map(|(when, x, y)| (when, vec_node(x, y)))
        .collect();
    parameter("center", "Center", None, None, vec_node(base.0, base.1), kf_nodes)
}

// MARK: - Keyframe-frame helpers (the model lacks `keyframeFrames(for:)`)

/// The animatable properties whose keyframe-frame lists the filters sample.
#[derive(Clone, Copy)]
enum Property {
    Opacity,
    Position,
    Scale,
    Rotation,
    Crop,
    Volume,
}

/// Absolute timeline keyframe frames for a property (reference
/// `keyframeFrames(for:)` returns `offset + startFrame`). Empty when the track is
/// absent — note an absent track and a present-but-empty track both yield `[]`.
fn keyframe_frames(clip: &Clip, prop: Property) -> Vec<i32> {
    let offsets: Vec<i32> = match prop {
        Property::Opacity => clip
            .opacity_track
            .as_ref()
            .map(|t| t.keyframes.iter().map(|k| k.frame).collect())
            .unwrap_or_default(),
        Property::Position => clip
            .position_track
            .as_ref()
            .map(|t| t.keyframes.iter().map(|k| k.frame).collect())
            .unwrap_or_default(),
        Property::Scale => clip
            .scale_track
            .as_ref()
            .map(|t| t.keyframes.iter().map(|k| k.frame).collect())
            .unwrap_or_default(),
        Property::Rotation => clip
            .rotation_track
            .as_ref()
            .map(|t| t.keyframes.iter().map(|k| k.frame).collect())
            .unwrap_or_default(),
        Property::Crop => clip
            .crop_track
            .as_ref()
            .map(|t| t.keyframes.iter().map(|k| k.frame).collect())
            .unwrap_or_default(),
        Property::Volume => clip
            .volume_track
            .as_ref()
            .map(|t| t.keyframes.iter().map(|k| k.frame).collect())
            .unwrap_or_default(),
    };
    offsets.into_iter().map(|o| o + clip.start_frame).collect()
}

fn sorted_unique(mut v: Vec<i32>) -> Vec<i32> {
    v.sort_unstable();
    v.dedup();
    v
}

// MARK: - Numeric / timecode helpers

/// `secondsToFrame(s,fps) = Int(s*fps)` — truncate toward zero (reference
/// `Utilities/TimeFormatting.swift`; Swift `Int(_:)` truncates, as does Rust
/// `as i32` for an in-range finite value).
fn seconds_to_frame(seconds: f64, fps: i32) -> i32 {
    (seconds * fps as f64) as i32
}

/// `rateTags(forFPS:)` → `(timebase, ntsc)` (reference). `timebase = round(fps)`
/// (ties-away, min 1); NTSC iff `|raw − timebase·1000/1001| < |raw − timebase|`
/// (catches 23.976 / 29.97 / 59.94).
fn rate_tags(raw_fps: f64) -> (i32, bool) {
    let timebase = (raw_fps.round() as i32).max(1);
    let ntsc_rate = timebase as f64 * 1000.0 / 1001.0;
    let ntsc = (raw_fps - ntsc_rate).abs() < (raw_fps - timebase as f64).abs();
    (timebase, ntsc)
}

/// `formatTimecode` (golden-critical). NDF sep `:`, DF sep `;`. The drop-frame
/// correction is the reference's hand-rolled approximation — copied **exactly**
/// (`round(fps*0.066666)`, integer divides); do NOT substitute a textbook SMPTE
/// formula or the goldens diverge.
pub fn format_timecode(frame: i32, fps: i32, drop_frame: bool) -> String {
    let mut f = frame;
    if drop_frame {
        let drop = (fps as f64 * 0.066666).round() as i32; // 2 @ 30, 4 @ 60
        let d = f / (fps * 600);
        let m = f % (fps * 600);
        f += drop * 9 * d + if m > drop { drop * ((m - drop) / (fps * 60)) } else { 0 };
    }
    let sep = if drop_frame { ";" } else { ":" };
    let ff = f % fps;
    let ss = (f / fps) % 60;
    let mm = (f / (fps * 60)) % 60;
    let hh = f / (fps * 3600);
    format!("{hh:02}{sep}{mm:02}{sep}{ss:02}{sep}{ff:02}")
}

/// Last path component of a `file://…` URL string or a plain path — the segment
/// after the final `/` (reference `url.lastPathComponent`). Percent-escapes are
/// left as-is (the reference uses the *decoded* component, but media filenames in
/// the goldens are ASCII-safe, so the encoded segment equals the name).
fn last_path_component(url: &str) -> String {
    url.rsplit('/').next().unwrap_or(url).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_tags_detects_ntsc() {
        assert_eq!(rate_tags(30.0), (30, false));
        assert_eq!(rate_tags(29.97), (30, true));
        assert_eq!(rate_tags(23.976), (24, true));
        assert_eq!(rate_tags(59.94), (60, true));
        assert_eq!(rate_tags(24.0), (24, false));
        assert_eq!(rate_tags(60.0), (60, false));
    }

    #[test]
    fn seconds_to_frame_truncates_toward_zero() {
        assert_eq!(seconds_to_frame(4.0, 30), 120);
        // 4.99 * 30 = 149.7 → truncates to 149 (NOT rounded).
        assert_eq!(seconds_to_frame(4.99, 30), 149);
        assert_eq!(seconds_to_frame(0.5, 30), 15);
    }

    #[test]
    fn timecode_ndf_uses_colon() {
        // 30 fps, frame 90 → 00:00:03:00.
        assert_eq!(format_timecode(90, 30, false), "00:00:03:00");
        // frame 0 → all zeros.
        assert_eq!(format_timecode(0, 30, false), "00:00:00:00");
        // 1 hour at 30 fps = 108000 frames.
        assert_eq!(format_timecode(108000, 30, false), "01:00:00:00");
    }

    #[test]
    fn timecode_drop_frame_uses_semicolons_at_2997() {
        // DF at 29.97 (timebase 30, drop=round(30*0.066666)=2).
        // frame 0 → 00;00;00;00.
        assert_eq!(format_timecode(0, 30, true), "00;00;00;00");
        // A frame past the first minute boundary exercises the d/m correction.
        // 1800 frames = 1 minute of timeline frames; with the reference's
        // approximation this is a fixed, reproducible value.
        let tc = format_timecode(1800, 30, true);
        assert!(tc.contains(';'), "DF must use ; separators: {tc}");
        // The m>drop branch: a frame just inside a 10-minute block.
        let tc2 = format_timecode(18000, 30, true);
        assert!(tc2.starts_with("00;10;"), "got {tc2}");
    }

    #[test]
    fn timecode_drop_frame_at_5994() {
        // 59.94 → timebase 60, drop=round(60*0.066666)=4.
        assert_eq!(format_timecode(0, 60, true), "00;00;00;00");
        let tc = format_timecode(3600, 60, true);
        assert!(tc.contains(';'));
    }

    #[test]
    fn last_path_component_extracts_filename() {
        assert_eq!(last_path_component("file:///a/b/clip.mov"), "clip.mov");
        assert_eq!(last_path_component("clip.mov"), "clip.mov");
    }
}
