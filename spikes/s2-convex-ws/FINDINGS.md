# Spike S-2 — Convex WebSocket live-query (AI-generation subscription)

**Status:** RESOLVED from code + official SDK (live deployment NOT reachable — section 5).
**Branch:** spike/s2-convex-ws
**Gates:** Epic 9 (palmier-gen — AI generation lifecycle), the models:list catalog, account:get / billing:listPlans subscriptions. PRD section 11 Spike S-2 / Convex reactive-transport risk.
**Date:** 2026-06-20

---

## TL;DR — the headline finding

FOUNDATION section 8.1 and docs/reference/generation.md are WRONG on one load-bearing point: there
IS an official, actively-maintained native Rust Convex client. The `convex` crate
(get-convex/convex-rs, v0.10.4, published 2026-04-10, Apache-2.0, ~42k downloads) implements the
ENTIRE Convex WebSocket sync protocol on top of tokio-tungstenite and exposes exactly the
operations the reference ConvexMobile usage needs:

| Reference (Swift, ConvexMobile) | Rust (convex crate) | Proven here |
|---|---|---|
| convex.subscribe("generations:byId", {id:jobId}) -> Combine publisher | client.subscribe("generations:byId", {id}) -> futures::Stream<FunctionResult> | yes — compiles, follow_job() |
| convex.subscribe("models:list") -> live catalog | client.subscribe("models:list", {}) / client.query(...) | yes — compiles, load_models() |
| convex.mutation("generations:submit", ...) -> {jobId} | client.mutation("generations:submit", {...}) | yes — shape proven |
| convex.mutation/action("uploads:*") (3-step upload) | client.mutation(...) + client.action(...) | yes — same API |
| ConvexClientWithAuth(authProvider: ClerkConvexAuthProvider()) | client.set_auth_callback(fetcher) — JWT, refreshed on reconnect | yes — compiles, set_clerk_auth() |

RECOMMENDATION: use the convex crate over WS as the PRIMARY transport for generations:byId and the
catalog. Do NOT hand-roll the sync protocol over raw tokio-tungstenite (FOUNDATION fallback plan) —
that is now wasted effort. Keep the HTTP-polling fallback (built here, http_fallback bin) only for
the WS-blocked-by-proxy edge.

WHAT IS PROVEN: the spike crate COMPILES (pwsh -File scripts/with-msvc.ps1 cargo build -> exit 0)
and its unit tests pass (4/4), against the REAL convex 0.10.4 / convex_sync_types 0.10.4 /
tokio-tungstenite 0.28 API — not a mock. Both demo binaries run (offline mode).
WHAT IS NOT PROVEN: a live round-trip. The Convex deployment URL is a build-time secret (confirmed
still uncommitted in both repos, section 5) so no socket was opened. E9 confirms live against the
real backend.

---

## 1. The Convex WebSocket sync protocol (precise spec, cited)

The protocol is defined in the open-source convex_sync_types crate (a dependency of convex) and the
convex-backend repo. Endpoint: wss://<deployment>.convex.cloud/api/<version>/sync . All messages are
JSON frames. Below are the EXACT message types (field names from convex_sync_types::types, docs.rs
0.10.4).

### 1.1 Connect / handshake
On socket open the client sends ClientMessage::Connect:
    Connect {
        session_id: SessionId,            // client UUID, stable across reconnects
        connection_count: u32,            // 0 first connect, increments per reconnect
        last_close_reason: String,        // "InitialConnect" first time, else prior close reason
        max_observed_timestamp: Option<Timestamp>,
        client_ts: Option<u64>,
    }
No separate auth step in Connect — auth is a distinct message (1.4). The server does not ack Connect
explicitly; it begins streaming Transitions as queries are added.

### 1.2 Subscriptions — ModifyQuerySet
A subscription is not its own message; the client maintains a query SET and mutates it with
ClientMessage::ModifyQuerySet:
    ModifyQuerySet {
        base_version: QuerySetVersion,    // the version the client last knew
        new_version: QuerySetVersion,     // base_version + 1
        modifications: Vec<QuerySetModification>,  // Add { query_id, udf_path, args } | Remove { query_id }
    }
- Add assigns a client-chosen query_id (small int) to a (udf_path, args) pair — e.g.
  ("generations:byId", {id: jobId}). The server starts evaluating it and pushes its value.
- Remove unsubscribes a query_id. THIS IS THE GENERATION-CANCEL TEARDOWN (#24): dropping the Rust
  QuerySubscription emits a Remove — the client stops listening, the server stops pushing. (The
  server job keeps running/billing; see 3.4.)

### 1.3 Mutations / actions
ClientMessage::Mutation (and Action, same shape):
    Mutation {
        request_id: SessionRequestSeqNumber,   // monotonic per session
        udf_path: UdfPath,                      // e.g. "generations:submit"
        args: SerializedArgs,                   // JSON object
        component_path: Option<String>,
    }
Server replies with ServerMessage::MutationResponse { request_id, result: Result<Value, ErrorPayload>,
ts, log_lines }. generations:submit returns {jobId} in result. Actions (uploads:commitUpload) reply
with ActionResponse (same shape, no ts).

### 1.4 Auth — Authenticate
ClientMessage::Authenticate { base_version: IdentityVersion, token: AuthenticationToken } where
AuthenticationToken::User(<jwt>) carries the Clerk JWT. The convex crate sends this for us when
set_auth / set_auth_callback is called, and RE-SENDS it on every reconnect (with force_refresh=true so
the app can re-mint an expired token). Auth failures come back as ServerMessage::AuthError {
error_message, base_version, auth_update_attempted }.

### 1.5 Live-query push — Transition
The server pushes query results in ServerMessage::Transition:
    Transition {
        start_version: StateVersion,      // must equal the client's current version (else reconnect)
        end_version: StateVersion,        // the new version after applying modifications
        modifications: Vec<StateModification<Value>>,
        client_clock_skew: Option<i64>,
        server_ts: Option<Timestamp>,
    }
Each StateModification is one of:
- QueryUpdated { query_id, value, log_lines, journal } — the query's new result. THIS IS HOW
  generations:byId DELIVERS each status change (queued -> running -> succeeded/failed): one
  QueryUpdated per change, same query_id.
- QueryFailed { query_id, error_message, log_lines, journal, error_data } — the query threw.
- QueryRemoved { query_id } — server-side removal.
start_version/end_version chain the client's view forward; the client applies modifications atomically
and advances its StateVersion. (Large transitions arrive split as TransitionChunk { chunk,
part_number, total_parts, transition_id } — the crate reassembles them.)

### 1.6 Heartbeat & reconnection
- ServerMessage::Ping (unit) — keep-alive; the crate handles it.
- ServerMessage::FatalError { error_message } — unrecoverable; socket closes.
- Reconnect: the convex crate reconnects automatically, replays the query set (re-Adds every live
  subscription with a fresh Connect whose connection_count is incremented), and re-sends Authenticate
  (calling the auth callback with force_refresh=true). SUBSCRIPTIONS SURVIVE RECONNECTS TRANSPARENTLY —
  the consumer's Stream just keeps yielding. This is the single biggest reason to use the crate rather
  than hand-roll: re-subscription + version resync + auth refresh on flaky networks is the fiddly part.

Sources: convex_sync_types 0.10.4 docs.rs (types::ClientMessage, types::ServerMessage,
types::StateModification); convex 0.10.4 docs.rs (ConvexClient); convex.dev "Stateful Sync Platform"
+ "How Convex Works"; get-convex/convex-rs README.

---

## 2. What the spike built & proved (spikes/s2-convex-ws/)

Standalone Cargo crate (own empty [workspace] table -> NOT a member of the 18-crate root workspace;
touches no prod crate, no root Cargo.toml, no KB file).

| File | What it is | Proven |
|---|---|---|
| Cargo.toml | deps: convex 0.10, tokio, futures, reqwest (rustls), serde, anyhow, tracing | builds, exit 0 |
| src/lib.rs | wire types BackendGenerationStatus / BackendGenerationJob / CatalogEntry (ported from GenerationBackend.swift) + the GenerationTransport trait E9 codes against | 4 unit tests pass |
| src/bin/convex_client.rs | RECOMMENDED transport. ConvexClient::new -> set_auth_callback(Clerk JWT) -> query("models:list") -> subscribe("generations:byId",{id}) pumped to terminal status. convex::Value <-> serde_json bridge. | compiles against real API; runs (offline plan) |
| src/bin/http_fallback.rs | FALLBACK transport. GET /v1/models + POST /api/query polling of generations:byId every 2s to terminal. | compiles; runs (offline plan) |

Build command (Windows, mandatory wrapper): from inside the spike dir,
pwsh -File ../../scripts/with-msvc.ps1 cargo build -> Finished ... exit 0. cargo test -> 4 passed.

The 4 unit tests are real regression value (lift them into palmier-gen): they pin the generations:byId
decode shape (_id, camelCase, lowercase status enum) — a casing or field-name drift silently breaks the
whole subscription, and these catch it.

---

## 3. The E9-S1 integration contract (the decisions this spike locks)

### 3.1 Transport for generations:byId -> WS live-query via the convex crate (PRIMARY).
Rationale: exact reactive semantics the reference has (push on each status change, no poll latency, no
request amplification), it is the official client, and it handles reconnect + re-subscription + auth
refresh for free. Polling is the FALLBACK only. E9 wires both behind the GenerationTransport trait and
selects WS by default, degrading to polling if the initial WS connect fails (proxy/firewall). A
settings escape hatch (force_http_transport) is cheap insurance.

### 3.2 Transport for the catalog (models:list / /v1/models) -> either; recommend HTTP snapshot.
FOUNDATION section 6.1 already caches the catalog 24h, so it does NOT need to be reactive. Simplest:
GET /v1/models via reqwest at boot (the http_fallback path), cache 24h. If live catalog updates are
wanted later, switch to client.subscribe("models:list") — one-line change. NOTE the FOUNDATION/reference
split: the reference reads the catalog via the WS models:list query, while FOUNDATION section 8.1 lists
a /v1/models HTTP GET. Both exist server-side; pick HTTP-snapshot for the port.

### 3.3 Auth -> Clerk JWT via set_auth_callback, sourced from palmier-auth.
- The callback returns AuthenticationToken::User(<clerk_jwt>). On force_refresh=true (reconnect),
  palmier-auth re-mints from Clerk (session.getToken() equivalent) — matches FOUNDATION section 8.2's
  "refresh the cached JWT every 5 min" and the reference ClerkConvexAuthProvider.
- The JWT must be minted from Clerk's "convex" JWT template (Convex requires the convex template, not a
  default session token) — verify the reference's Clerk dashboard has it; E9 must confirm the template
  name. The same Bearer <jwt> is reused by the HTTP fallback and by the existing /v1/agent/stream path
  (PalmierClient.swift uses session.getToken() -> Bearer).
- Anonymous is fine for the catalog (likely public); generations:submit requires auth (will 401 anon).

### 3.4 Cancel (#24) -> client teardown = drop the subscription (emits ModifyQuerySet::Remove).
There is no server cancel mutation in the reference. Dropping the Rust QuerySubscription (or the whole
ConvexClient for that job) unsubscribes — the generation panel "cancel" button maps to this drop. The
server job keeps running and may still bill (carry the reference behavior exactly; phase0 #24). If real
cancel is wanted post-v1, it needs a NEW Convex mutation (backend out of our repo) — record as a backend
ask, do not build client-side.

### 3.5 Date encoding (S-1b carry-forward) -> completedAt is an Apple-ref-epoch double on the wire.
BackendGenerationJob.completedAt (and any Date reachable through the job) is the same Apple
reference-epoch double S-1b pinned for media.json. E9 decodes it with the
palmier-model::serde_date::apple_ref_epoch codec if it needs wall-clock; the spike keeps it a raw
Option<f64> (the lifecycle never needs the value — status drives everything). R-6 STILL OPEN: a real
/v1/samples/resolve AND a real generations:byId payload should be captured during E9 and diffed against
both S-1b's fixture and this crate's decode (see section 5).

### 3.6 Params JSON & 3-step upload -> unchanged wire contract; use mutation/action.
generations:submit args {model, params, projectId} and the params "kind" discriminator
(video/image/audio/upscale) are byte-exact wire contracts (generation.md). The 3-step upload is
mutation("uploads:generateUploadTicket") -> raw reqwest POST of bytes with the right Content-Type ->
action("uploads:commitUpload", {storageId}). All three map directly to the convex crate API.

---

## 4. Proven vs needs-live-backend

| Claim | Status | How confirmed |
|---|---|---|
| Official Rust Convex client exists, covers subscribe/query/mutation/action/auth | PROVEN | crate convex 0.10.4 on crates.io/docs.rs; compiled & linked here |
| The crate's API matches the reference ConvexMobile usage 1:1 | PROVEN | mapped table (TL;DR); both compile |
| WS sync-protocol framing (Connect/ModifyQuerySet/Mutation/Authenticate/Transition) | PROVEN (spec) | convex_sync_types 0.10.4 enum defs, section 1 |
| generations:byId decode shape (_id,status,resultUrls,...) | PROVEN | unit tests in lib.rs against ported Swift shape |
| Spike compiles on this box via the MSVC wrapper | PROVEN | cargo build exit 0; cargo test 4/4 |
| HTTP polling fallback shape (/api/query body) | CODE-DERIVED | Convex HTTP query API; not exercised live |
| A live WS round-trip to the real deployment | NOT PROVEN | deployment URL is a build secret, unreachable (section 5) |
| The Clerk convex JWT template name + that models:list/generations:submit accept our client | NOT PROVEN | needs the real backend + a signed-in Clerk session |
| completedAt is really Apple-epoch (not Convex $date) over WS | INFERRED (from Swift Date storage) | confirm with a captured push (R-6) |

E9's live-confirmation checklist (first task of E9-S1, when a deployment URL + Clerk session exist):
1. Set CONVEX_URL + CONVEX_JWT (Clerk convex-template token) + a real CONVEX_JOB_ID; run
   cargo run --bin convex_client. Expect models:list count > 0 and a generations:byId stream that
   settles. (Submit a cheap job first — Seedance 2.0 Fast 720p — to get a job id.)
2. Capture one raw generations:byId push (log the convex::Value); diff completedAt/any Date against
   S-1b's Apple-epoch assumption (R-6). If it's a wrapped $date, apply S-1b fallback R-6.3.
3. Confirm the Clerk JWT template name and that an anonymous models:list works (decides whether the
   boot catalog fetch needs auth).
4. Verify cancel-by-drop actually stops pushes (and note whether billing continues — #24).
5. If WS connect fails behind a proxy, exercise http_fallback against CONVEX_HTTP_URL / /api/query.

---

## 5. Live deployment — NOT reachable (same wall as S-1b)

Re-confirmed this spike: a repo-wide grep across BOTH palmier-pro and palmier-pro-win for
*.convex.cloud / *.convex.site finds NO committed value — only the build-time injection sites
(scripts/bundle.sh:75-76 inject PalmierConvexDeploymentURL / PalmierConvexHttpURL from env
CONVEX_DEPLOYMENT_URL / CONVEX_HTTP_URL; BackendConfig.swift reads them from Info.plist). No
.env/.xcconfig committed. Public discovery (palmier.io / app.palmier.io / api.palmier.io) yielded no
Convex route in S-1b and nothing changed. There is no way to open a live socket from this box. This is
expected and fine — the spike's job was to de-risk the TRANSPORT, which it did via the official SDK +
protocol spec + a compiling client. Live exercise is an E9 task, gated on the same deployment URL that
R-6 needs anyway (capture both in one E9 session).

---

## 6. What the orchestrator must decide before Epic 9

1. Adopt the convex crate? (Strong recommend: YES.) It supersedes FOUNDATION section 8.1's "hit the
   HTTP API directly / hand-roll WS over tokio-tungstenite" plan and section 2.2's "no native Rust SDK"
   assumption. License: Apache-2.0 — compatible with the app's GPL-3.0 distribution (Apache-2.0 is
   GPLv3-compatible). Pin convex = "0.10" in palmier-gen (and likely palmier-auth for the shared
   client). This is a binding-amendment-worthy correction to FOUNDATION — recommend recording it
   alongside the phase0 reconciliation (orchestrator owns that KB edit).
2. One shared ConvexClient or per-concern? The reference uses a SINGLE ConvexClientWithAuth in
   AccountService shared by catalog + account + generation. The crate's client is Clone (shares one
   socket). Recommend: one client owned by palmier-auth/account state, handed to palmier-gen. Decide
   ownership before E9 wires subscriptions.
3. Catalog: reactive or 24h snapshot? (Recommend snapshot via /v1/models, 3.2.) Affects whether E9
   holds a long-lived models:list subscription.
4. Secure a deployment URL + a test Clerk account for E9 (FOUNDATION Open Question section 13.9: does
   the existing deployment accept our client, or do we stand up a Windows-port backend?). E9 cannot
   reach green on live confirmation, R-6 Date capture, or the JWT-template check without it. THIS IS
   THE GATING EXTERNAL DEPENDENCY FOR EPIC 9.
5. Cancel scope (#24): confirm client-teardown-only is acceptable for v1 (job keeps billing), or
   prioritize a backend cancel mutation (out-of-repo ask).

---

## 7. Handoff to E9 (palmier-gen)

- Lift src/lib.rs (BackendGenerationStatus, BackendGenerationJob, GenerationTransport, the decode
  tests) into crates/palmier-gen/src/.
- Implement GenerationTransport over the convex crate (the convex_client.rs body is the template:
  subscribe + value_to_json bridge + terminal-status loop). Wire the Clerk callback to palmier-auth.
- Keep http_fallback.rs as the polling impl behind the same trait; default to WS, fall back on
  WS-connect failure.
- The generation LIFECYCLE (placeholders, upload, download, index-mapped finalize, cost gating) from
  generation.md sits ABOVE this transport and is transport-agnostic — port it unchanged.
- Run the section 4 live-confirmation checklist as E9-S1's first acceptance gate the moment a
  deployment URL lands; capture the R-6 Date payload in the same session.

## Timeline
2026-06-20 | Spike S-2 complete. Found the official convex Rust crate (WS sync protocol, 0.10.4); it
covers the full reference ConvexMobile surface. Built a compiling Rust client skeleton (WS primary +
HTTP polling fallback), 4 unit tests green, both bins run offline. Live round-trip deferred to E9
(deployment URL is a build secret, unreachable — same wall as S-1b). Recommend adopting the crate and
amending FOUNDATION sections 2.2/8.1.
