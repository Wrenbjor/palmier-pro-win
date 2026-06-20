//! Spike S-2 — Convex WS live-query: shared wire types + transport contract.
//!
//! This library crate holds the pieces E9 (`palmier-gen`) will lift, independent
//! of which transport is chosen:
//!   * [`BackendGenerationStatus`] / [`BackendGenerationJob`] — the exact decode
//!     shape of `generations:byId`, ported from the reference
//!     `GenerationBackend.swift` (`_id`, `status`, `resultUrls`, `errorMessage`,
//!     `costCredits`, `completedAt`).
//!   * [`CatalogEntry`] — a minimal `models:list` row (the full per-kind caps
//!     decode is E9's job; here we only need enough to prove a live query pushes).
//!   * [`GenerationTransport`] — the trait E9 codes against. The `convex`-crate
//!     impl and the HTTP-polling impl are interchangeable behind it, so the
//!     WS-vs-polling decision is a single swap, not a rewrite (mirrors the S-1b
//!     "named switch" discipline).
//!
//! NB: nothing here touches a prod crate. It is a self-contained reference impl
//! that the E9 author reads and re-expresses inside `palmier-gen`.

use serde::Deserialize;

/// Generation job status. Wire values are lowercase (`queued|running|succeeded|
/// failed`) — matches the reference `BackendGenerationStatus` raw values exactly.
/// The port MUST keep these byte-identical; the backend emits these strings.
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
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

/// One generation job, as pushed by the `generations:byId` live query.
///
/// Field names match the reference Convex document shape (`_id`, `resultUrls`,
/// `errorMessage`, `costCredits`, `completedAt`). `resultUrls` maps 1:1 by index
/// onto the placeholder MediaAssets on success (see generation.md "Index-based
/// result mapping"). `completedAt` is an Apple-reference-epoch double on the wire
/// (Convex stores the Swift `Date`); E9 decodes it with the S-1b
/// `apple_ref_epoch` codec if it ever needs the wall-clock value.
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
    /// Apple-reference-epoch seconds (see S-1b). Decoded lazily by E9 if needed.
    #[serde(default)]
    pub completed_at: Option<f64>,
}

/// A minimal model-catalog row from `models:list`. The reference `CatalogEntry`
/// has a rich custom decoder (per-kind caps, pricing maps); E9 ports that in
/// full. Here we decode just enough to prove the live query delivers an array
/// and re-pushes when the catalog changes.
#[derive(Debug, Clone, Deserialize)]
pub struct CatalogEntry {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// `video|image|audio|upscale` — drives which caps struct E9 decodes.
    #[serde(default)]
    pub kind: Option<String>,
}

/// The contract E9 (`palmier-gen`) codes against. One method to follow a job to
/// settlement, one to load the catalog. The `convex`-crate transport and the
/// HTTP-polling transport both implement it, so swapping WS↔polling is a single
/// type substitution.
///
/// Kept deliberately small: this is the spike's *decision surface*, not the full
/// generation lifecycle (placeholders, upload, download — all of which sit ABOVE
/// this transport in `GenerationService`, transport-agnostic).
pub trait GenerationTransport {
    /// The terminal job (succeeded or failed). Implementations either:
    ///   * (WS) subscribe to `generations:byId` and drive the stream until a
    ///     terminal status arrives, or
    ///   * (HTTP) poll `generations:byId` on an interval until terminal.
    ///
    /// `on_update` is invoked for every intermediate (`queued`/`running`) push so
    /// the UI/generation panel can show progress — matching the reference's
    /// `while let` over the AsyncStream.
    fn follow_job(
        &mut self,
        job_id: &str,
        on_update: &mut dyn FnMut(&BackendGenerationJob),
    ) -> impl std::future::Future<Output = anyhow::Result<BackendGenerationJob>> + Send;

    /// Load the model catalog once (a snapshot). The WS impl can additionally
    /// keep a long-lived subscription for live updates; E9 decides whether the
    /// catalog needs to be reactive or a 24h-cached snapshot (FOUNDATION §6.1
    /// caches `/v1/models` for 24h, implying a snapshot is acceptable).
    fn load_models(
        &mut self,
    ) -> impl std::future::Future<Output = anyhow::Result<Vec<CatalogEntry>>> + Send;
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
        // The backend emits lowercase strings; a casing mismatch would silently
        // break the whole subscription. Guard it.
        let s: BackendGenerationStatus = serde_json::from_str("\"running\"").unwrap();
        assert_eq!(s, BackendGenerationStatus::Running);
        let s: BackendGenerationStatus = serde_json::from_str("\"succeeded\"").unwrap();
        assert_eq!(s, BackendGenerationStatus::Succeeded);
    }

    #[test]
    fn job_decodes_reference_document_shape() {
        // Mirrors the exact field names the reference `BackendGenerationJob`
        // decodes (`_id`, camelCase rest). A live `generations:byId` push must
        // deserialize into this with no remap.
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
        assert_eq!(job.result_urls.as_deref(), Some(&["https://cdn.example/out0.mp4".to_string()][..]));
        assert_eq!(job.cost_credits, Some(42));
        assert!(job.completed_at.is_some());
    }

    #[test]
    fn job_decodes_in_flight_minimal_shape() {
        // A queued job carries only id+status; everything else absent -> None.
        let json = serde_json::json!({ "_id": "job_x", "status": "queued" });
        let job: BackendGenerationJob = serde_json::from_value(json).unwrap();
        assert_eq!(job.status, BackendGenerationStatus::Queued);
        assert!(job.result_urls.is_none());
        assert!(!job.status.is_terminal());
    }
}
