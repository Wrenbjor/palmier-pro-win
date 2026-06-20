//! The port's download manifest — a Windows/Linux analogue of the reference's
//! `SearchIndexConfig.manifest` (`ModelDownloader.Manifest`), but for ONNX files.
//!
//! Ported from Spike S-3 (`spikes/s3-siglip2/src/manifest.rs`).
//!
//! The reference manifest names CoreML zips with their SHA256/bytes and a hosted
//! base URL (palmier-io/siglip2-base-coreml). The port can NOT reuse those files
//! (CoreML-only); ruling #13 (`docs/phase0-reconciliation.md`) says "source
//! ONNX/candle weights + publish a new manifest (different SHA256/sizes)". This
//! struct is that new manifest shape.
//!
//! Recommended source: `onnx-community/siglip2-base-patch16-256-ONNX`. For an offline
//! app we would either (a) re-host the chosen ONNX files under a palmier-io repo (as
//! the reference does for CoreML), or (b) pull directly from onnx-community at the
//! pinned revision. The SHA256/bytes below are PLACEHOLDERS to be filled from the
//! actual chosen files (the fp16 or fp32 variants — see the S-3 FINDINGS size table)
//! at hosting time; they are computed via `sha2` over the downloaded file, same as
//! the reference's `ModelDownloader.verify`. E11-S6 fleshes out the downloader.

use serde::{Deserialize, Serialize};

/// onnx-community base URL the proposed default manifest pulls from. E11-S6 may
/// repoint this to a re-hosted palmier-io mirror (recommended for v1).
pub const ONNX_COMMUNITY_BASE_URL: &str =
    "https://huggingface.co/onnx-community/siglip2-base-patch16-256-ONNX/resolve/main";

/// Placeholder marker for a not-yet-computed SHA256 (filled at hosting time).
pub const SHA_PLACEHOLDER: &str = "<fill-from-download>";

/// One downloadable file with integrity metadata (mirrors `Manifest.File`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFile {
    pub name: String,
    pub sha256: String,
    pub bytes: u64,
}

/// The ONNX model manifest (mirrors `ModelDownloader.Manifest`, ONNX variant).
///
/// Field constants agree with [`crate::spec`] by construction (see
/// [`OnnxManifest::proposed_default`]) — the spec is the single source of truth; this
/// manifest is its on-the-wire serialization, not a divergent duplicate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnnxManifest {
    pub model: String,
    pub version: i64,
    #[serde(rename = "embeddingDim")]
    pub embedding_dim: usize,
    #[serde(rename = "imageSize")]
    pub image_size: usize,
    #[serde(rename = "contextLength")]
    pub context_length: usize,
    /// onnx-community revision or a re-host base URL (parity with `hostedURL`).
    #[serde(rename = "baseURL")]
    pub base_url: String,
    pub files: OnnxFiles,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnnxFiles {
    #[serde(rename = "visionModel")]
    pub vision_model: ManifestFile,
    #[serde(rename = "textModel")]
    pub text_model: ManifestFile,
    pub tokenizer: ManifestFile,
}

impl OnnxManifest {
    /// The proposed default manifest for the port. SHA256/bytes are PLACEHOLDERS
    /// ([`SHA_PLACEHOLDER`]) — the orchestrator/E11-S6 fills them after choosing the
    /// fp16 vs fp32 variant and computing the hash, exactly like the reference did
    /// for its CoreML zips. Sizes shown in the S-3 FINDINGS table. The constants
    /// (`model`, `version`, `embeddingDim`, `imageSize`, `contextLength`) come from
    /// [`crate::spec`] so they cannot drift from the `.embed` header contract.
    pub fn proposed_default() -> Self {
        Self {
            model: crate::spec::MODEL.into(),
            version: crate::spec::MODEL_VERSION,
            embedding_dim: crate::spec::EMBEDDING_DIM,
            image_size: crate::spec::IMAGE_SIZE,
            context_length: crate::spec::CONTEXT_LENGTH,
            base_url: ONNX_COMMUNITY_BASE_URL.into(),
            files: OnnxFiles {
                // fp16 variant recommended for GPU (DirectML); fp32 for max parity.
                vision_model: ManifestFile {
                    name: "onnx/vision_model_fp16.onnx".into(),
                    sha256: SHA_PLACEHOLDER.into(),
                    bytes: 0,
                },
                text_model: ManifestFile {
                    name: "onnx/text_model_fp16.onnx".into(),
                    sha256: SHA_PLACEHOLDER.into(),
                    bytes: 0,
                },
                tokenizer: ManifestFile {
                    name: "tokenizer.json".into(),
                    sha256: SHA_PLACEHOLDER.into(),
                    bytes: 0,
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_serializes_with_camelcase_keys() {
        let m = OnnxManifest::proposed_default();
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("\"embeddingDim\":768"));
        assert!(s.contains("\"imageSize\":256"));
        assert!(s.contains("\"contextLength\":64"));
        assert!(s.contains("\"visionModel\""));
        assert!(s.contains("\"textModel\""));
        // round-trip
        let back: OnnxManifest = serde_json::from_str(&s).unwrap();
        assert_eq!(back.embedding_dim, 768);
        assert_eq!(back, m);
    }

    #[test]
    fn manifest_constants_agree_with_spec() {
        let m = OnnxManifest::proposed_default();
        assert_eq!(m.model, crate::spec::MODEL);
        assert_eq!(m.version, crate::spec::MODEL_VERSION);
        assert_eq!(m.embedding_dim, crate::spec::EMBEDDING_DIM);
        assert_eq!(m.image_size, crate::spec::IMAGE_SIZE);
        assert_eq!(m.context_length, crate::spec::CONTEXT_LENGTH);
        // modelVersion is 2 on the port (S-3 ruling: force a clean re-index).
        assert_eq!(m.version, 2);
    }
}
