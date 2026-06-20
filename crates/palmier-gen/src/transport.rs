//! The Convex transport seam — wire types + the [`GenerationTransport`] trait
//! (E9-S1; the S-2 spike slice).
//!
//! The macOS reference talks to Convex through the native `ConvexMobile` SDK
//! (`GenerationBackend.swift`): a reactive `subscribe("generations:byId")`, the
//! 3-step `uploads:*` flow, and `generations:submit`. The port re-expresses that
//! surface behind ONE trait so the whole generation lifecycle
//! ([`crate::service`]) is transport-agnostic and **unit-testable without a live
//! Convex** (the [`MockTransport`] drives every test).
//!
//! Two real impls live behind the trait (Spike S-2 / FINDINGS §3.1):
//! - **WS (primary)** — the official `convex` crate over the WebSocket sync
//!   protocol (`crate::convex_ws`, behind the `convex-transport` feature).
//! - **HTTP polling (fallback)** — [`HttpPollingTransport`], for the
//!   WS-blocked-by-proxy edge.
//!
//! Selecting WS vs polling is a single type substitution — the lifecycle never
//! changes. Cancellation (#24) is **client teardown only**: dropping the job
//! subscription (the [`JobStream`]) unsubscribes; the Convex job keeps
//! running/billing.

use std::pin::Pin;

use serde::Deserialize;

use crate::catalog::CatalogEntry;

/// Generation job status. Wire values are lowercase (`queued|running|succeeded|
/// failed`) — byte-identical to the reference `BackendGenerationStatus` raw
/// values. A casing/field drift silently breaks the whole subscription, so the
/// decode is pinned by [`crate::transport::tests`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendGenerationStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
}

impl BackendGenerationStatus {
    /// A job is settled (the subscription loop can stop) once it succeeds or
    /// fails. `queued`/`running` are the "keep listening" states.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

/// One generation job, as pushed by the `generations:byId` live query
/// (reference `BackendGenerationJob`). Field names match the Convex document
/// shape (`_id`, camelCase rest). `result_urls` maps 1:1 by index onto the
/// placeholder `MediaAsset`s on success. `completed_at` is an Apple-reference-
/// epoch double on the wire (decoded lazily — the lifecycle never needs it).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendGenerationJob {
    #[serde(rename = "_id")]
    pub id: String,
    pub status: BackendGenerationStatus,
    #[serde(default)]
    pub result_urls: Option<Vec<String>>,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub cost_credits: Option<i64>,
    #[serde(default)]
    pub completed_at: Option<f64>,
}

/// A typed generation error (reference `GenerationBackendError`). `Api` carries
/// the decoded `BackendErrorEnvelope{error:{code,message}}`; `NotConfigured`
/// stands in for "no Convex client" (the advisory/back-end-unavailable path);
/// `Transport` is everything else (connect/IO/decode).
#[derive(Debug, Clone)]
pub enum GenerationError {
    /// No Convex backend configured (signed out / no deployment URL).
    NotConfigured,
    /// A network/transport/decode failure with a human-readable message.
    Transport(String),
    /// A backend API error with the decoded envelope fields.
    Api { status: u16, code: String, message: String },
}

impl std::fmt::Display for GenerationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GenerationError::NotConfigured => write!(f, "Palmier backend not configured."),
            GenerationError::Transport(s) => write!(f, "{s}"),
            GenerationError::Api { message, .. } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for GenerationError {}

/// The decoded backend error envelope (reference `BackendErrorEnvelope`).
#[derive(Debug, Clone, Deserialize)]
pub struct BackendErrorEnvelope {
    pub error: BackendErrorInner,
}

/// The inner `{code, message}` of [`BackendErrorEnvelope`].
#[derive(Debug, Clone, Deserialize)]
pub struct BackendErrorInner {
    pub code: String,
    pub message: String,
}

impl BackendErrorEnvelope {
    /// Try to decode a non-2xx HTTP body into a typed [`GenerationError::Api`].
    /// Falls back to [`GenerationError::Transport`] when the body is not the
    /// envelope shape (reference `assertHTTPOK`).
    #[must_use]
    pub fn to_error(status: u16, body: &str) -> GenerationError {
        match serde_json::from_str::<BackendErrorEnvelope>(body) {
            Ok(env) => GenerationError::Api {
                status,
                code: env.error.code,
                message: env.error.message,
            },
            Err(_) => GenerationError::Transport(format!("HTTP {status}: {body}")),
        }
    }
}

/// A live stream of job pushes. Each `next()` yields the latest
/// [`BackendGenerationJob`] (a Convex `null` push is filtered out by the
/// transport). **Dropping this stream is the #24 client-teardown cancel**: the
/// WS impl emits a `ModifyQuerySet::Remove`; the polling impl stops its loop.
pub type JobStream = Pin<Box<dyn futures::Stream<Item = BackendGenerationJob> + Send>>;

/// The Convex surface the generation lifecycle codes against. One trait, two
/// real impls (WS + polling) plus the test [`MockTransport`]. All methods are
/// `&self` so a transport is cheaply shareable (`Arc`); the convex client is
/// internally `Clone` over one socket.
#[async_trait::async_trait]
pub trait GenerationTransport: Send + Sync {
    /// Load the model catalog (`models:list` / `/v1/models`). A 24h snapshot is
    /// acceptable (FOUNDATION §6.1) — the impl may also keep a live subscription.
    async fn load_models(&self) -> Result<Vec<CatalogEntry>, GenerationError>;

    /// Mint a 3-step upload ticket (`uploads:generateUploadTicket`) → the staging
    /// `uploadUrl`.
    async fn generate_upload_ticket(&self) -> Result<String, GenerationError>;

    /// Commit a staged upload (`uploads:commitUpload`) → the hosted `url`.
    async fn commit_upload(&self, storage_id: &str) -> Result<String, GenerationError>;

    /// Submit a generation job (`generations:submit`) → the `jobId`. `params` is
    /// the byte-faithful params JSON ([`crate::params`]); `model`/`project_id`
    /// are the sibling args.
    async fn submit(
        &self,
        model: &str,
        params: &serde_json::Value,
        project_id: Option<&str>,
    ) -> Result<String, GenerationError>;

    /// Subscribe to `generations:byId(jobId)` → a [`JobStream`]. Dropping the
    /// returned stream tears down the subscription (#24 cancel).
    async fn subscribe(&self, job_id: &str) -> Result<JobStream, GenerationError>;
}

// ─────────────────────────────────────────────────────────────────────────────
// Reference upload POST (3-step middle leg) — shared by every transport.
// ─────────────────────────────────────────────────────────────────────────────

/// POST raw bytes to a Convex staging URL with the given `Content-Type` and
/// decode `{storageId}` (the reference 3-step upload, leg b). Shared by all
/// transports because the staging URL is a plain HTTPS endpoint, not a Convex
/// function. A non-2xx body is decoded into a typed [`GenerationError`].
pub async fn post_upload_bytes(
    client: &reqwest::Client,
    upload_url: &str,
    content_type: &str,
    bytes: Vec<u8>,
) -> Result<String, GenerationError> {
    let resp = client
        .post(upload_url)
        .header(reqwest::header::CONTENT_TYPE, content_type)
        .body(bytes)
        .send()
        .await
        .map_err(|e| GenerationError::Transport(e.to_string()))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| GenerationError::Transport(e.to_string()))?;
    if !status.is_success() {
        return Err(BackendErrorEnvelope::to_error(status.as_u16(), &text));
    }
    #[derive(Deserialize)]
    struct StagingUploadResponse {
        #[serde(rename = "storageId")]
        storage_id: String,
    }
    serde_json::from_str::<StagingUploadResponse>(&text)
        .map(|r| r.storage_id)
        .map_err(|e| GenerationError::Transport(format!("decode storageId: {e}")))
}

// ─────────────────────────────────────────────────────────────────────────────
// MockTransport — the test double the whole lifecycle suite drives.
// ─────────────────────────────────────────────────────────────────────────────

/// A scripted [`GenerationTransport`] for unit tests (no live Convex). Construct
/// with [`MockTransport::builder`], script the catalog + the job-status sequence
/// + result URLs, and drive the full submit → subscribe → finalize lifecycle
/// deterministically.
#[cfg(any(test, feature = "test-mock"))]
pub use mock::MockTransport;

#[cfg(any(test, feature = "test-mock"))]
mod mock {
    use super::*;
    use std::sync::Mutex;

    /// A scripted transport. Records the submitted (model, params) and replays a
    /// fixed status sequence then result URLs.
    pub struct MockTransport {
        catalog: Vec<CatalogEntry>,
        statuses: Vec<BackendGenerationStatus>,
        result_urls: Vec<String>,
        error_message: Option<String>,
        not_configured: bool,
        uploaded: Mutex<Vec<(String, Vec<u8>)>>,
        submitted: Mutex<Vec<(String, serde_json::Value, Option<String>)>>,
    }

    impl MockTransport {
        /// A builder seeded with an empty catalog and a succeed-immediately job.
        #[must_use]
        pub fn builder() -> MockTransportBuilder {
            MockTransportBuilder {
                catalog: Vec::new(),
                statuses: vec![BackendGenerationStatus::Succeeded],
                result_urls: vec!["https://cdn.example/out0.mp4".to_string()],
                error_message: None,
                not_configured: false,
            }
        }

        /// The (model, params, project) tuples submitted so far.
        #[must_use]
        pub fn submitted(&self) -> Vec<(String, serde_json::Value, Option<String>)> {
            self.submitted.lock().unwrap().clone()
        }

        /// The (content_type, bytes) uploads recorded so far.
        #[must_use]
        pub fn uploads(&self) -> Vec<(String, Vec<u8>)> {
            self.uploaded.lock().unwrap().clone()
        }
    }

    /// Builder for [`MockTransport`].
    pub struct MockTransportBuilder {
        catalog: Vec<CatalogEntry>,
        statuses: Vec<BackendGenerationStatus>,
        result_urls: Vec<String>,
        error_message: Option<String>,
        not_configured: bool,
    }

    impl MockTransportBuilder {
        /// Set the catalog returned by `load_models`.
        #[must_use]
        pub fn catalog(mut self, catalog: Vec<CatalogEntry>) -> Self {
            self.catalog = catalog;
            self
        }

        /// Script the status sequence the job stream yields (last one should be
        /// terminal).
        #[must_use]
        pub fn statuses(mut self, statuses: Vec<BackendGenerationStatus>) -> Self {
            self.statuses = statuses;
            self
        }

        /// Set the result URLs delivered on a succeeded job.
        #[must_use]
        pub fn result_urls(mut self, urls: Vec<String>) -> Self {
            self.result_urls = urls;
            self
        }

        /// Script a failed terminal job with this message.
        #[must_use]
        pub fn fail_with(mut self, message: impl Into<String>) -> Self {
            self.statuses = vec![BackendGenerationStatus::Failed];
            self.error_message = Some(message.into());
            self
        }

        /// Make every call return [`GenerationError::NotConfigured`] (the
        /// signed-out / backend-unavailable path).
        #[must_use]
        pub fn not_configured(mut self) -> Self {
            self.not_configured = true;
            self
        }

        /// Build the transport.
        #[must_use]
        pub fn build(self) -> MockTransport {
            MockTransport {
                catalog: self.catalog,
                statuses: self.statuses,
                result_urls: self.result_urls,
                error_message: self.error_message,
                not_configured: self.not_configured,
                uploaded: Mutex::new(Vec::new()),
                submitted: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl GenerationTransport for MockTransport {
        async fn load_models(&self) -> Result<Vec<CatalogEntry>, GenerationError> {
            if self.not_configured {
                return Err(GenerationError::NotConfigured);
            }
            Ok(self.catalog.clone())
        }

        async fn generate_upload_ticket(&self) -> Result<String, GenerationError> {
            if self.not_configured {
                return Err(GenerationError::NotConfigured);
            }
            Ok("https://staging.example/upload/ticket".to_string())
        }

        async fn commit_upload(&self, storage_id: &str) -> Result<String, GenerationError> {
            if self.not_configured {
                return Err(GenerationError::NotConfigured);
            }
            Ok(format!("https://cdn.example/ref/{storage_id}"))
        }

        async fn submit(
            &self,
            model: &str,
            params: &serde_json::Value,
            project_id: Option<&str>,
        ) -> Result<String, GenerationError> {
            if self.not_configured {
                return Err(GenerationError::NotConfigured);
            }
            self.submitted.lock().unwrap().push((
                model.to_string(),
                params.clone(),
                project_id.map(str::to_string),
            ));
            Ok("job_mock_0001".to_string())
        }

        async fn subscribe(&self, job_id: &str) -> Result<JobStream, GenerationError> {
            if self.not_configured {
                return Err(GenerationError::NotConfigured);
            }
            let id = job_id.to_string();
            let result_urls = self.result_urls.clone();
            let error_message = self.error_message.clone();
            let jobs: Vec<BackendGenerationJob> = self
                .statuses
                .iter()
                .map(|&status| BackendGenerationJob {
                    id: id.clone(),
                    status,
                    result_urls: if status == BackendGenerationStatus::Succeeded {
                        Some(result_urls.clone())
                    } else {
                        None
                    },
                    error_message: if status == BackendGenerationStatus::Failed {
                        error_message.clone()
                    } else {
                        None
                    },
                    cost_credits: None,
                    completed_at: None,
                })
                .collect();
            Ok(Box::pin(futures::stream::iter(jobs)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_terminal_classification() {
        assert!(!BackendGenerationStatus::Queued.is_terminal());
        assert!(!BackendGenerationStatus::Running.is_terminal());
        assert!(BackendGenerationStatus::Succeeded.is_terminal());
        assert!(BackendGenerationStatus::Failed.is_terminal());
    }

    #[test]
    fn status_decodes_lowercase_wire_values() {
        let s: BackendGenerationStatus = serde_json::from_str("\"running\"").unwrap();
        assert_eq!(s, BackendGenerationStatus::Running);
        let s: BackendGenerationStatus = serde_json::from_str("\"succeeded\"").unwrap();
        assert_eq!(s, BackendGenerationStatus::Succeeded);
    }

    #[test]
    fn job_decodes_reference_document_shape() {
        let json = serde_json::json!({
            "_id": "job_abc123",
            "status": "succeeded",
            "resultUrls": ["https://cdn.example/out0.mp4"],
            "errorMessage": null,
            "costCredits": 42,
            "completedAt": 7.258464e8_f64
        });
        let job: BackendGenerationJob = serde_json::from_value(json).unwrap();
        assert_eq!(job.id, "job_abc123");
        assert_eq!(job.status, BackendGenerationStatus::Succeeded);
        assert_eq!(
            job.result_urls.as_deref(),
            Some(&["https://cdn.example/out0.mp4".to_string()][..])
        );
        assert_eq!(job.cost_credits, Some(42));
        assert!(job.completed_at.is_some());
    }

    #[test]
    fn job_decodes_in_flight_minimal_shape() {
        let json = serde_json::json!({ "_id": "job_x", "status": "queued" });
        let job: BackendGenerationJob = serde_json::from_value(json).unwrap();
        assert_eq!(job.status, BackendGenerationStatus::Queued);
        assert!(job.result_urls.is_none());
        assert!(!job.status.is_terminal());
    }

    #[test]
    fn error_envelope_decodes_to_typed_api_error() {
        let body = r#"{"error":{"code":"insufficient_credits","message":"Out of credits"}}"#;
        match BackendErrorEnvelope::to_error(402, body) {
            GenerationError::Api { status, code, message } => {
                assert_eq!(status, 402);
                assert_eq!(code, "insufficient_credits");
                assert_eq!(message, "Out of credits");
            }
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[test]
    fn non_envelope_body_falls_back_to_transport_error() {
        match BackendErrorEnvelope::to_error(500, "internal error") {
            GenerationError::Transport(msg) => assert!(msg.contains("500")),
            other => panic!("expected Transport error, got {other:?}"),
        }
    }
}
