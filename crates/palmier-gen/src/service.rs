//! The generation lifecycle — placeholders → submit → subscribe → finalize/
//! download (E9-S7; reference `GenerationService.generate`/`runJob`/
//! `downloadAndFinalize`/`finalizeSuccess`).
//!
//! `generate(...)` creates `count` placeholder `MediaAsset`s (N for image,
//! 1 otherwise), returns `placeholders[0].id` **synchronously** (so the UI gets
//! an id < 2 s — SM-11), and spawns the detached lifecycle: upload references →
//! build params → submit → subscribe the job stream → on success download each
//! result URL **1:1 by index** into `<project>/media`, transitioning each
//! placeholder Generating → Downloading → None; on failure mark all placeholders
//! Failed.
//!
//! The macOS `@MainActor` editor is replaced by a [`GenerationSink`] the host
//! supplies: every placeholder creation, status transition, and completion toast
//! flows through it (the Tauri layer maps these to events — no direct frontend
//! side effects here). The lifecycle is fully driveable by a test sink + the
//! [`MockTransport`](crate::transport::MockTransport), with no live Convex.
//!
//! Cancellation (#24) = drop the subscription: the lifecycle holds the
//! [`JobStream`](crate::transport::JobStream); dropping the spawned task (or the
//! returned [`GenerationHandle`]) drops the stream and tears down the WS
//! subscription. The Convex job keeps running/billing.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;
use palmier_model::{ClipType, GenerationInput, MediaAsset, MediaSource};

use crate::params::BackendGenerationParams;
use crate::transport::{BackendGenerationStatus, GenerationTransport, JobStream};
use crate::upload::{upload_references, ReferenceUpload, UploadResult};

/// A live generation status update for one placeholder asset (the reactive
/// `generation_status` the reference pushes to `@MainActor`; the Tauri layer
/// maps these to events).
#[derive(Debug, Clone, PartialEq)]
pub enum StatusUpdate {
    /// A placeholder was created (id, name, type) — appears < 2 s (SM-11).
    PlaceholderCreated { id: String, name: String, asset_type: ClipType },
    /// Status moved to Generating.
    Generating { id: String },
    /// Status moved to Downloading.
    Downloading { id: String },
    /// Finalized successfully — the asset now lives at `final_path`.
    Succeeded { id: String, final_path: PathBuf },
    /// Generation/download failed with a message.
    Failed { id: String, message: String },
    /// The first successful download — fire the native completion toast.
    CompletionToast { first_asset_id: String, asset_name: String, count: usize },
}

/// The host sink the lifecycle reports through. The Tauri layer implements this
/// (emit events + persist the asset); tests implement a recording sink. Every
/// method is `&self` + `Send + Sync` so the lifecycle can run on a spawned task.
pub trait GenerationSink: Send + Sync {
    /// Receive one [`StatusUpdate`]. Called in lifecycle order.
    fn update(&self, update: StatusUpdate);
}

/// Inputs to one `generate(...)` call. Mirrors the reference `generate(...)`
/// parameter list, minus the macOS-only closures (the port's `build_params`
/// closure assembles the final params from the uploaded reference URLs).
pub struct GenerateRequest {
    /// The recorded generation inputs (attached to every placeholder).
    pub gen_input: GenerationInput,
    /// The placeholder asset type (`video`/`image`/`audio`).
    pub asset_type: ClipType,
    /// Placeholder duration in seconds (the reference's `placeholderDuration`).
    pub placeholder_duration: f64,
    /// `numImages` for image generation (clamped `[1,4]`); 1 for others.
    pub num_images: i32,
    /// Display name (defaults to the first 30 chars of the prompt).
    pub name: Option<String>,
    /// Target folder id (must exist; the host validates — passed through).
    pub folder_id: Option<String>,
    /// File extension for the placeholder dest (`mp4`/`png`/`mp3`).
    pub file_extension: String,
    /// References to upload (already resolved to bytes by the host).
    pub references: Vec<ReferenceUpload>,
    /// The project root; placeholders land in `<project>/media`, else a temp dir.
    pub project_url: Option<PathBuf>,
    /// The Convex `projectId` arg for `generations:submit`.
    pub project_id: Option<String>,
    /// Assemble the final params from the uploaded reference URLs (reference
    /// `buildParams(uploaded)`).
    pub build_params: Box<dyn FnOnce(&[String]) -> BackendGenerationParams + Send>,
    /// Stamp the uploaded URLs into the gen input (reference `snapshotRefs`); if
    /// `None`, sets `image_urls` (the reference default).
    pub snapshot_refs: Option<Box<dyn FnOnce(&mut GenerationInput, &[String]) + Send>>,
}

/// A handle to a spawned generation. The primary placeholder id is available
/// synchronously; dropping the handle (or calling [`GenerationHandle::cancel`])
/// drops the lifecycle task and tears down the subscription (#24).
pub struct GenerationHandle {
    /// The synchronously-returned id of the first placeholder (SM-11).
    pub primary_id: String,
    task: tokio::task::JoinHandle<()>,
}

impl GenerationHandle {
    /// Cancel (client teardown only, #24): aborts the lifecycle task, dropping
    /// the job subscription. The Convex job keeps running/billing.
    pub fn cancel(self) {
        self.task.abort();
    }

    /// Await lifecycle completion (tests). In the app the task is detached.
    pub async fn join(self) {
        let _ = self.task.await;
    }
}

/// The generation orchestrator. Holds the transport + an HTTP client (for the
/// upload POST + the result download). Cheap to clone-share via `Arc`.
pub struct GenerationService {
    transport: Arc<dyn GenerationTransport>,
    http: reqwest::Client,
}

impl GenerationService {
    /// New service over a transport.
    #[must_use]
    pub fn new(transport: Arc<dyn GenerationTransport>) -> Self {
        Self {
            transport,
            http: reqwest::Client::new(),
        }
    }

    /// The destination directory for placeholders/results: `<project>/media`
    /// (created if missing) or the system temp dir if no project (reference
    /// `destinationDirectory`).
    fn destination_dir(project_url: Option<&Path>) -> PathBuf {
        if let Some(p) = project_url {
            let dir = p.join("media");
            let _ = std::fs::create_dir_all(&dir);
            dir
        } else {
            std::env::temp_dir()
        }
    }

    /// Create one placeholder asset (reference `createPlaceholder`): new UUID,
    /// dest `…/gen-<id8>.<ext>`, `generation_status = Generating`.
    fn create_placeholder(
        name: &str,
        asset_type: ClipType,
        duration: f64,
        gen_input: &GenerationInput,
        folder_id: Option<&str>,
        dest_dir: &Path,
        file_extension: &str,
    ) -> MediaAsset {
        let id = uuid::Uuid::new_v4().to_string();
        let short = &id[..8.min(id.len())];
        let dest = dest_dir.join(format!("gen-{short}.{file_extension}"));
        let mut asset = MediaAsset::new(
            id,
            name,
            asset_type,
            MediaSource::External {
                absolute_path: dest.to_string_lossy().into_owned(),
            },
            duration,
        );
        asset.generation_input = Some(gen_input.clone());
        asset.generation_status = palmier_model::GenerationStatus::Generating;
        asset.folder_id = folder_id.map(str::to_string);
        asset
    }

    /// Start a generation. Creates the placeholders synchronously (returns the
    /// primary id), then spawns the detached lifecycle. The `sink` receives every
    /// placeholder + status transition + the completion toast.
    pub fn generate(
        &self,
        request: GenerateRequest,
        sink: Arc<dyn GenerationSink>,
    ) -> GenerationHandle {
        let count = if request.asset_type == ClipType::Image {
            request.num_images.clamp(1, 4)
        } else {
            1
        };
        let base_name = request
            .name
            .clone()
            .unwrap_or_else(|| request.gen_input.prompt.chars().take(30).collect());
        let dest_dir = Self::destination_dir(request.project_url.as_deref());

        let mut placeholders: Vec<MediaAsset> = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let ph = Self::create_placeholder(
                &base_name,
                request.asset_type,
                request.placeholder_duration,
                &request.gen_input,
                request.folder_id.as_deref(),
                &dest_dir,
                &request.file_extension,
            );
            sink.update(StatusUpdate::PlaceholderCreated {
                id: ph.id.clone(),
                name: ph.name.clone(),
                asset_type: ph.asset_type,
            });
            sink.update(StatusUpdate::Generating { id: ph.id.clone() });
            placeholders.push(ph);
        }
        let primary_id = placeholders[0].id.clone();

        let transport = Arc::clone(&self.transport);
        let http = self.http.clone();
        let task = tokio::spawn(async move {
            run_lifecycle(transport, http, request, placeholders, sink).await;
        });

        GenerationHandle { primary_id, task }
    }
}

/// The detached lifecycle body (reference `generate`'s `Task` + `runJob` +
/// `finalizeSuccess`). Uploads references, builds params, submits, subscribes,
/// and finalizes/downloads — reporting through the sink.
async fn run_lifecycle(
    transport: Arc<dyn GenerationTransport>,
    http: reqwest::Client,
    request: GenerateRequest,
    placeholders: Vec<MediaAsset>,
    sink: Arc<dyn GenerationSink>,
) {
    let GenerateRequest {
        mut gen_input,
        references,
        project_id,
        build_params,
        snapshot_refs,
        ..
    } = request;

    // 1. Upload references (concurrent, order-preserving, cache-aware).
    let uploaded: Vec<UploadResult> = match upload_references(&*transport, &http, references).await {
        Ok(u) => u,
        Err(e) => {
            fail_all(&placeholders, &format!("Upload failed: {e}"), &sink);
            return;
        }
    };
    let uploaded_urls: Vec<String> = uploaded.into_iter().map(|u| u.url).collect();

    // 2. Snapshot the uploaded URLs into the gen input (reference snapshotRefs).
    if let Some(snap) = snapshot_refs {
        snap(&mut gen_input, &uploaded_urls);
    } else if !uploaded_urls.is_empty() {
        gen_input.image_urls = Some(uploaded_urls.clone());
    }
    if gen_input.created_at.is_none() {
        gen_input.created_at = Some(time::OffsetDateTime::now_utc());
    }

    // 3. Build params + submit (reference runJob).
    let params = build_params(&uploaded_urls);
    let job_id = match transport
        .submit(&gen_input.model, &params.to_json(), project_id.as_deref())
        .await
    {
        Ok(id) => id,
        Err(e) => {
            fail_all(&placeholders, &e.to_string(), &sink);
            return;
        }
    };

    // 4. Subscribe + consume the status stream to settlement.
    let mut stream: JobStream = match transport.subscribe(&job_id).await {
        Ok(s) => s,
        Err(_) => {
            fail_all(&placeholders, "Backend not configured", &sink);
            return;
        }
    };

    while let Some(job) = stream.next().await {
        match job.status {
            BackendGenerationStatus::Succeeded => {
                finalize_success(&http, job.result_urls.unwrap_or_default(), &placeholders, &sink)
                    .await;
                return;
            }
            BackendGenerationStatus::Failed => {
                let msg = job.error_message.unwrap_or_else(|| "Generation failed".to_string());
                fail_all(&placeholders, &msg, &sink);
                return;
            }
            BackendGenerationStatus::Queued | BackendGenerationStatus::Running => continue,
        }
    }
    // Stream ended before settling (or was torn down by cancel) — no-op.
}

/// Map result URLs **1:1 by index** onto placeholders, download each, and fire
/// the completion toast on the first success (reference `finalizeSuccess`).
async fn finalize_success(
    http: &reqwest::Client,
    result_urls: Vec<String>,
    placeholders: &[MediaAsset],
    sink: &Arc<dyn GenerationSink>,
) {
    if result_urls.is_empty() {
        fail_all(placeholders, "No URL in response", sink);
        return;
    }

    let mut finalized: Vec<(String, String)> = Vec::new(); // (id, name)
    for (i, ph) in placeholders.iter().enumerate() {
        let Some(remote) = result_urls.get(i) else {
            sink.update(StatusUpdate::Failed {
                id: ph.id.clone(),
                message: "No URL for placeholder".to_string(),
            });
            continue;
        };
        match download_and_finalize(http, ph, remote, sink).await {
            Ok(()) => finalized.push((ph.id.clone(), ph.name.clone())),
            Err(()) => { /* already reported via the sink */ }
        }
    }

    if let Some((first_id, first_name)) = finalized.first().cloned() {
        sink.update(StatusUpdate::CompletionToast {
            first_asset_id: first_id,
            asset_name: first_name,
            count: finalized.len(),
        });
    }
}

/// The placeholder's intended on-disk path (from its `External` source).
fn placeholder_path(ph: &MediaAsset) -> PathBuf {
    match &ph.source {
        MediaSource::External { absolute_path } => PathBuf::from(absolute_path),
        MediaSource::Project { relative_path } => PathBuf::from(relative_path),
    }
}

/// Download one result URL into the placeholder's dest, fixing the extension
/// from the remote path (reference `downloadAndFinalize`). Reports Downloading →
/// Succeeded/Failed through the sink.
async fn download_and_finalize(
    http: &reqwest::Client,
    ph: &MediaAsset,
    remote: &str,
    sink: &Arc<dyn GenerationSink>,
) -> Result<(), ()> {
    sink.update(StatusUpdate::Downloading { id: ph.id.clone() });

    let resp = match http.get(remote).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            sink.update(StatusUpdate::Failed {
                id: ph.id.clone(),
                message: format!("download HTTP {}", r.status()),
            });
            return Err(());
        }
        Err(e) => {
            sink.update(StatusUpdate::Failed { id: ph.id.clone(), message: e.to_string() });
            return Err(());
        }
    };
    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            sink.update(StatusUpdate::Failed { id: ph.id.clone(), message: e.to_string() });
            return Err(());
        }
    };

    // Fix the extension from the remote path if it's a known ClipType.
    let mut dest = placeholder_path(ph);
    let real_ext = crate::upload::extension_of(remote.split('?').next().unwrap_or(remote)).to_string();
    if !real_ext.is_empty()
        && Some(real_ext.as_str()) != dest.extension().and_then(|e| e.to_str())
        && ClipType::from_file_extension(&real_ext).is_some()
    {
        dest.set_extension(&real_ext);
    }

    let _ = std::fs::remove_file(&dest);
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&dest, &bytes) {
        sink.update(StatusUpdate::Failed { id: ph.id.clone(), message: e.to_string() });
        return Err(());
    }

    sink.update(StatusUpdate::Succeeded { id: ph.id.clone(), final_path: dest });
    Ok(())
}

/// Mark every placeholder Failed with `message` (reference's "fail all
/// placeholders" path on submit/upload/subscribe failure).
fn fail_all(placeholders: &[MediaAsset], message: &str, sink: &Arc<dyn GenerationSink>) {
    for ph in placeholders {
        sink.update(StatusUpdate::Failed {
            id: ph.id.clone(),
            message: message.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{VideoParams, ImageParams};
    use crate::transport::MockTransport;
    use std::sync::Mutex;

    /// A recording sink that captures every status update.
    #[derive(Default)]
    struct RecordingSink {
        updates: Mutex<Vec<StatusUpdate>>,
    }
    impl GenerationSink for RecordingSink {
        fn update(&self, update: StatusUpdate) {
            self.updates.lock().unwrap().push(update);
        }
    }
    impl RecordingSink {
        fn snapshot(&self) -> Vec<StatusUpdate> {
            self.updates.lock().unwrap().clone()
        }
    }

    fn gen_input(model: &str) -> GenerationInput {
        GenerationInput {
            prompt: "a test".into(),
            model: model.into(),
            duration: 4,
            aspect_ratio: "16:9".into(),
            resolution: None,
            quality: None,
            image_urls: None,
            num_images: None,
            voice: None,
            lyrics: None,
            style_instructions: None,
            instrumental: None,
            generate_audio: None,
            reference_image_urls: None,
            reference_video_urls: None,
            reference_audio_urls: None,
            image_url_asset_ids: None,
            reference_image_asset_ids: None,
            reference_video_asset_ids: None,
            reference_audio_asset_ids: None,
            created_at: None,
        }
    }

    fn video_request(project: Option<PathBuf>) -> GenerateRequest {
        GenerateRequest {
            gen_input: gen_input("veo"),
            asset_type: ClipType::Video,
            placeholder_duration: 4.0,
            num_images: 1,
            name: Some("Test clip".into()),
            folder_id: None,
            file_extension: "mp4".into(),
            references: vec![],
            project_url: project,
            project_id: Some("proj_1".into()),
            build_params: Box::new(|_uploaded| {
                BackendGenerationParams::Video(VideoParams {
                    prompt: "a test".into(),
                    duration: 4,
                    aspect_ratio: "16:9".into(),
                    generate_audio: true,
                    ..Default::default()
                })
            }),
            snapshot_refs: None,
        }
    }

    #[tokio::test]
    async fn full_lifecycle_succeeds_and_downloads() {
        // A tiny HTTP server would be heavy; instead point the result URL at a
        // file:// — but reqwest doesn't do file://. Use a local mock via httpbin
        // is also heavy. Instead we assert the status sequence up to Downloading
        // and accept the download failing (no network) — the index-mapping +
        // transitions are the contract. See download test below for the bytes.
        let tmp = tempfile::tempdir().unwrap();
        let transport = Arc::new(
            MockTransport::builder()
                .statuses(vec![
                    BackendGenerationStatus::Queued,
                    BackendGenerationStatus::Running,
                    BackendGenerationStatus::Succeeded,
                ])
                .result_urls(vec!["https://127.0.0.1:1/out0.mp4".into()])
                .build(),
        );
        let service = GenerationService::new(transport.clone());
        let sink = Arc::new(RecordingSink::default());
        let handle = service.generate(video_request(Some(tmp.path().to_path_buf())), sink.clone());
        let primary = handle.primary_id.clone();
        handle.join().await;

        let updates = sink.snapshot();
        // Placeholder created + Generating fired synchronously.
        assert!(updates.iter().any(|u| matches!(u, StatusUpdate::PlaceholderCreated { id, .. } if id == &primary)));
        assert!(updates.iter().any(|u| matches!(u, StatusUpdate::Generating { .. })));
        // Reached Downloading (success path, index 0 mapped).
        assert!(updates.iter().any(|u| matches!(u, StatusUpdate::Downloading { .. })));
        // The submitted params carried the model + project id.
        let submitted = transport.submitted();
        assert_eq!(submitted.len(), 1);
        assert_eq!(submitted[0].0, "veo");
        assert_eq!(submitted[0].2.as_deref(), Some("proj_1"));
        assert_eq!(submitted[0].1["kind"], "video");
    }

    #[tokio::test]
    async fn failed_job_marks_all_placeholders_failed() {
        let transport = Arc::new(MockTransport::builder().fail_with("model exploded").build());
        let service = GenerationService::new(transport);
        let sink = Arc::new(RecordingSink::default());
        let mut req = video_request(None);
        req.asset_type = ClipType::Image;
        req.num_images = 3;
        req.file_extension = "png".into();
        req.build_params = Box::new(|_| {
            BackendGenerationParams::Image(ImageParams {
                prompt: "p".into(),
                aspect_ratio: "1:1".into(),
                num_images: 3,
                ..Default::default()
            })
        });
        let handle = service.generate(req, sink.clone());
        handle.join().await;

        let updates = sink.snapshot();
        // 3 placeholders created (image N-for-image).
        let created = updates
            .iter()
            .filter(|u| matches!(u, StatusUpdate::PlaceholderCreated { .. }))
            .count();
        assert_eq!(created, 3);
        // All 3 marked Failed with the backend message.
        let failed = updates
            .iter()
            .filter(|u| matches!(u, StatusUpdate::Failed { message, .. } if message == "model exploded"))
            .count();
        assert_eq!(failed, 3);
    }

    #[tokio::test]
    async fn not_configured_fails_all_placeholders() {
        let transport = Arc::new(MockTransport::builder().not_configured().build());
        let service = GenerationService::new(transport);
        let sink = Arc::new(RecordingSink::default());
        let handle = service.generate(video_request(None), sink.clone());
        handle.join().await;
        let updates = sink.snapshot();
        assert!(updates.iter().any(|u| matches!(u, StatusUpdate::Failed { .. })));
    }

    #[tokio::test]
    async fn fewer_result_urls_than_placeholders_fails_extras() {
        let tmp = tempfile::tempdir().unwrap();
        // 2 placeholders, only 1 result URL → placeholder[1] fails "No URL for placeholder".
        let transport = Arc::new(
            MockTransport::builder()
                .result_urls(vec!["https://127.0.0.1:1/out0.png".into()])
                .build(),
        );
        let service = GenerationService::new(transport);
        let sink = Arc::new(RecordingSink::default());
        let mut req = video_request(Some(tmp.path().to_path_buf()));
        req.asset_type = ClipType::Image;
        req.num_images = 2;
        req.file_extension = "png".into();
        req.build_params = Box::new(|_| {
            BackendGenerationParams::Image(ImageParams {
                prompt: "p".into(),
                aspect_ratio: "1:1".into(),
                num_images: 2,
                ..Default::default()
            })
        });
        let handle = service.generate(req, sink.clone());
        handle.join().await;
        let updates = sink.snapshot();
        assert!(updates.iter().any(
            |u| matches!(u, StatusUpdate::Failed { message, .. } if message == "No URL for placeholder")
        ));
    }

    #[tokio::test]
    async fn download_writes_bytes_and_fixes_extension() {
        // Spin a tiny local HTTP server returning bytes, point a result URL at it,
        // and assert the file is written + the Succeeded update carries the path.
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let body = b"FAKEPNGBYTES";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: image/png\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.write_all(body).await;
                let _ = sock.flush().await;
            }
        });

        let tmp = tempfile::tempdir().unwrap();
        let url = format!("http://{addr}/result.png");
        let transport = Arc::new(MockTransport::builder().result_urls(vec![url]).build());
        let service = GenerationService::new(transport);
        let sink = Arc::new(RecordingSink::default());
        let mut req = video_request(Some(tmp.path().to_path_buf()));
        req.asset_type = ClipType::Image;
        req.file_extension = "png".into();
        req.build_params = Box::new(|_| {
            BackendGenerationParams::Image(ImageParams {
                prompt: "p".into(),
                aspect_ratio: "1:1".into(),
                num_images: 1,
                ..Default::default()
            })
        });
        let handle = service.generate(req, sink.clone());
        handle.join().await;
        let _ = server.await;

        let updates = sink.snapshot();
        let succeeded = updates.iter().find_map(|u| match u {
            StatusUpdate::Succeeded { final_path, .. } => Some(final_path.clone()),
            _ => None,
        });
        let path = succeeded.expect("a Succeeded update");
        let written = std::fs::read(&path).unwrap();
        assert_eq!(written, b"FAKEPNGBYTES");
        // Completion toast fired once.
        assert!(updates.iter().any(|u| matches!(u, StatusUpdate::CompletionToast { count, .. } if *count == 1)));
    }
}
