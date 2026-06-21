//! The axum loopback HTTP server + the [`McpServer`] start/stop boot seam.
//!
//! [`McpServer::start`] binds a TCP listener to **`127.0.0.1:<port>` only** (SM-C3),
//! mounts the `POST /mcp` JSON-RPC route (with the three validators) and the
//! `GET /.well-known/oauth-protected-resource` route, and serves on a background
//! tokio task. [`McpServer::stop`] triggers a graceful shutdown. The Tauri boot
//! sequence (Epic 1, step 6) calls these behind the `io.palmier.pro.mcp.enabled`
//! pref — that wiring is NOT done here (boot-integration story).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use palmier_tools::ToolExecutor;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::jsonrpc::handle_body;
use crate::validators::validate_request;
use crate::well_known::oauth_protected_resource_body;
use crate::{BIND_HOST, DEFAULT_PORT};

/// Server configuration. The **bind host is always [`BIND_HOST`] (loopback)** — only
/// the port is configurable (SM-C3: the server must never be bindable to a
/// non-localhost interface).
#[derive(Debug, Clone, Copy)]
pub struct ServerConfig {
    /// The loopback port to bind (default [`DEFAULT_PORT`] = 19789).
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> ServerConfig {
        ServerConfig { port: DEFAULT_PORT }
    }
}

/// A callback the server invokes after a **successful mutating** `tools/call`
/// (see [`crate::jsonrpc::handle_body`]). It is a plain `Fn` so `palmier-mcp` stays
/// free of any Tauri dependency: the Tauri layer passes a closure capturing its
/// `AppHandle` that emits `timeline://changed`, so an external MCP client's edit
/// refetches the live Project window the same way the in-app paths do.
///
/// `Arc`-wrapped + `Send + Sync` because the serving task runs on the tokio
/// runtime and the closure is shared across the per-request handler clones.
pub type MutationCallback = Arc<dyn Fn() + Send + Sync>;

/// The shared state every request handler sees: the tool executor, the bound port
/// (needed to compute the allowed Origin and the `.well-known` body), and the
/// optional post-mutation hook.
#[derive(Clone)]
struct AppState {
    executor: Arc<ToolExecutor>,
    port: u16,
    on_mutation: Option<MutationCallback>,
}

/// A running (or stopped) loopback MCP server. Construct with [`McpServer::start`];
/// drop or call [`McpServer::stop`] to shut it down.
pub struct McpServer {
    /// The actual bound address (with the OS-assigned port if `port` was 0).
    local_addr: SocketAddr,
    /// Fires graceful shutdown; `None` once [`McpServer::stop`] has consumed it.
    shutdown: Option<oneshot::Sender<()>>,
    /// The serving task handle, awaited by [`McpServer::stop`].
    task: Option<tokio::task::JoinHandle<()>>,
}

impl McpServer {
    /// Build the axum router for the given state. Exposed for tests.
    fn router(state: AppState) -> Router {
        Router::new()
            .route("/mcp", post(mcp_handler))
            // Some clients probe `/mcp` with GET for an SSE channel; ack it.
            .route("/mcp", get(mcp_get_handler))
            .route(
                "/.well-known/oauth-protected-resource",
                get(well_known_handler),
            )
            .with_state(state)
    }

    /// Start the server bound to loopback on `config.port`. Returns once the listener
    /// is bound (so callers can immediately connect); serving runs on a background
    /// task. The `executor` is the shared [`ToolExecutor`] both this server and the
    /// in-app agent use.
    ///
    /// A `port` of 0 binds an OS-assigned free port (useful for tests); read it back
    /// via [`McpServer::local_addr`].
    pub async fn start(
        executor: Arc<ToolExecutor>,
        config: ServerConfig,
    ) -> std::io::Result<McpServer> {
        McpServer::start_with_hook(executor, config, None).await
    }

    /// Like [`McpServer::start`], but also registers an `on_mutation` callback fired
    /// after every **successful mutating** `tools/call` from an external client (see
    /// [`MutationCallback`]). The Tauri boot seam passes a closure that emits
    /// `timeline://changed` so the open Project window refetches; tests pass a counter
    /// to assert the hook fires for mutations and NOT for reads.
    pub async fn start_with_hook(
        executor: Arc<ToolExecutor>,
        config: ServerConfig,
        on_mutation: Option<MutationCallback>,
    ) -> std::io::Result<McpServer> {
        let addr = SocketAddr::from((BIND_HOST, config.port));
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;

        let state = AppState { executor, port: local_addr.port(), on_mutation };
        let router = McpServer::router(state);

        let (tx, rx) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = rx.await;
                })
                .await;
        });

        Ok(McpServer { local_addr, shutdown: Some(tx), task: Some(task) })
    }

    /// Convenience: start on [`DEFAULT_PORT`] (19789).
    pub async fn start_default(executor: Arc<ToolExecutor>) -> std::io::Result<McpServer> {
        McpServer::start(executor, ServerConfig::default()).await
    }

    /// The actual bound socket address (host is always loopback; port is the real one
    /// even if `config.port` was 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The bound port.
    pub fn port(&self) -> u16 {
        self.local_addr.port()
    }

    /// Gracefully stop the server, awaiting the serving task. Idempotent-ish: calling
    /// twice is harmless (the second call is a no-op).
    pub async fn stop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        // Best-effort shutdown if `stop` was never called: signal the task so it
        // doesn't leak. We can't await here, but the graceful-shutdown future will
        // fire and the task will end.
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// `POST /mcp` — run the three validators, then dispatch the JSON-RPC body.
async fn mcp_handler(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(e) = validate_request(&headers, state.port) {
        return (e.status, e.reason).into_response();
    }

    let body_str = match std::str::from_utf8(&body) {
        Ok(s) => s,
        Err(_) => return (StatusCode::BAD_REQUEST, "body is not valid UTF-8").into_response(),
    };

    match handle_body(body_str, &state.executor, state.on_mutation.as_ref()) {
        Some(response_json) => (
            StatusCode::OK,
            [("content-type", "application/json")],
            response_json,
        )
            .into_response(),
        // All-notification payload: JSON-RPC says no response body. 202 Accepted.
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// `GET /mcp` — some clients open an SSE channel here; reply with a minimal
/// keep-alive stream header so the connection is acknowledged (reference
/// `MCPHTTPServer.swift:84-87`). Validators do NOT gate GET (no body to validate).
async fn mcp_get_handler() -> Response {
    (
        StatusCode::OK,
        [
            ("content-type", "text/event-stream"),
            ("cache-control", "no-cache"),
        ],
        ": connected\n\n",
    )
        .into_response()
}

/// `GET /.well-known/oauth-protected-resource` — the loopback resource metadata.
async fn well_known_handler(State(state): State<AppState>) -> Response {
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        oauth_protected_resource_body(state.port),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_default_port() {
        assert_eq!(ServerConfig::default().port, DEFAULT_PORT);
    }

    #[tokio::test]
    async fn server_binds_loopback_and_reports_addr() {
        let exec = Arc::new(ToolExecutor::new());
        // port 0 → OS-assigned; confirms host is always loopback.
        let mut server = McpServer::start(exec, ServerConfig { port: 0 }).await.unwrap();
        assert!(server.local_addr().ip().is_loopback());
        assert_ne!(server.port(), 0);
        server.stop().await;
    }
}
