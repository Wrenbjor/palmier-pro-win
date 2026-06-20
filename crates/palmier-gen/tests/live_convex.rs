//! Live-Convex round-trip — **GATED** (E9-S1 / Spike S-2 §4–5). Requires a real
//! deployment URL + a Clerk JWT minted for the `convex` template + an existing
//! generation job id. None are available yet (the deployment URL is a build
//! secret — S-2 §5), so every test here is `#[ignore]`d and only runs when the
//! env vars are set:
//!
//! ```text
//! CONVEX_URL=https://<deployment>.convex.cloud
//! CONVEX_JWT=<clerk session token, `convex` template>
//! CONVEX_JOB_ID=<an existing generation job id>      (for the subscribe test)
//! ```
//!
//! Run with:
//! ```text
//! pwsh -File scripts/with-msvc.ps1 cargo test --package palmier-gen --features convex-transport -- --ignored
//! ```
//!
//! This is E9-S1's first acceptance gate (S-2 exit): `/v1/models` over the
//! transport AND a `generations:byId` round-trip. Until a deployment URL lands,
//! the transport is proven only by compilation (the spike) + the mock-driven
//! lifecycle suite.

#![cfg(feature = "convex-transport")]

use std::sync::Arc;

use palmier_gen::{ConvexWsTransport, GenerationTransport, JwtProvider};

/// A static JWT provider sourced from `CONVEX_JWT` (anonymous if unset).
struct EnvJwt(Option<String>);
impl JwtProvider for EnvJwt {
    fn jwt(&self, _force_refresh: bool) -> Option<String> {
        self.0.clone()
    }
}

fn url() -> Option<String> {
    std::env::var("CONVEX_URL").ok()
}

#[tokio::test]
#[ignore = "GATED: needs a live Convex deployment URL + Clerk JWT (S-2 §5)"]
async fn live_models_list_round_trip() {
    let Some(url) = url() else {
        eprintln!("CONVEX_URL unset — skipping (this test is #[ignore]d by default)");
        return;
    };
    let jwt = std::env::var("CONVEX_JWT").ok();
    let transport = ConvexWsTransport::connect(&url, Arc::new(EnvJwt(jwt)))
        .await
        .expect("connect");
    let models = transport.load_models().await.expect("models:list");
    assert!(!models.is_empty(), "expected a non-empty catalog");
}

#[tokio::test]
#[ignore = "GATED: needs a live deployment URL + Clerk JWT + CONVEX_JOB_ID (S-2 §5)"]
async fn live_generations_by_id_round_trip() {
    use futures::StreamExt;
    let Some(url) = url() else { return };
    let Some(job_id) = std::env::var("CONVEX_JOB_ID").ok() else {
        eprintln!("CONVEX_JOB_ID unset — skipping the subscribe round-trip");
        return;
    };
    let jwt = std::env::var("CONVEX_JWT").ok();
    let transport = ConvexWsTransport::connect(&url, Arc::new(EnvJwt(jwt)))
        .await
        .expect("connect");
    let mut stream = transport.subscribe(&job_id).await.expect("subscribe");
    // Pump until terminal (or the stream ends).
    while let Some(job) = stream.next().await {
        if job.status.is_terminal() {
            return;
        }
    }
}
