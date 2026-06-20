//! 3-step Convex reference upload + the upload cache (E9-S6; reference
//! `GenerationService.uploadReferences` / `GenerationBackend.uploadReference`).
//!
//! Exact 3-step contract (reference):
//! 1. `uploads:generateUploadTicket` → `{uploadUrl}`
//! 2. HTTP **POST** the bytes to that URL with the correct **`Content-Type`**
//!    (mapped from the file extension / clip type), assert 2xx, decode
//!    `{storageId}`.
//! 3. `uploads:commitUpload {storageId}` → `{url}` (the hosted URL).
//!
//! The **Content-Type map must match the reference** — the backend may key on
//! it. `upload_references` runs uploads concurrently, reuses the per-asset cache
//! when fresh, and **re-sorts results back into input order** (index ordering is
//! load-bearing). The cache TTL is **6 days** and is recorded only for pristine
//! (non-trimmed/non-preprocessed) assets.

use std::time::Duration;

use palmier_model::ClipType;

use crate::transport::{post_upload_bytes, GenerationError, GenerationTransport};

/// Upload-cache TTL: 6 days (reference `6 * 24 * 60 * 60`).
pub const UPLOAD_CACHE_TTL: Duration = Duration::from_secs(6 * 24 * 60 * 60);

/// Map a file extension / clip type to its `Content-Type` (reference
/// `GenerationService.contentType(for:fallback:)`). The map is byte-exact with
/// the reference; the backend may reject a mismatch.
#[must_use]
pub fn content_type_for(extension: &str, fallback: ClipType) -> &'static str {
    match extension.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "gif" => "image/gif",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        _ => match fallback {
            ClipType::Image => "image/jpeg",
            ClipType::Video => "video/mp4",
            ClipType::Audio => "audio/mpeg",
            ClipType::Text => "application/octet-stream",
            ClipType::Lottie => "application/json",
        },
    }
}

/// Extract the lowercase extension from a path-like string.
#[must_use]
pub fn extension_of(path: &str) -> &str {
    path.rsplit('.').next().filter(|e| *e != path).unwrap_or("")
}

/// One reference to upload: its on-disk bytes path, its clip type, and an
/// optional cache slot (only set for pristine assets).
pub struct ReferenceUpload {
    /// The local file path (used only to derive the extension/Content-Type).
    pub path: String,
    /// The raw bytes to POST (already trimmed/preprocessed if applicable).
    pub bytes: Vec<u8>,
    pub clip_type: ClipType,
    /// A fresh cached remote URL to reuse instead of uploading (cache hit).
    pub cached_url: Option<String>,
    /// Whether to record the result in the cache (pristine, non-trimmed only).
    pub cacheable: bool,
}

/// The result of one reference upload: the hosted URL + whether it should be
/// cached (so the caller can stamp the asset's `cachedRemoteURL[ExpiresAt]`).
pub struct UploadResult {
    pub url: String,
    pub cacheable: bool,
}

/// Upload one reference via the exact 3-step contract (reference
/// `uploadReference`). Returns the hosted URL.
pub async fn upload_one(
    transport: &dyn GenerationTransport,
    http: &reqwest::Client,
    path: &str,
    bytes: Vec<u8>,
    clip_type: ClipType,
) -> Result<String, GenerationError> {
    // 1. ticket
    let upload_url = transport.generate_upload_ticket().await?;
    // 2. POST bytes with the mapped Content-Type, decode storageId
    let content_type = content_type_for(extension_of(path), clip_type);
    let storage_id = post_upload_bytes(http, &upload_url, content_type, bytes).await?;
    // 3. commit
    transport.commit_upload(&storage_id).await
}

/// Upload every reference concurrently, reusing fresh cache hits, and return the
/// hosted URLs **in input order** with the per-index cacheable flag (reference
/// `uploadReferences`). An empty input yields an empty vec.
pub async fn upload_references(
    transport: &dyn GenerationTransport,
    http: &reqwest::Client,
    refs: Vec<ReferenceUpload>,
) -> Result<Vec<UploadResult>, GenerationError> {
    if refs.is_empty() {
        return Ok(Vec::new());
    }
    let mut futs = Vec::with_capacity(refs.len());
    for (i, r) in refs.into_iter().enumerate() {
        let transport = transport;
        let http = http;
        futs.push(async move {
            if let Some(hit) = r.cached_url {
                // Cache hit — no upload, never re-cache.
                return Ok::<_, GenerationError>((i, UploadResult { url: hit, cacheable: false }));
            }
            let url = upload_one(transport, http, &r.path, r.bytes, r.clip_type).await?;
            Ok((i, UploadResult { url, cacheable: r.cacheable }))
        });
    }
    // Run concurrently, then re-sort by the original index (load-bearing order).
    let mut indexed = futures::future::try_join_all(futs).await?;
    indexed.sort_by_key(|(i, _)| *i);
    Ok(indexed.into_iter().map(|(_, r)| r).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::MockTransport;

    #[test]
    fn content_type_map_matches_reference() {
        assert_eq!(content_type_for("jpg", ClipType::Image), "image/jpeg");
        assert_eq!(content_type_for("JPEG", ClipType::Image), "image/jpeg");
        assert_eq!(content_type_for("png", ClipType::Image), "image/png");
        assert_eq!(content_type_for("webp", ClipType::Image), "image/webp");
        assert_eq!(content_type_for("heic", ClipType::Image), "image/heic");
        assert_eq!(content_type_for("gif", ClipType::Image), "image/gif");
        assert_eq!(content_type_for("mp4", ClipType::Video), "video/mp4");
        assert_eq!(content_type_for("m4v", ClipType::Video), "video/mp4");
        assert_eq!(content_type_for("mov", ClipType::Video), "video/quicktime");
        assert_eq!(content_type_for("mp3", ClipType::Audio), "audio/mpeg");
        assert_eq!(content_type_for("wav", ClipType::Audio), "audio/wav");
        assert_eq!(content_type_for("m4a", ClipType::Audio), "audio/mp4");
        // unknown extension falls back to the clip type's default.
        assert_eq!(content_type_for("xyz", ClipType::Image), "image/jpeg");
        assert_eq!(content_type_for("", ClipType::Video), "video/mp4");
        assert_eq!(content_type_for("", ClipType::Audio), "audio/mpeg");
    }

    #[test]
    fn extension_extraction() {
        assert_eq!(extension_of("a/b/clip.MP4"), "MP4");
        assert_eq!(extension_of("noext"), "");
        assert_eq!(extension_of("/tmp/x.png"), "png");
    }

    #[tokio::test]
    async fn cache_hit_skips_upload_and_preserves_order() {
        let transport = MockTransport::builder().build();
        let http = reqwest::Client::new();
        let refs = vec![
            ReferenceUpload {
                path: "a.png".into(),
                bytes: vec![],
                clip_type: ClipType::Image,
                cached_url: Some("https://cdn/cached0".into()),
                cacheable: false,
            },
            ReferenceUpload {
                path: "b.png".into(),
                bytes: vec![],
                clip_type: ClipType::Image,
                cached_url: Some("https://cdn/cached1".into()),
                cacheable: false,
            },
        ];
        let out = upload_references(&transport, &http, refs).await.unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].url, "https://cdn/cached0");
        assert_eq!(out[1].url, "https://cdn/cached1");
        // No real upload happened (both cache hits) so the mock recorded none.
        assert!(transport.uploads().is_empty());
    }

    #[tokio::test]
    async fn empty_refs_yield_empty() {
        let transport = MockTransport::builder().build();
        let http = reqwest::Client::new();
        let out = upload_references(&transport, &http, vec![]).await.unwrap();
        assert!(out.is_empty());
    }
}
