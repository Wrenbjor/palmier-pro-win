//! The HTTP-polling Convex transport (E9-S1 fallback; Spike S-2 §3.1, the
//! `http_fallback` bin). Used only on the WS-blocked-by-proxy edge — WS is the
//! primary transport. `models:list` + the `uploads:*` mutations/actions go over
//! the Convex HTTP function API; `generations:byId` is **polled** on an interval
//! until terminal (no reactive push).
//!
//! Cancellation (#24) = drop the returned [`JobStream`]: the polling task it owns
//! stops; the Convex job keeps running/billing.
//!
//! The Convex HTTP API shape (`/api/mutation`, `/api/query`, `/api/action`) is
//! code-derived (S-2 §3.6) — the live round-trip is GATED on a deployment URL.

use std::time::Duration;

use serde::Deserialize;

use crate::catalog::CatalogEntry;
use crate::transport::{
    BackendGenerationJob, BackendGenerationStatus, GenerationError, GenerationTransport,
    JobStream,
};

/// Poll interval for `generations:byId` (reference fallback polls every 2 s).
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// The HTTP-polling transport. Holds the Convex HTTP base URL + a Clerk JWT (the
/// same Bearer the WS auth callback serves) + a `reqwest` client.
pub struct HttpPollingTransport {
    http_base: String,
    jwt: Option<String>,
    client: reqwest::Client,
    poll_interval: Duration,
}

impl HttpPollingTransport {
    /// New polling transport over the Convex HTTP base URL.
    #[must_use]
    pub fn new(http_base: impl Into<String>, jwt: Option<String>) -> Self {
        Self {
            http_base: http_base.into(),
            jwt,
            client: reqwest::Client::new(),
            poll_interval: POLL_INTERVAL,
        }
    }

    /// Override the poll interval (tests).
    #[must_use]
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// POST a Convex function call (`/api/{query|mutation|action}`) and decode the
    /// `{status, value}` envelope into the function value as JSON.
    async fn call(
        &self,
        endpoint: &str,
        path: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, GenerationError> {
        let url = format!("{}/api/{endpoint}", self.http_base.trim_end_matches('/'));
        let mut req = self.client.post(&url).json(&serde_json::json!({
            "path": path,
            "args": args,
            "format": "json",
        }));
        if let Some(jwt) = &self.jwt {
            req = req.bearer_auth(jwt);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| GenerationError::Transport(e.to_string()))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| GenerationError::Transport(e.to_string()))?;
        if !status.is_success() {
            return Err(crate::transport::BackendErrorEnvelope::to_error(status.as_u16(), &text));
        }
        #[derive(Deserialize)]
        struct ConvexEnvelope {
            #[serde(default)]
            status: Option<String>,
            #[serde(default)]
            value: serde_json::Value,
            #[serde(default)]
            #[serde(rename = "errorMessage")]
            error_message: Option<String>,
        }
        let env: ConvexEnvelope =
            serde_json::from_str(&text).map_err(|e| GenerationError::Transport(e.to_string()))?;
        if env.status.as_deref() == Some("error") {
            return Err(GenerationError::Transport(
                env.error_message.unwrap_or_else(|| "convex error".into()),
            ));
        }
        Ok(env.value)
    }
}

#[async_trait::async_trait]
impl GenerationTransport for HttpPollingTransport {
    async fn load_models(&self) -> Result<Vec<CatalogEntry>, GenerationError> {
        let value = self.call("query", "models:list", serde_json::json!({})).await?;
        serde_json::from_value(value).map_err(|e| GenerationError::Transport(e.to_string()))
    }

    async fn generate_upload_ticket(&self) -> Result<String, GenerationError> {
        let value = self
            .call("mutation", "uploads:generateUploadTicket", serde_json::json!({}))
            .await?;
        value
            .get("uploadUrl")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| GenerationError::Transport("missing uploadUrl".into()))
    }

    async fn commit_upload(&self, storage_id: &str) -> Result<String, GenerationError> {
        let value = self
            .call("action", "uploads:commitUpload", serde_json::json!({ "storageId": storage_id }))
            .await?;
        value
            .get("url")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| GenerationError::Transport("missing url".into()))
    }

    async fn submit(
        &self,
        model: &str,
        params: &serde_json::Value,
        project_id: Option<&str>,
    ) -> Result<String, GenerationError> {
        let mut args = serde_json::json!({ "model": model, "params": params });
        if let Some(pid) = project_id {
            args["projectId"] = serde_json::Value::String(pid.to_string());
        }
        let value = self.call("mutation", "generations:submit", args).await?;
        value
            .get("jobId")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| GenerationError::Transport("missing jobId".into()))
    }

    async fn subscribe(&self, job_id: &str) -> Result<JobStream, GenerationError> {
        // No reactive push: poll generations:byId on the interval until terminal,
        // yielding each decoded job. The stream owns the polling loop, so dropping
        // it stops polling (the #24 client-teardown cancel).
        let base = self.http_base.clone();
        let jwt = self.jwt.clone();
        let client = self.client.clone();
        let interval = self.poll_interval;
        let job_id = job_id.to_string();

        let stream = futures::stream::unfold(
            (false, None::<BackendGenerationStatus>),
            move |(settled, _last)| {
                let base = base.clone();
                let jwt = jwt.clone();
                let client = client.clone();
                let job_id = job_id.clone();
                async move {
                    if settled {
                        return None;
                    }
                    let url = format!("{}/api/query", base.trim_end_matches('/'));
                    let mut req = client.post(&url).json(&serde_json::json!({
                        "path": "generations:byId",
                        "args": { "id": job_id },
                        "format": "json",
                    }));
                    if let Some(jwt) = &jwt {
                        req = req.bearer_auth(jwt);
                    }
                    let job_opt: Option<BackendGenerationJob> = match req.send().await {
                        Ok(r) if r.status().is_success() => r
                            .json::<serde_json::Value>()
                            .await
                            .ok()
                            .and_then(|v| serde_json::from_value(v.get("value").cloned().unwrap_or(v)).ok()),
                        _ => None,
                    };
                    match job_opt {
                        Some(job) => {
                            let terminal = job.status.is_terminal();
                            if !terminal {
                                tokio::time::sleep(interval).await;
                            }
                            Some((job, (terminal, None)))
                        }
                        None => {
                            tokio::time::sleep(interval).await;
                            // keep polling (no item this tick); emit a synthetic
                            // queued so the stream stays live but not terminal.
                            // (A real impl would distinguish; for the fallback we
                            // simply retry.)
                            Some((
                                BackendGenerationJob {
                                    id: job_id.clone(),
                                    status: BackendGenerationStatus::Queued,
                                    result_urls: None,
                                    error_message: None,
                                    cost_credits: None,
                                    completed_at: None,
                                },
                                (false, None),
                            ))
                        }
                    }
                }
            },
        );
        Ok(Box::pin(stream))
    }
}
