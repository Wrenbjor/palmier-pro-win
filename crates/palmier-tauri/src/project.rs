//! Project lifecycle orchestration: registry-backed Recent list, create/open, and
//! autosave-on-switch (story E1-S7) + the sample carousel (story E1-S8).
//!
//! This is the `palmier-tauri` orchestration layer over `palmier-project`'s
//! [`ProjectRegistry`] / [`ProjectDocument`] / [`SampleProjectService`] (the
//! filesystem + serde logic lives there; Epic 1 owns the lifecycle that calls it —
//! see docs/reference/project-io.md "Registry"/"Create"/"Open"/"Autosave" and the
//! E1-S7 / E1-S8 acceptance criteria).
//!
//! ## Managed state ([`ProjectState`])
//! A process-wide [`ProjectState`] holds the live [`ProjectRegistry`] and the
//! **active** [`ProjectDocument`] (the project whose window is foreground), behind
//! mutexes. The Home browser reads the registry; create/open mutate it; switching
//! away **force-flushes** the previous active document (FR-2's "switching
//! auto-saves the previous").
//!
//! ## Lifecycle commands (the seam `src-ui/home` calls via `invoke`)
//! - [`list_recent`] — registry `sorted_entries` (newest-first by `last_opened`).
//! - [`create_project`] — Save-As dialog (defaulting into the storage dir) → write
//!   a new empty `.palmier` bundle → `register` → open its Project window.
//! - [`open_project`] — look up the entry by id → `register` (bump `last_opened`) →
//!   set it active (flushing the previous) → open its Project window.
//! - [`open_sample`] — resolve + materialize a sample bundle (NOT registry-tracked)
//!   and open its window, reporting download progress over an event.
//! - [`delete_project`] — trash the bundle (Recycle Bin / XDG trash) + drop entry.
//! - [`return_home`] — flush the active document before showing Home.
//!
//! ## Autosave-on-switch (reference `AppState.showHome`)
//! The active document is force-flushed (`flush_if_dirty`) before another project
//! is made active OR before returning Home, mirroring the reference
//! "autosave-before-ordering-out". Because the editor's live timeline is not owned
//! here (Epic 5/6), a flush uses the **last-saved** [`BundleSnapshot`] retained with
//! the active doc — on first-class editor integration this snapshot is replaced by
//! the live view-model snapshot. The dirty-flag + force-flush plumbing is real and
//! tested; what a flush writes is the timeline the create/open seeded (a no-op
//! until the editor marks edits), so navigation never loses a registered project.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tauri_plugin_dialog::DialogExt;

use palmier_project::{
    read_bundle, BundleSnapshot, ProjectDocument, ProjectEntry, ProjectRegistry,
    SampleProjectService,
};
use palmier_model::Timeline;

use crate::window;

/// The reference default project name (`Constants.swift Project.defaultProjectName`).
pub const DEFAULT_PROJECT_NAME: &str = "Untitled Project";

/// Process-wide project lifecycle state (managed by Tauri).
///
/// Holds the registry and the **active** document + its last-saved snapshot. All
/// fields are behind a `Mutex` so commands can mutate them from any thread.
pub struct ProjectState {
    /// The recent-projects registry (`project-registry.json`).
    registry: Mutex<ProjectRegistry>,
    /// The currently-active project (foreground window), if any, paired with the
    /// last snapshot written for it (used by the force-flush-on-switch).
    active: Mutex<Option<ActiveProject>>,
    /// The sample service (None when no Convex HTTP URL is configured ⇒ empty
    /// carousel, offline-safe).
    samples: Option<SampleProjectService>,
}

/// The active project: its document (path + dirty flag + autosave debounce) and the
/// snapshot to write on a force-flush.
struct ActiveProject {
    /// Entry id (the Project window label suffix).
    id: String,
    document: ProjectDocument,
    snapshot: BundleSnapshot,
}

impl ProjectState {
    /// Build the state from a loaded registry and an optional sample service.
    pub fn new(registry: ProjectRegistry, samples: Option<SampleProjectService>) -> Self {
        Self {
            registry: Mutex::new(registry),
            active: Mutex::new(None),
            samples,
        }
    }

    /// Autonomous-test affordance: which project (if any) to auto-open at boot, from env.
    /// `PALMIER_OPEN_PROJECT=<id>` opens that registry entry; `PALMIER_OPEN_FIRST_RECENT=1`
    /// opens the most-recently-opened entry. Returns `None` in the normal case (neither
    /// set ⇒ boot shows only Home). Lets the orchestrator drive the editor without a click.
    pub fn boot_open_id(&self) -> Option<String> {
        let reg = self.registry.lock().expect("registry mutex");
        if let Ok(id) = std::env::var("PALMIER_OPEN_PROJECT") {
            let id = id.trim();
            if !id.is_empty() && reg.sorted_entries().iter().any(|e| e.id.to_string() == id) {
                return Some(id.to_string());
            }
        }
        if std::env::var("PALMIER_OPEN_FIRST_RECENT").is_ok() {
            return reg.sorted_entries().first().map(|e| e.id.to_string());
        }
        None
    }
}

/// Build the managed [`ProjectState`] from the booted [`palmier_auth::Auth`].
///
/// Loads the registry from `project-registry.json` (missing ⇒ empty, lenient) and
/// attaches a [`SampleProjectService`] only when a Convex HTTP URL is configured —
/// otherwise the carousel is empty and offline-safe (OQ-9 / R-4). A transport-build
/// failure also degrades to no sample service rather than failing boot.
pub fn build_state(auth: &palmier_auth::Auth) -> ProjectState {
    let registry = ProjectRegistry::load().unwrap_or_else(|e| {
        tracing::warn!(target: "project", error = %e, "no registry path; using an in-memory registry");
        // Fall back to a registry under the OS temp dir so the app still runs.
        ProjectRegistry::with_path(std::env::temp_dir().join("palmier-project-registry.json"))
    });

    let samples = sample_service(auth);
    ProjectState::new(registry, samples)
}

/// Build the [`SampleProjectService`] from the auth config's Convex HTTP URL, if
/// present. `None` ⇒ no carousel (offline-safe).
fn sample_service(auth: &palmier_auth::Auth) -> Option<SampleProjectService> {
    let http_url = auth.config().convex_http_url.as_ref()?.to_string();
    let cache_root = SampleProjectService::default_cache_root()?;
    match palmier_project::HttpSampleBackend::new(http_url) {
        Ok(backend) => {
            tracing::debug!(target: "project", "sample service configured (Convex HTTP)");
            Some(SampleProjectService::new(Box::new(backend), cache_root))
        }
        Err(e) => {
            tracing::warn!(target: "project", error = %e, "sample transport build failed; empty carousel");
            None
        }
    }
}

/// A Recent-list row for the Home browser (mirrors the frontend `RecentProject`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    /// Registry entry id (passed back to [`open_project`]).
    pub id: String,
    /// Display title (bundle filename minus `.palmier`).
    pub title: String,
    /// The bundle path (diagnostics / tooltip).
    pub path: String,
    /// Last-opened timestamp as Unix seconds (the sort key; newest-first).
    pub last_opened: i64,
    /// Whether the bundle still exists on disk.
    pub accessible: bool,
}

impl RecentProject {
    fn from_entry(e: &ProjectEntry) -> Self {
        RecentProject {
            id: e.id.to_string(),
            title: e.name(),
            path: e.url.to_string_lossy().to_string(),
            last_opened: e.last_opened_date.unix_timestamp(),
            accessible: e.is_accessible(),
        }
    }
}

/// A sample-carousel row (mirrors the frontend `SampleSummary`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleCard {
    pub slug: String,
    pub title: String,
    pub poster_url: Option<String>,
}

/// Sample download-progress event payload (`sample://progress`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleProgress {
    pub slug: String,
    /// 0.0..=1.0.
    pub progress: f64,
}

// ─── Recent list (E1-S7) ─────────────────────────────────────────────────────

/// The recent projects, **newest-first** by `last_opened_date` (FR-2).
#[tauri::command]
pub fn list_recent(state: State<'_, ProjectState>) -> Vec<RecentProject> {
    let reg = state.registry.lock().expect("registry mutex");
    reg.sorted_entries()
        .iter()
        .map(RecentProject::from_entry)
        .collect()
}

// ─── Create (E1-S7) ──────────────────────────────────────────────────────────

/// Create a new project (File → New / Home "New Project").
///
/// Opens the **Save-As dialog** (reference `NSSavePanel`, defaulting into
/// `~/Documents/Palmier Pro` with name "Untitled Project", filtered to `.palmier`),
/// writes a new empty `.palmier` bundle at the chosen path, registers it, makes it
/// active (flushing any previous), and opens its Project window.
///
/// Returns the new entry id, or `None` when the user **cancels** the dialog (the
/// Home UI treats `None` as a no-op, not an error).
#[tauri::command]
pub fn create_project<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, ProjectState>,
) -> Result<Option<String>, String> {
    // Save-As dialog (reference NSSavePanel): default dir + name, `.palmier` filter.
    let Some(path) = prompt_save_bundle(&app) else {
        return Ok(None); // user cancelled
    };
    let path = normalize_bundle_path(&path.to_string_lossy());

    // Build + write a new empty project bundle (Timeline with reference defaults).
    let snapshot = BundleSnapshot::new(Timeline::new());
    palmier_project::write_bundle(&path, &snapshot).map_err(|e| e.to_string())?;

    // Register it (new entry, created+lastOpened = now), then read back its id.
    let id = {
        let mut reg = state.registry.lock().expect("registry mutex");
        reg.register(&path).map_err(|e| e.to_string())?;
        entry_id_for_path(&reg, &path).ok_or("registered project not found")?
    };

    set_active(&app, &state, &id, path.clone(), snapshot)?;
    window::open_project_window(&app, &id).map_err(|e| e.to_string())?;
    tracing::info!(target: "project", id = %id, path = %path.display(), "created project");
    Ok(Some(id))
}

/// Open an existing project via the **Open dialog** (File → Open / Home "Open…").
///
/// Opens the `.palmier` open dialog (reference `NSOpenPanel`), reads the chosen
/// bundle, registers it (bump `last_opened`), makes it active (flushing the
/// previous), and opens its window. Returns the entry id, or `None` on cancel.
#[tauri::command]
pub fn open_project_dialog<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, ProjectState>,
) -> Result<Option<String>, String> {
    let Some(path) = prompt_open_bundle(&app) else {
        return Ok(None);
    };

    // Read the bundle (open round-trips, FR-2).
    let loaded = read_bundle(&path).map_err(|e| e.to_string())?;
    let mut snapshot = BundleSnapshot::new(loaded.timeline);
    snapshot.manifest = loaded.manifest;
    snapshot.generation_log = loaded.generation_log;

    let id = {
        let mut reg = state.registry.lock().expect("registry mutex");
        reg.register(&path).map_err(|e| e.to_string())?;
        entry_id_for_path(&reg, &path).ok_or("registered project not found")?
    };

    set_active(&app, &state, &id, path, snapshot)?;
    window::open_project_window(&app, &id).map_err(|e| e.to_string())?;
    tracing::info!(target: "project", id = %id, "opened project from dialog");
    Ok(Some(id))
}

// ─── Open (E1-S7) ────────────────────────────────────────────────────────────

/// Open an existing registered project by its **entry id** (Recent list click):
/// bump `last_opened`, make it active (flushing the previous), and open its window.
///
/// The bundle is read to seed the active snapshot; a corrupt/missing bundle is a
/// surfaced error (the Recent row shows it as inaccessible). Replaces E1-S4's
/// window-only stub with the real registry-backed open.
#[tauri::command]
pub fn open_project<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, ProjectState>,
    project_id: String,
) -> Result<(), String> {
    let path = {
        let reg = state.registry.lock().expect("registry mutex");
        path_for_entry_id(&reg, &project_id).ok_or("unknown project id")?
    };

    // Read the bundle to seed the active snapshot (open round-trips, FR-2).
    let loaded = read_bundle(&path).map_err(|e| e.to_string())?;
    let mut snapshot = BundleSnapshot::new(loaded.timeline);
    snapshot.manifest = loaded.manifest;
    snapshot.generation_log = loaded.generation_log;

    // Bump last_opened (re-register the existing path).
    {
        let mut reg = state.registry.lock().expect("registry mutex");
        reg.register(&path).map_err(|e| e.to_string())?;
    }

    set_active(&app, &state, &project_id, path, snapshot)?;
    window::open_project_window(&app, &project_id).map_err(|e| e.to_string())?;
    tracing::info!(target: "project", id = %project_id, "opened project");
    Ok(())
}

// ─── Delete (E1-S7) ──────────────────────────────────────────────────────────

/// Delete a project by entry id: trash the bundle (Recycle Bin / XDG trash) then
/// drop the registry entry (reference `ProjectRegistry.delete`).
#[tauri::command]
pub fn delete_project(state: State<'_, ProjectState>, project_id: String) -> Result<(), String> {
    let mut reg = state.registry.lock().expect("registry mutex");
    let path = path_for_entry_id(&reg, &project_id).ok_or("unknown project id")?;
    reg.delete(&path).map_err(|e| e.to_string())?;
    tracing::info!(target: "project", id = %project_id, "deleted project (trashed bundle)");
    Ok(())
}

// ─── Return Home / autosave-on-switch (E1-S7) ────────────────────────────────

/// Return to Home, **force-flushing the active project first** (reference
/// `AppState.showHome`: autosave-before-hide). This is FR-2's "switching
/// auto-saves the previous". Keeps the `show_home` command name the frontend +
/// menu router already call (replaces E1-S4's window-only stub).
#[tauri::command]
pub fn show_home<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, ProjectState>,
) -> Result<(), String> {
    flush_active(&app, &state)?;
    // E8-S7: clear the agent panel's session state (the next project open reloads
    // its own chat/). No project active ⇒ no chat tabs.
    if let Some(agent) = app.try_state::<crate::agent::AgentState>() {
        agent.clear_sessions();
    }
    window::show_home(&app).map_err(|e| e.to_string())
}

// ─── Samples (E1-S8) ─────────────────────────────────────────────────────────

/// The sample carousel summaries. **Degrades to an empty list** when no Convex HTTP
/// URL is configured OR the catalog fetch fails (offline) — never an error
/// (OQ-9 / R-4).
#[tauri::command]
pub fn list_samples(state: State<'_, ProjectState>) -> Vec<SampleCard> {
    let Some(svc) = state.samples.as_ref() else {
        tracing::debug!(target: "project", "no sample service configured; empty carousel");
        return Vec::new();
    };
    match svc.list() {
        Ok(list) => list
            .into_iter()
            .map(|s| SampleCard {
                slug: s.slug,
                title: s.title,
                poster_url: s.poster_url,
            })
            .collect(),
        Err(e) => {
            // Offline / unreachable Convex ⇒ empty carousel, logged not surfaced.
            tracing::info!(target: "project", error = %e, "samples unavailable; empty carousel (offline-safe)");
            Vec::new()
        }
    }
}

/// Resolve + materialize a sample bundle and open it (NOT registry-tracked, FR-4).
/// Download progress is emitted over the `sample://progress` event as
/// `completed/total`. A failure (resolve/download) is surfaced to the carousel.
#[tauri::command]
pub fn open_sample<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, ProjectState>,
    slug: String,
) -> Result<(), String> {
    let Some(svc) = state.samples.as_ref() else {
        return Err("Samples are unavailable: backend not configured.".to_string());
    };

    // Materialize with progress → window. Use a sample-keyed window id so a sample
    // gets its own window like any project, but it is never registered.
    let app_for_progress = app.clone();
    let slug_for_progress = slug.clone();
    let bundle = svc
        .materialize(&slug, move |p| {
            let _ = app_for_progress.emit(
                "sample://progress",
                SampleProgress {
                    slug: slug_for_progress.clone(),
                    progress: p,
                },
            );
        })
        .map_err(|e| e.to_string())?;

    // Read it to seed the active snapshot (samples round-trip, FR-4) — but do NOT
    // register (reference `openProject(register:false)`).
    let loaded = read_bundle(&bundle).map_err(|e| e.to_string())?;
    let mut snapshot = BundleSnapshot::new(loaded.timeline);
    snapshot.manifest = loaded.manifest;
    snapshot.generation_log = loaded.generation_log;

    let window_id = format!("sample-{}", palmier_project::safe_name(&slug));
    set_active(&app, &state, &window_id, bundle, snapshot)?;
    window::open_project_window(&app, &window_id).map_err(|e| e.to_string())?;
    tracing::info!(target: "project", slug = %slug, "opened sample (not registry-tracked)");
    Ok(())
}

// ─── internals ───────────────────────────────────────────────────────────────

/// Make `id` the active project, **flushing the previously-active one first**
/// (force-flush-on-switch — reference autosave-before-ordering-out).
///
/// E8-S7: after the previous project is flushed and the new one is installed, load
/// the new project's chat sessions ([`AgentState::load_project_sessions`]) so the
/// agent panel's tab bar / history shows that project's prior chats (newest-first,
/// with a fresh empty current tab prepended — reference `loadSessions`).
fn set_active<R: Runtime>(
    app: &AppHandle<R>,
    state: &State<'_, ProjectState>,
    id: &str,
    path: PathBuf,
    snapshot: BundleSnapshot,
) -> Result<(), String> {
    // Flush the previous active project before replacing it (also captures its
    // chat into the bundle).
    flush_active(app, state)?;
    {
        let mut active = state.active.lock().expect("active mutex");
        *active = Some(ActiveProject {
            id: id.to_string(),
            document: ProjectDocument::new(Some(path.clone())),
            snapshot,
        });
    }
    // Load the new project's chat sessions into the agent panel (offline-safe: a
    // missing chat/ dir ⇒ just the fresh current tab).
    if let Some(agent) = app.try_state::<crate::agent::AgentState>() {
        agent.load_project_sessions(&path);
    }
    Ok(())
}

/// Force-flush the active document if dirty (no-op when clean / none active).
///
/// E8-S7: before flushing, refresh the active snapshot's `chat_files` from the
/// agent's tab sessions ([`AgentState::capture_chat_snapshot`]) so the chat is
/// persisted into the bundle's `chat/` dir on this save (ruling #4 — chat writes
/// on document save). `app` is the handle used to reach the `AgentState`.
fn flush_active<R: Runtime>(
    app: &AppHandle<R>,
    state: &State<'_, ProjectState>,
) -> Result<(), String> {
    let mut active = state.active.lock().expect("active mutex");
    if let Some(ap) = active.as_mut() {
        // Pull the latest chat snapshot into the bundle snapshot before saving.
        if let Some(agent) = app.try_state::<crate::agent::AgentState>() {
            ap.snapshot.chat_files = agent.capture_chat_snapshot();
        }
        let flushed = ap
            .document
            .flush_if_dirty(&ap.snapshot)
            .map_err(|e| e.to_string())?;
        if flushed {
            tracing::info!(target: "project", id = %ap.id, "autosaved active project before switch");
        }
    }
    Ok(())
}

/// The active project's bundle root (`.palmier` dir), if a project is open. The
/// chat sessions live under `<root>/chat/` (E8-S7).
pub fn active_project_root<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    let state = app.try_state::<ProjectState>()?;
    let active = state.active.lock().expect("active mutex");
    active.as_ref().and_then(|ap| ap.document.path().map(Path::to_path_buf))
}

/// Mark the active document dirty because a **chat session changed** (ruling #4 —
/// the reference `agentService.onSessionsChanged → updateChangeCount(.changeDone)`
/// marks the doc dirty so the chat persists on the next save, NOT eagerly). A
/// no-op when no project is active. Called by the agent tab commands + after each
/// turn.
pub fn mark_chat_dirty<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<ProjectState>() else {
        return;
    };
    let mut active = state.active.lock().expect("active mutex");
    if let Some(ap) = active.as_mut() {
        ap.document.mark_chat_changed();
    }
}

/// Open the Save-As dialog for a new `.palmier` bundle (reference `NSSavePanel`):
/// default into `~/Documents/Palmier Pro` with the file name "Untitled Project" and
/// a `.palmier` filter. Returns the chosen path, or `None` on cancel.
fn prompt_save_bundle<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    let mut builder = app
        .dialog()
        .file()
        .add_filter("Palmier Project", &[palmier_project::project::FILE_EXTENSION])
        .set_file_name(DEFAULT_PROJECT_NAME);
    if let Some(dir) = storage_dir() {
        builder = builder.set_directory(dir);
    }
    builder.blocking_save_file().and_then(|p| p.into_path().ok())
}

/// Open the Open dialog for an existing `.palmier` bundle (reference `NSOpenPanel`).
/// Returns the chosen path, or `None` on cancel.
fn prompt_open_bundle<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    let mut builder = app
        .dialog()
        .file()
        .add_filter("Palmier Project", &[palmier_project::project::FILE_EXTENSION]);
    if let Some(dir) = storage_dir() {
        builder = builder.set_directory(dir);
    }
    builder.blocking_pick_file().and_then(|p| p.into_path().ok())
}

/// The storage directory the dialogs default into (`~/Documents/Palmier Pro`),
/// created on demand (reference `Project.storageDirectory`).
fn storage_dir() -> Option<PathBuf> {
    let dir = dirs::document_dir()?.join("Palmier Pro");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

/// Normalize the user-chosen bundle path: ensure it ends in `.palmier` (the Save-As
/// dialog should enforce this, but be defensive).
fn normalize_bundle_path(raw: &str) -> PathBuf {
    let p = PathBuf::from(raw);
    if p.extension().and_then(|s| s.to_str()) == Some(palmier_project::project::FILE_EXTENSION) {
        p
    } else {
        let mut s = raw.to_string();
        s.push('.');
        s.push_str(palmier_project::project::FILE_EXTENSION);
        PathBuf::from(s)
    }
}

/// Find the entry id (as a string) for a path in the registry.
fn entry_id_for_path(reg: &ProjectRegistry, path: &Path) -> Option<String> {
    let key = palmier_project::normalize_path(path);
    reg.entries()
        .iter()
        .find(|e| palmier_project::normalize_path(&e.url) == key)
        .map(|e| e.id.to_string())
}

/// Find the bundle path for an entry id.
fn path_for_entry_id(reg: &ProjectRegistry, id: &str) -> Option<PathBuf> {
    reg.entries()
        .iter()
        .find(|e| e.id.to_string() == id)
        .map(|e| e.url.clone())
}

/// The default storage directory the New dialog opens into
/// (`~/Documents/Palmier Pro`, reference `Project.storageDirectory`). Created on
/// demand. Exposed to the frontend for diagnostics / display.
#[tauri::command]
pub fn default_storage_dir() -> Option<String> {
    storage_dir().map(|d| d.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use palmier_project::FixtureSampleBackend;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A unique scratch dir under the OS temp dir (no `uuid` dep in this crate; a
    /// process-pid + monotonic counter is unique enough for serial unit tests).
    fn scratch() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!(
            "palmier-e1s7-{}-{}",
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn state_with_registry(dir: &Path) -> ProjectState {
        let reg = ProjectRegistry::with_path(dir.join("project-registry.json"));
        ProjectState::new(reg, None)
    }

    #[test]
    fn recent_row_maps_entry_fields() {
        let dir = scratch();
        let mut reg = ProjectRegistry::with_path(dir.join("project-registry.json"));
        let bundle = dir.join("My Movie.palmier");
        std::fs::create_dir_all(&bundle).unwrap();
        reg.register(&bundle).unwrap();
        let row = RecentProject::from_entry(&reg.entries()[0]);
        assert_eq!(row.title, "My Movie");
        assert!(row.accessible);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn entry_id_path_round_trips() {
        let dir = scratch();
        let mut reg = ProjectRegistry::with_path(dir.join("project-registry.json"));
        let bundle = dir.join("Proj.palmier");
        reg.register(&bundle).unwrap();
        let id = entry_id_for_path(&reg, &bundle).unwrap();
        let back = path_for_entry_id(&reg, &id).unwrap();
        assert_eq!(palmier_project::normalize_path(&back), palmier_project::normalize_path(&bundle));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn normalize_bundle_path_adds_extension() {
        assert_eq!(
            normalize_bundle_path("/x/My Project"),
            PathBuf::from("/x/My Project.palmier")
        );
        assert_eq!(
            normalize_bundle_path("/x/Already.palmier"),
            PathBuf::from("/x/Already.palmier")
        );
    }

    #[test]
    fn list_samples_empty_without_service() {
        // No sample service ⇒ empty carousel (offline-safe), no panic.
        let dir = scratch();
        let _state = state_with_registry(&dir);
        // Can't build a `State` outside Tauri; assert the service-None branch via the
        // service directly: a state with no samples returns empty.
        assert!(_state.samples.is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sample_service_offline_lists_empty_via_fixture() {
        // The list_samples degradation is exercised through the service: an offline
        // fixture backend errors, and the command maps that to an empty vec.
        let dir = scratch();
        let backend = FixtureSampleBackend {
            offline: true,
            ..Default::default()
        };
        let svc = SampleProjectService::new(Box::new(backend), dir.join("Samples"));
        assert!(svc.list().is_err(), "offline ⇒ Err; command maps to empty carousel");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
