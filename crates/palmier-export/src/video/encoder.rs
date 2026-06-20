//! Hardware-encoder selection + fallback chain — E6-S5.
//!
//! The provisioned FFmpeg is the **LGPL** build (docs/windows-harness-notes.md):
//! it has **no libx264/libx265** (GPL), so software H.264/H.265 encode is
//! unavailable. H.264/H.265 must use a **hardware** encoder; ProRes uses the
//! always-present `prores_ks`.
//!
//! The **selection** is pure: given an availability predicate (`is this FFmpeg
//! encoder registered?`), [`select_encoder`] walks the per-format fallback chain
//! in priority order and returns the first available encoder, or
//! [`ExportError::NoHardwareEncoder`] naming the whole chain it tried. This keeps
//! the priority order unit-testable with a fake predicate (no GPU needed). The
//! real predicate ([`ffmpeg_encoder_available`], behind `gpu-export`) asks
//! `ffmpeg-next` whether `avcodec_find_encoder_by_name` resolves.
//!
//! ## Fallback order (priority high → low)
//!
//! Per the story: try **NVENC** first (the §10 reference GPU is an RTX 4060), then
//! **QSV** (Intel), then **AMF** (AMD), then **MediaFoundation** (the OS-level
//! Windows HW encoder, a last HW resort). ProRes is a single fixed encoder.

use super::spec::ExportFormat;
use super::ExportError;

/// The hardware-encoder vendor/path an [`EncoderPlan`] selected. Diagnostic only
/// — the FFmpeg encoder name in the plan is what actually drives the encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwVendor {
    /// NVIDIA NVENC (`*_nvenc`).
    Nvenc,
    /// Intel Quick Sync (`*_qsv`).
    Qsv,
    /// AMD AMF (`*_amf`).
    Amf,
    /// Windows Media Foundation (`*_mf`) — OS-level HW encode.
    MediaFoundation,
    /// ProRes via `prores_ks` — software but **LGPL-clean** (not a HW path; this
    /// variant marks the ProRes lane).
    ProResKs,
}

impl HwVendor {
    /// A human label for diagnostics / the export log.
    pub fn label(self) -> &'static str {
        match self {
            HwVendor::Nvenc => "NVENC (NVIDIA)",
            HwVendor::Qsv => "Quick Sync (Intel)",
            HwVendor::Amf => "AMF (AMD)",
            HwVendor::MediaFoundation => "Media Foundation (Windows)",
            HwVendor::ProResKs => "prores_ks (LGPL software)",
        }
    }
}

/// The chosen encoder: the FFmpeg encoder name + its vendor + the FFmpeg pixel
/// format the encoder wants its input frames in. The `render` module configures
/// the codec context with this and converts the readback RGBA → `pix_fmt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderPlan {
    /// The FFmpeg encoder short name (e.g. `"h264_nvenc"`, `"prores_ks"`).
    pub ffmpeg_name: &'static str,
    /// Which HW path / encoder this is.
    pub vendor: HwVendor,
    /// The FFmpeg pixel-format name the encoder ingests. HW H.264/H.265 take
    /// `nv12`; ProRes 422 takes `yuv422p10le` (10-bit 4:2:2, the ProRes 422
    /// pixel layout).
    pub pix_fmt: &'static str,
}

/// The H.264 fallback chain, priority high → low (NVENC → QSV → AMF → MF).
pub const ENCODER_FALLBACK_H264: &[(&str, HwVendor)] = &[
    ("h264_nvenc", HwVendor::Nvenc),
    ("h264_qsv", HwVendor::Qsv),
    ("h264_amf", HwVendor::Amf),
    ("h264_mf", HwVendor::MediaFoundation),
];

/// The H.265/HEVC fallback chain, priority high → low (NVENC → QSV → AMF → MF).
pub const ENCODER_FALLBACK_H265: &[(&str, HwVendor)] = &[
    ("hevc_nvenc", HwVendor::Nvenc),
    ("hevc_qsv", HwVendor::Qsv),
    ("hevc_amf", HwVendor::Amf),
    ("hevc_mf", HwVendor::MediaFoundation),
];

/// The fixed ProRes 422 encoder (LGPL-clean, always available in the LGPL build).
pub const PRORES_KS: (&str, HwVendor) = ("prores_ks", HwVendor::ProResKs);

/// The fallback chain for `format`.
fn chain(format: ExportFormat) -> &'static [(&'static str, HwVendor)] {
    match format {
        ExportFormat::H264 => ENCODER_FALLBACK_H264,
        ExportFormat::H265 => ENCODER_FALLBACK_H265,
        // ProRes is a single fixed encoder; return a one-element static slice.
        ExportFormat::ProRes422 => std::slice::from_ref(&PRORES_KS),
    }
}

/// The encoder input pixel format for `vendor`. HW H.264/H.265 ingest `nv12`
/// (the universal HW-encoder input); ProRes 422 ingests 10-bit 4:2:2.
fn pix_fmt_for(vendor: HwVendor) -> &'static str {
    match vendor {
        HwVendor::ProResKs => "yuv422p10le",
        _ => "nv12",
    }
}

/// Choose the encoder for `format`, walking its fallback chain in priority order
/// and returning the first one `available(name)` reports present.
///
/// `available` is the availability predicate — in production it asks FFmpeg
/// whether the encoder is registered ([`ffmpeg_encoder_available`]); tests pass a
/// fake. ProRes always resolves (its `prores_ks` is in the LGPL build), so the
/// only `NoHardwareEncoder` path is H.264/H.265 with **no** HW encoder present.
///
/// Returns [`ExportError::NoHardwareEncoder`] (naming the full chain it probed)
/// when none of `format`'s encoders is available — never a silent fall-through.
pub fn select_encoder(
    format: ExportFormat,
    available: impl Fn(&str) -> bool,
) -> Result<EncoderPlan, ExportError> {
    // ProRes is the LGPL-guaranteed lane: `prores_ks` is always in the LGPL
    // build, so it resolves regardless of the (HW-encoder-oriented) predicate.
    if format == ExportFormat::ProRes422 {
        let (name, vendor) = PRORES_KS;
        return Ok(EncoderPlan {
            ffmpeg_name: name,
            vendor,
            pix_fmt: pix_fmt_for(vendor),
        });
    }

    let chain = chain(format);
    for &(name, vendor) in chain {
        if available(name) {
            return Ok(EncoderPlan {
                ffmpeg_name: name,
                vendor,
                pix_fmt: pix_fmt_for(vendor),
            });
        }
    }
    Err(ExportError::NoHardwareEncoder {
        codec: format.label(),
        tried: chain.iter().map(|(n, _)| *n).collect(),
    })
}

/// Ask FFmpeg whether an encoder is registered by name (the production
/// availability predicate for [`select_encoder`]). Behind `gpu-export` because it
/// links `ffmpeg-next`.
#[cfg(feature = "gpu-export")]
pub fn ffmpeg_encoder_available(name: &str) -> bool {
    ffmpeg_next::encoder::find_by_name(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prores_always_resolves_to_prores_ks() {
        // Even with nothing else available, ProRes selects prores_ks.
        let plan = select_encoder(ExportFormat::ProRes422, |_| false).unwrap();
        assert_eq!(plan.ffmpeg_name, "prores_ks");
        assert_eq!(plan.vendor, HwVendor::ProResKs);
        assert_eq!(plan.pix_fmt, "yuv422p10le");
    }

    #[test]
    fn h264_prefers_nvenc_when_all_available() {
        let plan = select_encoder(ExportFormat::H264, |_| true).unwrap();
        assert_eq!(plan.ffmpeg_name, "h264_nvenc");
        assert_eq!(plan.vendor, HwVendor::Nvenc);
        assert_eq!(plan.pix_fmt, "nv12");
    }

    #[test]
    fn h264_falls_back_in_priority_order() {
        // No NVENC → QSV.
        let plan = select_encoder(ExportFormat::H264, |n| n != "h264_nvenc").unwrap();
        assert_eq!(plan.ffmpeg_name, "h264_qsv");
        // No NVENC, no QSV → AMF.
        let plan =
            select_encoder(ExportFormat::H264, |n| n == "h264_amf" || n == "h264_mf").unwrap();
        assert_eq!(plan.ffmpeg_name, "h264_amf");
        // Only MediaFoundation → MF.
        let plan = select_encoder(ExportFormat::H264, |n| n == "h264_mf").unwrap();
        assert_eq!(plan.ffmpeg_name, "h264_mf");
        assert_eq!(plan.vendor, HwVendor::MediaFoundation);
    }

    #[test]
    fn h265_prefers_nvenc_then_falls_back() {
        let plan = select_encoder(ExportFormat::H265, |_| true).unwrap();
        assert_eq!(plan.ffmpeg_name, "hevc_nvenc");
        let plan = select_encoder(ExportFormat::H265, |n| n == "hevc_qsv").unwrap();
        assert_eq!(plan.ffmpeg_name, "hevc_qsv");
    }

    #[test]
    fn no_hw_encoder_errors_with_full_chain() {
        // No HW encoder at all → H.264 errors, naming every probed encoder.
        let err = select_encoder(ExportFormat::H264, |_| false).unwrap_err();
        match err {
            ExportError::NoHardwareEncoder { codec, tried } => {
                assert_eq!(codec, "H.264");
                assert_eq!(
                    tried,
                    vec!["h264_nvenc", "h264_qsv", "h264_amf", "h264_mf"]
                );
            }
            other => panic!("expected NoHardwareEncoder, got {other:?}"),
        }
        // H.265 likewise.
        let err = select_encoder(ExportFormat::H265, |_| false).unwrap_err();
        match err {
            ExportError::NoHardwareEncoder { codec, tried } => {
                assert_eq!(codec, "H.265");
                assert_eq!(
                    tried,
                    vec!["hevc_nvenc", "hevc_qsv", "hevc_amf", "hevc_mf"]
                );
            }
            other => panic!("expected NoHardwareEncoder, got {other:?}"),
        }
    }

    #[test]
    fn error_message_mentions_lgpl_and_prores_fallback() {
        let err = select_encoder(ExportFormat::H264, |_| false).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("LGPL"), "explains the LGPL constraint: {msg}");
        assert!(msg.contains("ProRes"), "suggests the ProRes fallback: {msg}");
        assert!(msg.contains("h264_nvenc"), "names the probed chain: {msg}");
    }
}
