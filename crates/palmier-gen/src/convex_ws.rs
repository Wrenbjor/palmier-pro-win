//! The WS Convex transport (E9-S1 primary; reference `ConvexMobile`). Built on
//! the official `convex` crate over the WebSocket sync protocol (ruling #25 /
//! Spike S-2). Feature-gated behind `convex-transport` so the lifecycle + mock +
//! every unit test build network-free.
//!
//! Lifts the spike's `convex_client.rs`: `ConvexClient::new` → `set_auth_callback`
//! (Clerk JWT) → `query("models:list")` / `subscribe("generations:byId")` /
//! `mutation`/`action`. The `convex::Value` ↔ `serde_json` bridge keeps the wire
//! decode in one place (the same shapes the mock + the rest of the lifecycle use).
//!
//! **Live round-trip is GATED** (no deployment URL / Clerk account yet — S-2 §5).
//! The construction + auth + call shapes compile against the real `convex` 0.10
//! API (proven by the spike); the live exercise is E9-S1's first acceptance gate
//! the moment a deployment URL lands. The integration test that opens a socket is
//! `#[ignore]`d (see `tests/`).

#![cfg(feature = "convex-transport")]

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use convex::{ConvexClient, FunctionResult, Value};
use futures::StreamExt;
use tokio::sync::Mutex;

use crate::catalog::CatalogEntry;
use crate::transport::{
    BackendGenerationJob, GenerationError, GenerationTransport, JobStream,
};

/// A Clerk-JWT provider the transport calls to mint/refresh the token. The app
/// wires this to `palmier-auth`'s token cache; `force_refresh` (on WS reconnect)
/// re-mints from Clerk (S-2 §3.3). Returns `None` to stay anonymous.
pub trait JwtProvider: Send + Sync + 'static {
    /// Provide the current Clerk JWT (the `convex` template). `force_refresh` is
    /// set on reconnect so the cache re-pulls an expired token.
    fn jwt(&self, force_refresh: bool) -> Option<String>;
}

/// The WS transport. Wraps the `convex` client (Clone over one socket) behind a
/// `Mutex` so `&self` async calls serialize their use of the client.
pub struct ConvexWsTransport {
    client: Mutex<ConvexClient>,
}

impl ConvexWsTransport {
    /// Connect to a Convex deployment and install the Clerk-JWT auth callback.
    ///
    /// GATED: opens a real WS socket — only call with a reachable deployment URL
    /// + a Clerk session (S-2 §5). The lifecycle code paths and tests never call
    /// this (they use the mock / polling transport).
    pub async fn connect(
        deployment_url: &str,
        jwt_provider: Arc<dyn JwtProvider>,
    ) -> Result<Self, GenerationError> {
        let mut client = ConvexClient::new(deployment_url)
            .await
            .map_err(|e| GenerationError::Transport(format!("ConvexClient::new: {e}")))?;

        // Dynamic auth callback: serve the Clerk JWT, re-minted on reconnect.
        let provider = Arc::clone(&jwt_provider);
        let fetcher = Box::new(move |force_refresh: bool| {
            let provider = Arc::clone(&provider);
            let fut: Pin<
                Box<dyn Future<Output = Result<convex::AuthenticationToken, anyhow::Error>> + Send>,
            > = Box::pin(async move {
                match provider.jwt(force_refresh) {
                    Some(jwt) => Ok(convex::AuthenticationToken::User(jwt)),
                    None => Ok(convex::AuthenticationToken::None),
                }
            });
            fut
        });
        client.set_auth_callback(Some(fetcher)).await;

        Ok(Self {
            client: Mutex::new(client),
        })
    }
}

/// `convex::Value` → `serde_json::Value` bridge (lifted from the spike).
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

/// `serde_json::Value` → `convex::Value` bridge for mutation/action args.
fn json_to_value(j: &serde_json::Value) -> Value {
    use serde_json::Value as J;
    match j {
        J::Null => Value::Null,
        J::Bool(b) => Value::Boolean(*b),
        J::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int64(i)
            } else {
                Value::Float64(n.as_f64().unwrap_or(0.0))
            }
        }
        J::String(s) => Value::String(s.clone()),
        J::Array(a) => Value::Array(a.iter().map(json_to_value).collect()),
        J::Object(o) => {
            Value::Object(o.iter().map(|(k, v)| (k.clone(), json_to_value(v))).collect())
        }
    }
}

fn expect_value(result: FunctionResult) -> Result<Value, GenerationError> {
    match result {
        FunctionResult::Value(v) => Ok(v),
        FunctionResult::ErrorMessage(e) => Err(GenerationError::Transport(e)),
        FunctionResult::ConvexError(e) => Err(GenerationError::Transport(e.message)),
    }
}

#[async_trait::async_trait]
impl GenerationTransport for ConvexWsTransport {
    async fn load_models(&self) -> Result<Vec<CatalogEntry>, GenerationError> {
        let mut client = self.client.lock().await;
        let result = client
            .query("models:list", BTreeMap::new())
            .await
            .map_err(|e| GenerationError::Transport(e.to_string()))?;
        let value = expect_value(result)?;
        let json = value_to_json(&value);
        serde_json::from_value(json).map_err(|e| GenerationError::Transport(e.to_string()))
    }

    async fn generate_upload_ticket(&self) -> Result<String, GenerationError> {
        let mut client = self.client.lock().await;
        let result = client
            .mutation("uploads:generateUploadTicket", BTreeMap::new())
            .await
            .map_err(|e| GenerationError::Transport(e.to_string()))?;
        let json = value_to_json(&expect_value(result)?);
        json.get("uploadUrl")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| GenerationError::Transport("missing uploadUrl".into()))
    }

    async fn commit_upload(&self, storage_id: &str) -> Result<String, GenerationError> {
        let mut client = self.client.lock().await;
        let mut args = BTreeMap::new();
        args.insert("storageId".to_string(), Value::String(storage_id.to_string()));
        let result = client
            .action("uploads:commitUpload", args)
            .await
            .map_err(|e| GenerationError::Transport(e.to_string()))?;
        let json = value_to_json(&expect_value(result)?);
        json.get("url")
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
        let mut client = self.client.lock().await;
        let mut args = BTreeMap::new();
        args.insert("model".to_string(), Value::String(model.to_string()));
        args.insert("params".to_string(), json_to_value(params));
        if let Some(pid) = project_id {
            args.insert("projectId".to_string(), Value::String(pid.to_string()));
        }
        let result = client
            .mutation("generations:submit", args)
            .await
            .map_err(|e| GenerationError::Transport(e.to_string()))?;
        let json = value_to_json(&expect_value(result)?);
        json.get("jobId")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| GenerationError::Transport("missing jobId".into()))
    }

    async fn subscribe(&self, job_id: &str) -> Result<JobStream, GenerationError> {
        let mut client = self.client.lock().await;
        let mut args = BTreeMap::new();
        args.insert("id".to_string(), Value::String(job_id.to_string()));
        let sub = client
            .subscribe("generations:byId", args)
            .await
            .map_err(|e| GenerationError::Transport(e.to_string()))?;
        // Map each FunctionResult push → decoded BackendGenerationJob, filtering
        // null/error pushes. Dropping the returned stream drops `sub` → the WS
        // ModifyQuerySet::Remove (the #24 client-teardown cancel).
        let stream = sub.filter_map(|result| async move {
            let value = match result {
                FunctionResult::Value(v) => v,
                FunctionResult::ErrorMessage(_) | FunctionResult::ConvexError(_) => return None,
            };
            let json = value_to_json(&value);
            if json.is_null() {
                return None;
            }
            serde_json::from_value::<BackendGenerationJob>(json).ok()
        });
        Ok(Box::pin(stream))
    }
}
