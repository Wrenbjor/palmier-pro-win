//! Spike S-2 — FALLBACK transport: plain HTTP via `reqwest`.
//!
//! Used only when the WS sync protocol is unreachable (corporate proxy strips
//! WebSocket upgrades, or the deployment is fronted by an HTTP-only gateway). Two
//! pieces:
//!
//!   A. `/v1/models` GET — the Convex HTTP action FOUNDATION §8.1 lists. This is
//!      a normal REST GET; no WS needed. Auth: optional Clerk JWT bearer.
//!      (This is also the cleaner path for the 24h-cached catalog snapshot.)
//!
//!   B. generations:byId POLLING — Convex exposes an HTTP query endpoint
//!      (`POST {deployment}/api/query`, body {path, args, format}) that runs a
//!      query once and returns its current value. We poll it on an interval until
//!      the job reaches a terminal status. This reproduces the live-query
//!      *outcome* (eventual succeeded/failed) without a socket, at the cost of
//!      latency = poll interval and N extra requests.
//!
//! Run:
//!   pwsh -File ../../scripts/with-msvc.ps1 cargo run --bin http_fallback
//! env:
//!   CONVEX_HTTP_URL=https://<deployment>.convex.site     (the .site HTTP host; for /v1/models)
//!   CONVEX_URL=https://<deployment>.convex.cloud          (the .cloud host; for /api/query)
//!   CONVEX_JWT=<clerk jwt>                                 (optional)
//!   CONVEX_JOB_ID=<job id>                                 (optional; demo poll)
//!
//! Without the URLs set, runs OFFLINE (prints the request shapes, exits 0).

use std::time::Duration;

use anyhow::{Context, Result};
use s2_convex_ws::{BackendGenerationJob, CatalogEntry};

/// Poll cadence for generations:byId in the fallback. 2s balances latency vs
/// request volume; E9 can back off (2s while running, faster on first poll).
const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Safety cap so a stuck job can't poll forever. Generation jobs are minutes-long;
/// 20 min is generous. E9 ties this to the job's own timeout.
const MAX_POLLS: u32 = 600;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,s2_convex_ws=debug".into()),
        )
        .init();

    let http_url = std::env::var("CONVEX_HTTP_URL").ok();
    let cloud_url = std::env::var("CONVEX_URL").ok();
    let jwt = std::env::var("CONVEX_JWT").ok();

    if http_url.is_none() && cloud_url.is_none() {
        print_offline_plan();
        return Ok(());
    }

    let client = reqwest::Client::builder()
        .user_agent("palmier-pro-win/s2-spike")
        .build()?;

    // --- A. /v1/models GET --------------------------------------------------
    if let Some(base) = &http_url {
        match get_models(&client, base, jwt.as_deref()).await {
            Ok(models) => tracing::info!(count = models.len(), "/v1/models GET decoded"),
            Err(e) => tracing::error!(error = %e, "/v1/models failed"),
        }
    }

    // --- B. generations:byId polling ---------------------------------------
    if let (Some(base), Some(job_id)) = (&cloud_url, std::env::var("CONVEX_JOB_ID").ok()) {
        let job = poll_job(&client, base, &job_id, jwt.as_deref()).await?;
        tracing::info!(status = ?job.status, "job settled (via polling)");
    } else {
        tracing::info!("CONVEX_URL or CONVEX_JOB_ID missing — skipping poll demo");
    }

    Ok(())
}

/// `GET {http_base}/v1/models` with optional bearer auth. Decodes the catalog
/// array. The reference's `/v1/models` is an HTTP action returning the same rows
/// `models:list` pushes over WS.
async fn get_models(
    client: &reqwest::Client,
    http_base: &str,
    jwt: Option<&str>,
) -> Result<Vec<CatalogEntry>> {
    let url = format!("{}/v1/models", http_base.trim_end_matches('/'));
    let mut req = client.get(&url);
    if let Some(jwt) = jwt {
        req = req.bearer_auth(jwt);
    }
    let resp = req.send().await.context("GET /v1/models")?;
    let status = resp.status();
    let body = resp.text().await?;
    anyhow::ensure!(status.is_success(), "/v1/models HTTP {status}: {body}");
    // The endpoint may wrap rows as {models:[...]} or return a bare array; try both.
    let entries = serde_json::from_str::<Vec<CatalogEntry>>(&body)
        .or_else(|_| {
            #[derive(serde::Deserialize)]
            struct Wrap {
                models: Vec<CatalogEntry>,
            }
            serde_json::from_str::<Wrap>(&body).map(|w| w.models)
        })
        .context("decode /v1/models body")?;
    Ok(entries)
}

/// Poll `generations:byId` via the Convex HTTP query endpoint until terminal.
///
/// Convex HTTP query API: `POST {deployment}.convex.cloud/api/query` with JSON
/// body `{ "path": "generations:byId", "args": { "id": <jobId> }, "format": "json" }`
/// (+ `Authorization: Bearer <jwt>`). Response: `{ "status": "success", "value": <job> }`
/// or `{ "status": "error", "errorMessage": ... }`. This is the documented
/// single-shot query transport that backs the WS one.
async fn poll_job(
    client: &reqwest::Client,
    cloud_base: &str,
    job_id: &str,
    jwt: Option<&str>,
) -> Result<BackendGenerationJob> {
    let url = format!("{}/api/query", cloud_base.trim_end_matches('/'));
    let body = serde_json::json!({
        "path": "generations:byId",
        "args": { "id": job_id },
        "format": "json",
    });

    for attempt in 0..MAX_POLLS {
        let mut req = client.post(&url).json(&body);
        if let Some(jwt) = jwt {
            req = req.bearer_auth(jwt);
        }
        let resp = req.send().await.context("POST /api/query")?;
        let status = resp.status();
        let env: QueryEnvelope = resp.json().await.context("decode /api/query envelope")?;
        anyhow::ensure!(status.is_success(), "/api/query HTTP {status}");

        match env.status.as_str() {
            "success" => {
                let value = env.value.unwrap_or(serde_json::Value::Null);
                if value.is_null() {
                    tracing::warn!("generations:byId returned null (not found yet)");
                } else {
                    let job: BackendGenerationJob =
                        serde_json::from_value(value).context("decode polled job")?;
                    tracing::info!(attempt, status = ?job.status, "poll");
                    if job.status.is_terminal() {
                        return Ok(job);
                    }
                }
            }
            other => anyhow::bail!(
                "query error: {} — {}",
                other,
                env.error_message.unwrap_or_default()
            ),
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    anyhow::bail!("generations:byId did not settle within {MAX_POLLS} polls")
}

#[derive(serde::Deserialize)]
struct QueryEnvelope {
    status: String,
    #[serde(default)]
    value: Option<serde_json::Value>,
    #[serde(default, rename = "errorMessage")]
    error_message: Option<String>,
}

fn print_offline_plan() {
    tracing::info!("CONVEX_HTTP_URL / CONVEX_URL unset — OFFLINE mode (compile proof, no backend).");
    println!(
        "\nHTTP fallback request shapes:\n\
         \n\
         A. GET  {{CONVEX_HTTP_URL}}/v1/models            [Authorization: Bearer <jwt>?]\n\
            <- 200 [CatalogEntry, ...]   (or {{ models: [...] }})\n\
         \n\
         B. POST {{CONVEX_URL}}/api/query                 [Authorization: Bearer <jwt>?]\n\
            -> {{ \"path\": \"generations:byId\", \"args\": {{ \"id\": \"<jobId>\" }}, \"format\": \"json\" }}\n\
            <- {{ \"status\": \"success\", \"value\": <BackendGenerationJob | null> }}\n\
            ...repeat every 2s until value.status in {{succeeded, failed}}.\n\
         \n\
         Tradeoff vs WS: +latency (poll interval) and +N requests; NO push.\n\
         Use ONLY when the WS upgrade is blocked. WS (bin convex_client) is primary.\n"
    );
}
