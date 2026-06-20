//! Spike S-2 — RECOMMENDED transport: the official `convex` Rust crate (WS sync).
//!
//! Proves (in code; live-exercise gated on a reachable deployment + Clerk JWT —
//! see FINDINGS §4) that a Rust client can do EXACTLY what the macOS reference
//! `GenerationBackend.swift` does with ConvexMobile:
//!   * `convex.subscribe("generations:byId", {id})` -> reactive job stream
//!   * `convex.mutation("generations:submit", {...})` -> jobId
//!   * `convex.subscribe("models:list")` -> live catalog
//!   * dynamic Clerk-JWT auth refreshed on reconnect (`set_auth_callback`)
//!
//! Run (needs a real deployment URL + a Clerk JWT minted for the `convex` JWT
//! template):
//!   pwsh -File ../../scripts/with-msvc.ps1 cargo run --bin convex_client
//! with env:
//!   CONVEX_URL=https://<deployment>.convex.cloud
//!   CONVEX_JWT=<clerk session token, `convex` template>   (optional; anon if unset)
//!   CONVEX_JOB_ID=<an existing generation job id>          (optional; demo subscribe)
//!
//! Without CONVEX_URL set, `main` runs the OFFLINE path: it prints the protocol
//! plan and the exact call shapes, and exits 0 (so `cargo run` proves the code
//! compiles + the API surface is correct without a backend). This is the
//! "code-derived, live-confirmation deferred to E9" posture FINDINGS describes.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use anyhow::{Context, Result};
use convex::{ConvexClient, FunctionResult, Value};
use futures::StreamExt;
use s2_convex_ws::{BackendGenerationJob, BackendGenerationStatus, CatalogEntry};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,s2_convex_ws=debug,convex=info".into()),
        )
        .init();

    let Ok(url) = std::env::var("CONVEX_URL") else {
        print_offline_plan();
        return Ok(());
    };

    tracing::info!(%url, "connecting to Convex deployment (WS sync protocol)");
    let mut client = ConvexClient::new(&url)
        .await
        .with_context(|| format!("ConvexClient::new({url}) — WS connect/handshake failed"))?;

    // --- Auth: dynamic Clerk-JWT callback (refreshed on reconnect). ---------
    // In the real app, palmier-auth holds the cached Clerk session token (the
    // `convex` JWT template) and refreshes it; here we read it once from env and
    // re-serve it. `force_refresh=true` on reconnect is where palmier-auth would
    // call Clerk's `session.getToken()` again. If no token: stay anonymous (the
    // catalog/`/v1/models` may be public; generations require auth).
    if let Ok(jwt) = std::env::var("CONVEX_JWT") {
        set_clerk_auth(&mut client, jwt).await;
        tracing::info!("auth set via set_auth_callback (Clerk JWT)");
    } else {
        tracing::warn!("no CONVEX_JWT — proceeding anonymous; generations:submit will likely 401");
    }

    // --- 1. models:list — live catalog (proves a live query pushes). --------
    match load_models(&mut client).await {
        Ok(models) => tracing::info!(count = models.len(), "models:list snapshot decoded"),
        Err(e) => tracing::error!(error = %e, "models:list failed"),
    }

    // --- 2. generations:byId — the generation subscription. -----------------
    // If a job id is provided, follow it to settlement exactly like the
    // reference's `while let` over the AsyncStream.
    if let Ok(job_id) = std::env::var("CONVEX_JOB_ID") {
        let final_job = follow_job(&mut client, &job_id, &mut |j| {
            tracing::info!(status = ?j.status, "generations:byId push");
        })
        .await?;
        tracing::info!(status = ?final_job.status, "job settled");
        report(&final_job);
    } else {
        tracing::info!("no CONVEX_JOB_ID — skipping generations:byId demo");
    }

    Ok(())
}

/// Install a dynamic auth callback that serves a (static, in this demo) Clerk JWT.
/// In the port this closure delegates to palmier-auth's token cache; the
/// `force_refresh` arg fires on WS reconnect so the cache can re-mint from Clerk.
async fn set_clerk_auth(client: &mut ConvexClient, jwt: String) {
    let fetcher = Box::new(move |_force_refresh: bool| {
        let jwt = jwt.clone();
        let fut: Pin<Box<dyn Future<Output = Result<convex::AuthenticationToken, anyhow::Error>> + Send>> =
            Box::pin(async move { Ok(convex::AuthenticationToken::User(jwt)) });
        fut
    });
    client.set_auth_callback(Some(fetcher)).await;
}

/// Load the model catalog. Uses a one-shot `query` (the crate's `query` is a
/// `subscribe` that returns the first value), which is the right shape for the
/// 24h-cached snapshot FOUNDATION §6.1 wants. For a *reactive* catalog, swap to
/// `subscribe` + a long-lived task.
async fn load_models(client: &mut ConvexClient) -> Result<Vec<CatalogEntry>> {
    let result = client
        .query("models:list", BTreeMap::new())
        .await
        .context("models:list query")?;
    let value = expect_value(result).context("models:list returned an error")?;
    let json = value_to_json(&value);
    let entries: Vec<CatalogEntry> =
        serde_json::from_value(json).context("decode models:list -> Vec<CatalogEntry>")?;
    Ok(entries)
}

/// Follow `generations:byId(jobId)` to a terminal status. This is the spike's
/// core proof: subscribe, pump the stream, decode each push into
/// `BackendGenerationJob`, stop on succeeded/failed. The subscription
/// auto-unsubscribes when the stream is dropped (the reference's
/// `continuation.onTermination` cancel) — that IS the #24 client-teardown cancel.
async fn follow_job(
    client: &mut ConvexClient,
    job_id: &str,
    on_update: &mut dyn FnMut(&BackendGenerationJob),
) -> Result<BackendGenerationJob> {
    let mut args = BTreeMap::new();
    args.insert("id".to_string(), Value::String(job_id.to_string()));

    let mut sub = client
        .subscribe("generations:byId", args)
        .await
        .context("subscribe generations:byId")?;

    while let Some(result) = sub.next().await {
        let value = match result {
            FunctionResult::Value(v) => v,
            FunctionResult::ErrorMessage(e) => {
                anyhow::bail!("generations:byId server error: {e}")
            }
            FunctionResult::ConvexError(e) => {
                anyhow::bail!("generations:byId app error: {}", e.message)
            }
        };

        // The query yields `BackendGenerationJob?` — Convex `null` => job vanished.
        let json = value_to_json(&value);
        if json.is_null() {
            tracing::warn!("generations:byId pushed null (job not found / removed)");
            continue;
        }
        let job: BackendGenerationJob =
            serde_json::from_value(json).context("decode generations:byId push")?;

        on_update(&job);
        if job.status.is_terminal() {
            return Ok(job);
        }
        // else queued|running -> keep listening (dropping `sub` later cancels).
    }

    anyhow::bail!("generations:byId stream ended before the job settled")
}

fn report(job: &BackendGenerationJob) {
    match job.status {
        BackendGenerationStatus::Succeeded => {
            let urls = job.result_urls.clone().unwrap_or_default();
            tracing::info!(?urls, cost = ?job.cost_credits, "SUCCESS — download these 1:1 onto placeholders");
        }
        BackendGenerationStatus::Failed => {
            tracing::error!(reason = ?job.error_message, "FAILED — mark all placeholders .failed");
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// convex::Value <-> serde_json bridge.
//
// The `convex` crate has its own `Value` type (the Convex value system: Null,
// Int64, Float64, Boolean, String, Bytes, Array, Object, ...). We bridge to
// serde_json so the existing reference decode shapes (BackendGenerationJob,
// CatalogEntry) work unchanged. E9 may instead decode straight from
// convex::Value, but the serde bridge keeps the wire contract in ONE place.
// ---------------------------------------------------------------------------

fn expect_value(result: FunctionResult) -> Result<Value> {
    match result {
        FunctionResult::Value(v) => Ok(v),
        FunctionResult::ErrorMessage(e) => anyhow::bail!("server error: {e}"),
        FunctionResult::ConvexError(e) => anyhow::bail!("app error: {}", e.message),
    }
}

fn value_to_json(v: &Value) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        Value::Null => J::Null,
        Value::Int64(i) => J::Number((*i).into()),
        Value::Float64(f) => serde_json::Number::from_f64(*f).map(J::Number).unwrap_or(J::Null),
        Value::Boolean(b) => J::Bool(*b),
        Value::String(s) => J::String(s.clone()),
        Value::Bytes(b) => J::Array(b.iter().map(|byte| J::Number((*byte).into())).collect()),
        Value::Array(a) => J::Array(a.iter().map(value_to_json).collect()),
        Value::Object(o) => {
            J::Object(o.iter().map(|(k, val)| (k.clone(), value_to_json(val))).collect())
        }
    }
}

fn print_offline_plan() {
    tracing::info!("CONVEX_URL unset — OFFLINE mode (compile + API-surface proof, no backend).");
    println!(
        "\nConvex WS sync protocol — call plan the `convex` crate executes for us:\n\
         \n\
         1. ConvexClient::new(CONVEX_URL)\n\
            -> opens wss://<deployment>.convex.cloud/api/<v>/sync, sends ClientMessage::Connect\n\
               {{ session_id, connection_count: 0, last_close_reason: \"InitialConnect\" }}\n\
         2. set_auth_callback(Clerk JWT)\n\
            -> ClientMessage::Authenticate {{ base_version, token: User(<jwt>) }}\n\
         3. query/subscribe \"models:list\"\n\
            -> ClientMessage::ModifyQuerySet {{ Add(query_id, \"models:list\", args) }}\n\
            <- ServerMessage::Transition {{ StateModification::QueryUpdated(query_id, value) }}\n\
         4. mutation \"generations:submit\" {{ model, params, projectId }}\n\
            -> ClientMessage::Mutation {{ request_id, udf_path, args }}\n\
            <- ServerMessage::MutationResponse {{ request_id, result: jobId }}\n\
         5. subscribe \"generations:byId\" {{ id: jobId }}\n\
            -> ClientMessage::ModifyQuerySet {{ Add(query_id2, \"generations:byId\", {{id}}) }}\n\
            <- Transition QueryUpdated(query_id2, job@queued)\n\
            <- Transition QueryUpdated(query_id2, job@running)\n\
            <- Transition QueryUpdated(query_id2, job@succeeded|failed)   [TERMINAL]\n\
         6. drop subscription -> ModifyQuerySet {{ Remove(query_id2) }}   [#24 client cancel]\n\
         \n\
         Set CONVEX_URL (+ CONVEX_JWT, CONVEX_JOB_ID) to exercise it live.\n"
    );
}
