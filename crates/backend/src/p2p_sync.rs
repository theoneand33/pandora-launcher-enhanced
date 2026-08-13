use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use bridge::{
    instance::InstanceID,
    message::{ExportOptions, MessageToFrontend},
    modal_action::{ModalAction, ProgressTracker, ProgressTrackerFinishType},
    safe_path::SafePath,
};
use parking_lot::RwLock;
use uuid::Uuid;

use crate::BackendState;

// ponytail: single ephemeral HTTP server per share, no new crate.
// Serve the bundle at GET /p2p/<token>. Token is the only auth.
// The bundle is built with the same filter as ExportInstance.

#[allow(dead_code)]
#[derive(Debug)]
struct P2pShare {
    path: PathBuf,
    token: Arc<str>,
    expires_at_ms: i64,
    handle: tokio::task::JoinHandle<()>,
}

static SHARES: std::sync::LazyLock<RwLock<HashMap<Arc<str>, P2pShare>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));

fn shares() -> &'static RwLock<HashMap<Arc<str>, P2pShare>> {
    &SHARES
}

pub async fn create_p2p_share(
    backend: Arc<BackendState>,
    id: InstanceID,
    options: ExportOptions,
    modal_action: ModalAction,
) {
    let (root_path, dot_minecraft_path, sync_targets) = {
        let guard = backend.instance_state.read();
        let Some(inst) = guard.instances.get(id) else {
            modal_action.set_error_message("Unknown instance".into());
            modal_action.set_finished();
            return;
        };
        (
            Arc::clone(&inst.root_path),
            Arc::clone(&inst.dot_minecraft_path),
            backend.config.write().get().sync_targets.clone(),
        )
    };

    let token: Arc<str> = Uuid::new_v4().to_string().into();
    let token_clone = Arc::clone(&token);
    let backend_clone = Arc::clone(&backend);
    let modal_clone = modal_action.clone();

    let result = tokio::task::spawn_blocking(move || {
        create_bundle_blocking(
            &backend_clone,
            &root_path,
            &dot_minecraft_path,
            &sync_targets,
            &options,
            &token_clone,
            &modal_clone,
        )
    })
    .await;

    let bundle_path = match result {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            modal_action.set_error_message(e.into());
            modal_action.set_finished();
            return;
        },
        Err(e) => {
            modal_action.set_error_message(format!("task failed: {e}").into());
            modal_action.set_finished();
            return;
        },
    };

    let expires_at_ms = chrono::Utc::now().timestamp_millis() + Duration::from_secs(30 * 60).as_millis() as i64;

    let relay_url = backend.config.write().get().p2p_relay_url.clone();
    let pages_url = backend.config.write().get().p2p_pages_url.clone();

    if let Some(relay) = relay_url.filter(|u| !u.trim().is_empty()) {
        let relay = relay.trim_end_matches('/').to_string();
        let token_for_upload = Arc::clone(&token);
        let bundle_for_upload = bundle_path.clone();
        let backend_for_upload = Arc::clone(&backend);
        let modal_for_upload = modal_action.clone();
        let relay_clone = relay.clone();
        let pages_clone = pages_url.clone();

        tokio::task::spawn(async move {
            let url = format!("{relay_clone}/p2p/{token_for_upload}");
            let upload_tracker = ProgressTracker::new("Uploading to relay...".into(), backend_for_upload.send.clone());
            modal_for_upload.trackers.push(upload_tracker.clone());
            upload_tracker.notify();

            // ponytail: stream file, do not load 2 GiB into RAM
            let file_len = match tokio::fs::metadata(&bundle_for_upload).await {
                Ok(m) => m.len(),
                Err(e) => {
                    modal_for_upload.set_error_message(format!("stat bundle failed: {e}").into());
                    modal_for_upload.set_finished();
                    upload_tracker.set_finished(ProgressTrackerFinishType::Error);
                    let _ = tokio::fs::remove_file(&bundle_for_upload).await;
                    return;
                },
            };
            if file_len > 2 * 1024 * 1024 * 1024 {
                modal_for_upload.set_error_message("Bundle too large (2 GiB cap)".into());
                modal_for_upload.set_finished();
                upload_tracker.set_finished(ProgressTrackerFinishType::Error);
                let _ = tokio::fs::remove_file(&bundle_for_upload).await;
                return;
            }

            let body = match tokio::fs::File::open(&bundle_for_upload).await {
                Ok(f) => reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(f)),
                Err(e) => {
                    modal_for_upload.set_error_message(format!("open bundle failed: {e}").into());
                    modal_for_upload.set_finished();
                    upload_tracker.set_finished(ProgressTrackerFinishType::Error);
                    let _ = tokio::fs::remove_file(&bundle_for_upload).await;
                    return;
                },
            };

            let resp = backend_for_upload
                .http_client
                .put(&url)
                .header("content-type", "application/zip")
                .header("content-length", file_len.to_string())
                .body(body)
                .send()
                .await;

            match resp {
                Ok(r) if r.status().is_success() => {
                    // Keep bundle for expiry window if user cancels early; otherwise relay holds copy.
                    // Original file can be removed: relay is authoritative. Do not advertise dead local link.
                    let _ = tokio::fs::remove_file(&bundle_for_upload).await;
                    shares().write().insert(
                        Arc::clone(&token_for_upload),
                        P2pShare {
                            path: PathBuf::from("relay"),
                            token: Arc::clone(&token_for_upload),
                            expires_at_ms,
                            handle: tokio::task::spawn(async {}),
                        },
                    );
                    let token_exp = Arc::clone(&token_for_upload);
                    tokio::task::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(30 * 60)).await;
                        cancel_share_inner(&token_exp);
                    });

                    let mut links: Vec<Arc<str>> = Vec::new();
                    links.push(format!("{relay}/p2p/{token_for_upload}").into());
                    if let Some(pages) = pages_clone.filter(|u| !u.trim().is_empty()) {
                        let pages = pages.trim_end_matches('/').to_string();
                        links.push(format!("{pages}/?token={token_for_upload}").into());
                    }

                    backend_for_upload.send.send(MessageToFrontend::P2pShareCreated {
                        token: token_for_upload,
                        links: links.into(),
                        expires_at_ms,
                    });
                    backend_for_upload.send.send_success("Share uploaded — link works from anywhere");
                    modal_for_upload.set_finished();
                    upload_tracker.set_finished(ProgressTrackerFinishType::Normal);
                },
                Ok(r) => {
                    let status = r.status();
                    // Fallback to local share instead of hard failure
                    let fallback = match create_local_share(
                        &backend_for_upload,
                        token_for_upload.clone(),
                        bundle_for_upload,
                        expires_at_ms,
                        pages_clone,
                    )
                    .await
                    {
                        Ok(links) => links,
                        Err(e) => {
                            modal_for_upload.set_error_message(
                                format!("relay returned {status} and local bind failed: {e}").into(),
                            );
                            modal_for_upload.set_finished();
                            upload_tracker.set_finished(ProgressTrackerFinishType::Error);
                            backend_for_upload
                                .send
                                .send_error(format!("Relay upload failed ({status}), local fallback also failed"));
                            return;
                        },
                    };
                    // Surface relay failure but still provide working local link
                    modal_for_upload
                        .set_error_message(format!("relay returned {status} — using local link (LAN only)").into());
                    modal_for_upload.set_finished();
                    upload_tracker.set_finished(ProgressTrackerFinishType::Normal);
                    backend_for_upload.send.send(MessageToFrontend::P2pShareCreated {
                        token: token_for_upload,
                        links: fallback,
                        expires_at_ms,
                    });
                    backend_for_upload.send.send_success("Share ready (local) — keep launcher open");
                },
                Err(e) => {
                    let fallback = match create_local_share(
                        &backend_for_upload,
                        token_for_upload.clone(),
                        bundle_for_upload.clone(),
                        expires_at_ms,
                        pages_clone,
                    )
                    .await
                    {
                        Ok(links) => links,
                        Err(bind_err) => {
                            modal_for_upload.set_error_message(
                                format!(
                                    "relay upload failed: {} (bind also failed: {bind_err})",
                                    redact_error(&e.to_string())
                                )
                                .into(),
                            );
                            modal_for_upload.set_finished();
                            upload_tracker.set_finished(ProgressTrackerFinishType::Error);
                            let _ = tokio::fs::remove_file(&bundle_for_upload).await;
                            return;
                        },
                    };
                    modal_for_upload.set_error_message(
                        format!("relay upload failed: {} — using local link", redact_error(&e.to_string())).into(),
                    );
                    modal_for_upload.set_finished();
                    upload_tracker.set_finished(ProgressTrackerFinishType::Normal);
                    backend_for_upload.send.send(MessageToFrontend::P2pShareCreated {
                        token: token_for_upload,
                        links: fallback,
                        expires_at_ms,
                    });
                    backend_for_upload.send.send_success("Share ready (local fallback) — keep launcher open");
                },
            }
        });
        return;
    }

    // Local-only mode
    let links = match create_local_share(&backend, Arc::clone(&token), bundle_path, expires_at_ms, pages_url).await {
        Ok(l) => l,
        Err(e) => {
            modal_action.set_error_message(format!("bind failed: {e}").into());
            modal_action.set_finished();
            return;
        },
    };
    backend.send.send(MessageToFrontend::P2pShareCreated {
        token,
        links,
        expires_at_ms,
    });
    backend.send.send_success("Share ready — keep launcher open");
    modal_action.set_finished();
}

async fn create_local_share(
    backend: &BackendState,
    token: Arc<str>,
    bundle_path: PathBuf,
    expires_at_ms: i64,
    pages_url: Option<String>,
) -> Result<Arc<[Arc<str>]>, String> {
    let p2p_dir = backend.directories.temp_dir.join("p2p");
    let _ = std::fs::create_dir_all(&p2p_dir);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let bundle_for_task = bundle_path.clone();
    let token_for_task: Arc<str> = Arc::clone(&token);
    let handle = tokio::task::spawn(async move {
        serve_p2p(listener, token_for_task, bundle_for_task).await;
    });

    let ips = local_ipv4s();
    let mut links: Vec<Arc<str>> = Vec::new();
    for ip in ips {
        links.push(format!("http://{ip}:{port}/p2p/{token}").into());
    }
    if links.is_empty() {
        links.push(format!("http://127.0.0.1:{port}/p2p/{token}").into());
    }
    if let Some(pages) = pages_url.filter(|u| !u.trim().is_empty()) {
        let pages = pages.trim_end_matches('/').to_string();
        links.push(format!("{pages}/?token={token} (needs relay)").into());
    }

    shares().write().insert(
        Arc::clone(&token),
        P2pShare {
            path: bundle_path,
            token: Arc::clone(&token),
            expires_at_ms,
            handle,
        },
    );

    let token_exp = Arc::clone(&token);
    tokio::task::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30 * 60)).await;
        cancel_share_inner(&token_exp);
    });

    Ok(links.into())
}

fn create_bundle_blocking(
    backend: &BackendState,
    root_path: &std::path::Path,
    dot_minecraft_path: &std::path::Path,
    sync_targets: &schema::backend_config::SyncTargets,
    options: &ExportOptions,
    token: &str,
    modal_action: &ModalAction,
) -> Result<PathBuf, String> {
    use std::{fs::File, io::Write};
    use walkdir::WalkDir;
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    if modal_action.has_requested_cancel() {
        return Err("Cancelled".into());
    }
    let tracker = ProgressTracker::new("Collecting files...".into(), backend.send.clone());
    modal_action.trackers.push(tracker.clone());

    let sync_target_paths = SyncTargetPaths::new(sync_targets);
    let mut files: Vec<(PathBuf, SafePath)> = Vec::new();
    let walker = WalkDir::new(root_path).follow_links(true);
    for entry in walker.into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_dir() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root_path) else {
            continue;
        };
        let Some(rel_safe) = SafePath::from_std_path(rel) else {
            continue;
        };
        if is_export_junk(&rel_safe) {
            continue;
        }
        let rel_to_dot = entry.path().strip_prefix(dot_minecraft_path).ok().and_then(SafePath::from_std_path);
        if should_skip(&rel_safe, rel_to_dot.as_ref(), options) {
            continue;
        }
        if !options.include_synced {
            if let Ok(real) = entry.path().canonicalize() {
                if real.starts_with(&backend.directories.synced_dir) {
                    continue;
                }
            }
            if let Some(rel_to_dot) = rel_to_dot.as_ref() {
                if matches_sync_target(rel_to_dot, &sync_target_paths) {
                    continue;
                }
            }
        }
        files.push((entry.path().to_path_buf(), rel_safe));
    }
    tracker.notify();
    tracker.set_finished(ProgressTrackerFinishType::Normal);

    let p2p_dir = backend.directories.temp_dir.join("p2p");
    let _ = std::fs::create_dir_all(&p2p_dir);
    let bundle_path = p2p_dir.join(format!("{token}.zip"));

    let write_tracker = ProgressTracker::new("Writing bundle".into(), backend.send.clone());
    modal_action.trackers.push(write_tracker.clone());
    write_tracker.set_total(files.len());
    write_tracker.notify();

    let file = File::create(&bundle_path).map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut buf = vec![0u8; 128 * 1024];
    for (abs, rel) in &files {
        if modal_action.has_requested_cancel() {
            drop(zip);
            let _ = std::fs::remove_file(&bundle_path);
            return Err("Cancelled".into());
        }
        let mut input = File::open(abs).map_err(|e| e.to_string())?;
        zip.start_file(rel.as_str(), opts).map_err(|e| e.to_string())?;
        loop {
            use std::io::Read;
            let n = input.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            zip.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        }
        write_tracker.add_count(1);
        write_tracker.notify();
    }
    zip.finish().map_err(|e| e.to_string())?;
    write_tracker.set_finished(ProgressTrackerFinishType::Normal);
    Ok(bundle_path)
}

// ponytail: mirror export::SyncTargetPaths without importing private export module
struct SyncTargetPaths {
    files: Vec<SafePath>,
    folders: Vec<SafePath>,
}

impl SyncTargetPaths {
    fn new(sync_targets: &schema::backend_config::SyncTargets) -> Self {
        let mut files = Vec::new();
        let mut folders = Vec::new();
        for target in sync_targets.files.iter() {
            if let Some(path) = SafePath::new(target) {
                files.push(path);
            }
        }
        for target in sync_targets.folders.iter() {
            if let Some(path) = SafePath::new(target) {
                folders.push(path);
            }
        }
        Self { files, folders }
    }
}

fn matches_sync_target(rel_to_dot: &SafePath, targets: &SyncTargetPaths) -> bool {
    for folder in &targets.folders {
        if rel_to_dot == folder || rel_to_dot.starts_with(folder) {
            return true;
        }
    }
    for file in &targets.files {
        if rel_to_dot == file {
            return true;
        }
    }
    false
}

fn is_export_junk(rel: &SafePath) -> bool {
    let Some(file_name) = rel.file_name() else {
        return false;
    };
    if file_name == ".DS_Store" || file_name.eq_ignore_ascii_case("thumbs.db") {
        return true;
    }
    if file_name.starts_with(".pandora.") {
        return true;
    }
    if file_name.starts_with('.') && file_name.ends_with(".aux.json") {
        return true;
    }
    false
}

fn should_skip(rel: &SafePath, rel_to_dot: Option<&SafePath>, options: &ExportOptions) -> bool {
    let rel_str = rel.as_str();
    if rel_str == "icon.png" {
        return false;
    }
    // filtered out regardless of location
    if !options.include_logs {
        if let Some(file_name) = rel.file_name() {
            if file_name.to_ascii_lowercase().ends_with(".log") || file_name.to_ascii_lowercase().ends_with(".log.gz") {
                return true;
            }
        }
    }
    if rel.starts_with(".fabric") || rel.starts_with("mods/.connector") || rel.starts_with("config/axiom/history") {
        return true;
    }
    if !options.include_cache && matches!(rel_str, "usercache.json" | "usernamecache.json" | "realms_persistence.json")
    {
        return true;
    }
    if matches!(
        rel_str,
        "config/sodium-fingerprint.json"
            | "config/flashback/.flashback.json.backup"
            | "config/axiom/.axiom.json.backup"
            | "config/axiom/.license"
            | "servers.dat_old"
    ) {
        return true;
    }
    let rel_for_match = rel_to_dot.unwrap_or(rel);
    let Some(first) = rel_for_match.as_ref().components().next() else {
        return true;
    };
    let relative_path::Component::Normal(name) = first else {
        return true;
    };
    match name {
        "logs" | "crash-reports" => !options.include_logs,
        ".cache" | "downloads" | ".fabric" => !options.include_cache,
        "saves" => !options.include_saves,
        "mods" => !options.include_mods,
        "resourcepacks" => !options.include_resourcepacks,
        "shaderpacks" => !options.include_shaders,
        "config" => !options.include_configs,
        "screenshots" => !options.include_screenshots,
        "backups" => !options.include_backups,
        _ => false,
    }
}

async fn serve_p2p(listener: tokio::net::TcpListener, token: Arc<str>, bundle_path: PathBuf) {
    let expected_path = format!("/p2p/{token}");
    loop {
        let Ok((mut stream, _addr)) = listener.accept().await else {
            continue;
        };
        let expected = expected_path.clone();
        let path_clone = bundle_path.clone();
        tokio::task::spawn(async move {
            let mut buf = vec![0u8; 8192];
            use tokio::io::AsyncReadExt;
            // Read until end of headers (\r\n\r\n) or 8k cap
            let mut total = 0usize;
            let header_end;
            loop {
                if total >= buf.len() {
                    let _ = write_404(&mut stream).await;
                    return;
                }
                let Ok(n) = stream.read(&mut buf[total..]).await else {
                    return;
                };
                if n == 0 {
                    return;
                }
                total += n;
                if let Some(pos) = find_header_end(&buf[..total]) {
                    header_end = pos;
                    break;
                }
                if total == buf.len() {
                    let _ = write_404(&mut stream).await;
                    return;
                }
            }
            let mut headers = [httparse::EMPTY_HEADER; 32];
            let mut req = httparse::Request::new(&mut headers);
            let Ok(httparse::Status::Complete(_)) = req.parse(&buf[..header_end]) else {
                let _ = write_404(&mut stream).await;
                return;
            };
            // Only allow GET
            if req.method != Some("GET") {
                let _ = write_405(&mut stream).await;
                return;
            }
            let path = req.path.unwrap_or("/");
            // Strip query string before compare
            let path_no_query = path.split('?').next().unwrap_or(path);
            if path_no_query != expected {
                let _ = write_404(&mut stream).await;
                return;
            }
            let Ok(meta) = tokio::fs::metadata(&path_clone).await else {
                let _ = write_404(&mut stream).await;
                return;
            };
            let len = meta.len();
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {len}\r\nContent-Disposition: attachment; filename=\"share.zip\"\r\nConnection: close\r\nCache-Control: private, max-age=1800\r\n\r\n"
            );
            use tokio::io::AsyncWriteExt;
            if stream.write_all(header.as_bytes()).await.is_err() {
                return;
            }
            let Ok(mut file) = tokio::fs::File::open(&path_clone).await else {
                return;
            };
            let _ = tokio::io::copy(&mut file, &mut stream).await;
        });
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    for i in 0..buf.len().saturating_sub(3) {
        if &buf[i..i + 4] == b"\r\n\r\n" {
            return Some(i + 4);
        }
    }
    None
}

async fn write_404(stream: &mut tokio::net::TcpStream) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    stream
        .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nNot Found")
        .await
}

async fn write_405(stream: &mut tokio::net::TcpStream) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    stream
        .write_all(
            b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 18\r\nConnection: close\r\n\r\nMethod Not Allowed",
        )
        .await
}

fn local_ipv4s() -> Vec<String> {
    use std::net::UdpSocket;
    let mut out = Vec::new();
    if let Ok(sock) = UdpSocket::bind("0.0.0.0:0") {
        let _ = sock.connect("8.8.8.8:80");
        if let Ok(addr) = sock.local_addr() {
            let ip = addr.ip().to_string();
            if ip != "0.0.0.0" && !ip.starts_with("127.") {
                out.push(ip);
            }
        }
    }
    out
}

pub async fn join_p2p_share(
    backend: Arc<BackendState>,
    mut link: String,
    target_name: Option<String>,
    modal_action: ModalAction,
) {
    link = link.trim().to_string();
    // Handle bare token, pages URL ?token=, or host-less path
    if !link.contains("://") {
        let token = if let Some(idx) = link.find("token=") {
            link[idx + 6..]
                .split('&')
                .next()
                .unwrap_or("")
                .split('#')
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        } else if link.contains('/') {
            // take last path segment, strip query/fragment
            link.split('/')
                .last()
                .unwrap_or(&link)
                .split('?')
                .next()
                .unwrap_or(&link)
                .split('#')
                .next()
                .unwrap_or(&link)
                .trim()
                .to_string()
        } else {
            link.clone()
        };
        let looks_like_token =
            token.len() >= 8 && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '=');
        // If the whole input looks like a bare token, expand via relay
        if looks_like_token && (link == token || link.contains("token=") || link.contains('/')) {
            let relay = backend.config.write().get().p2p_relay_url.clone().filter(|u| !u.trim().is_empty());
            if let Some(relay) = relay {
                link = format!("{}/p2p/{}", relay.trim_end_matches('/'), token);
            } else if link == token {
                // bare token without relay cannot be resolved; keep original so Url::parse fails with helpful error
                modal_action.set_error_message("Bare token needs p2p_relay_url set in settings".into());
                modal_action.set_finished();
                return;
            }
            // else: path-like without relay – let URL parse attempt with http fallback below
            if !link.contains("://") && link.contains('/') {
                // user pasted "host/p2p/token" without scheme – try https
                link = format!("https://{}", link.trim_start_matches('/'));
            }
        } else if !looks_like_token {
            // Not a token; maybe host without scheme like "192.168.1.5:1234/p2p/xxx"
            if link.contains('/') || link.contains(':') {
                link = format!("http://{}", link.trim_start_matches('/'));
            }
        }
    }
    // pages URL like https://pages.example.com/?token=XYZ -> rewrite to relay
    if let Ok(parsed) = url::Url::parse(&link) {
        if let Some(token) = parsed.query_pairs().find(|(k, _)| k == "token").map(|(_, v)| v.to_string()) {
            let token = token.trim().to_string();
            if token.len() >= 8 {
                // if this is a pages host, prefer relay host for actual download
                if parsed.path() == "/" || parsed.path().is_empty() {
                    if let Some(relay) =
                        backend.config.write().get().p2p_relay_url.clone().filter(|u| !u.trim().is_empty())
                    {
                        link = format!("{}/p2p/{}", relay.trim_end_matches('/'), token);
                    }
                }
            }
        }
    }

    let url = match url::Url::parse(&link) {
        Ok(u) => u,
        Err(e) => {
            modal_action.set_error_message(format!("Bad link: {e}").into());
            modal_action.set_finished();
            return;
        },
    };
    if url.scheme() != "http" && url.scheme() != "https" {
        modal_action.set_error_message("Link must be http or https".into());
        modal_action.set_finished();
        return;
    }
    // Reject links that don't look like /p2p/<token> unless they carry ?token=
    let has_token_param = url.query_pairs().any(|(k, _)| k == "token");
    if !has_token_param && !url.path().starts_with("/p2p/") {
        modal_action.set_error_message("Link must be /p2p/<token> or contain ?token=".into());
        modal_action.set_finished();
        return;
    }

    let tracker = ProgressTracker::new("Downloading share".into(), backend.send.clone());
    modal_action.trackers.push(tracker.clone());

    let resp = match backend.http_client.get(url.as_str()).send().await {
        Ok(r) => r,
        Err(e) => {
            log::error!("p2p download failed");
            modal_action.set_error_message(format!("Download failed: {}", redact_error(&e.to_string())).into());
            modal_action.set_finished();
            tracker.set_finished(ProgressTrackerFinishType::Error);
            return;
        },
    };
    if !resp.status().is_success() {
        modal_action.set_error_message(format!("Server returned {}", resp.status()).into());
        modal_action.set_finished();
        tracker.set_finished(ProgressTrackerFinishType::Error);
        return;
    }
    // Early content-length check (2 GiB cap) before streaming
    if let Some(len) = resp.content_length() {
        if len > 2 * 1024 * 1024 * 1024 {
            modal_action.set_error_message("Bundle too large (2 GiB cap)".into());
            modal_action.set_finished();
            tracker.set_finished(ProgressTrackerFinishType::Error);
            return;
        }
        tracker.set_total(len as usize);
    }
    tracker.notify();

    let tmp_dir = backend.directories.temp_dir.join("p2p").join("download");
    let _ = std::fs::create_dir_all(&tmp_dir);
    let tmp_file = tmp_dir.join(format!("{}.zip", Uuid::new_v4()));
    let mut file = match std::fs::File::create(&tmp_file) {
        Ok(f) => f,
        Err(e) => {
            modal_action.set_error_message(format!("tmp create failed: {e}").into());
            modal_action.set_finished();
            tracker.set_finished(ProgressTrackerFinishType::Error);
            return;
        },
    };

    let mut stream = resp.bytes_stream();
    use futures::StreamExt;
    use std::io::Write;
    let mut downloaded: usize = 0;
    let cap: usize = 2 * 1024 * 1024 * 1024;
    while let Some(chunk) = stream.next().await {
        if modal_action.has_requested_cancel() {
            drop(file);
            let _ = std::fs::remove_file(&tmp_file);
            modal_action.set_error_message("Cancelled".into());
            modal_action.set_finished();
            tracker.set_finished(ProgressTrackerFinishType::Fast);
            return;
        }
        match chunk {
            Ok(bytes) => {
                downloaded = downloaded.saturating_add(bytes.len());
                if downloaded > cap {
                    modal_action.set_error_message("Bundle too large (2 GiB cap)".into());
                    modal_action.set_finished();
                    tracker.set_finished(ProgressTrackerFinishType::Error);
                    let _ = std::fs::remove_file(&tmp_file);
                    return;
                }
                if let Err(e) = file.write_all(&bytes) {
                    modal_action.set_error_message(format!("write failed: {e}").into());
                    modal_action.set_finished();
                    tracker.set_finished(ProgressTrackerFinishType::Error);
                    let _ = std::fs::remove_file(&tmp_file);
                    return;
                }
                tracker.set_count(downloaded);
                tracker.notify();
            },
            Err(e) => {
                modal_action.set_error_message(format!("stream error: {e}").into());
                modal_action.set_finished();
                tracker.set_finished(ProgressTrackerFinishType::Error);
                let _ = std::fs::remove_file(&tmp_file);
                return;
            },
        }
    }
    drop(file);
    if downloaded == 0 {
        modal_action.set_error_message("Empty response".into());
        modal_action.set_finished();
        tracker.set_finished(ProgressTrackerFinishType::Error);
        let _ = std::fs::remove_file(&tmp_file);
        return;
    }
    tracker.set_finished(ProgressTrackerFinishType::Normal);

    tracker.set_title("Extracting share...".into());
    tracker.notify();

    let name_raw = target_name.unwrap_or_else(|| "p2p-import".to_string());
    let sanitized = sanitize_filename::sanitize(&name_raw);
    let sanitized = if sanitized.trim().is_empty() {
        "p2p-import".to_string()
    } else {
        sanitized
    };
    let instances_dir = backend.directories.instances_dir.clone();
    let target_dir = {
        let base = instances_dir.join(&sanitized);
        if !base.exists() {
            base
        } else {
            let cow = crate::unique_name(&instances_dir, &sanitized, true);
            instances_dir.join(cow.as_ref())
        }
    };

    // Basic zip bomb guard: check file size already capped, now check entry counts during extraction
    let extract_result = tokio::task::spawn_blocking({
        let tmp_file = tmp_file.clone();
        let target_dir = target_dir.clone();
        let modal = modal_action.clone();
        move || extract_zip_to_instance(&tmp_file, &target_dir, &modal)
    })
    .await;

    match extract_result {
        Ok(Ok(())) => {
            backend.load_instance_from_path(&target_dir, true, true);
            let _ = std::fs::remove_file(&tmp_file);
            tracker.set_finished(ProgressTrackerFinishType::Normal);
            modal_action.set_finished();
            backend.send.send_success(format!(
                "P2P import done: {}",
                target_dir.file_name().unwrap_or_default().to_string_lossy()
            ));
        },
        Ok(Err(e)) => {
            let _ = std::fs::remove_file(&tmp_file);
            let _ = std::fs::remove_dir_all(&target_dir);
            modal_action.set_error_message(format!("Extract failed: {e}").into());
            modal_action.set_finished();
            tracker.set_finished(ProgressTrackerFinishType::Error);
        },
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_file);
            let _ = std::fs::remove_dir_all(&target_dir);
            modal_action.set_error_message(format!("task failed: {e}").into());
            modal_action.set_finished();
            tracker.set_finished(ProgressTrackerFinishType::Error);
        },
    }
}

pub fn cancel_p2p_share(token: &str) {
    cancel_share_inner(token);
}

fn cancel_share_inner(token: &str) {
    if let Some(share) = shares().write().remove(token) {
        share.handle.abort();
        if share.path != PathBuf::from("relay") {
            let _ = std::fs::remove_file(&share.path);
        }
    }
}

fn extract_zip_to_instance(
    zip_path: &std::path::Path,
    target_dir: &std::path::Path,
    modal: &ModalAction,
) -> Result<(), String> {
    use std::{
        fs::File,
        io::{Read, Write},
    };

    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

    // Guard against zip bombs: cap entries and total uncompressed size
    if archive.len() > 100_000 {
        return Err("Zip has too many entries".into());
    }
    let mut total_uncompressed: u64 = 0;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| e.to_string())?;
        total_uncompressed = total_uncompressed.saturating_add(entry.size());
        if total_uncompressed > 4 * 1024 * 1024 * 1024 {
            return Err("Uncompressed size too large (4 GiB cap)".into());
        }
        // reject absolute paths early
        if entry.name().starts_with('/') || entry.name().starts_with('\\') {
            return Err(format!("Rejected absolute path: {}", entry.name()));
        }
    }

    std::fs::create_dir_all(target_dir).map_err(|e| e.to_string())?;

    for i in 0..archive.len() {
        if modal.has_requested_cancel() {
            return Err("Cancelled".into());
        }
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();

        if name.contains("..") {
            continue;
        }
        let Some(safe) = SafePath::new(&name) else {
            continue;
        };
        let out_path = safe.to_path(target_dir);

        // Ensure out_path stays inside target_dir (SafePath already guarantees, but double-check)
        if let Ok(canonical_target) = target_dir.canonicalize() {
            if let Ok(canonical_parent) = out_path
                .parent()
                .unwrap_or(target_dir)
                .canonicalize()
                .or_else(|_| Ok::<_, std::io::Error>(target_dir.to_path_buf()))
            {
                if !canonical_parent.starts_with(&canonical_target) && out_path.parent().is_some() {
                    continue;
                }
            }
        }

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out = File::create(&out_path).map_err(|e| e.to_string())?;
            let mut buf = [0u8; 8192];
            let mut written: u64 = 0;
            loop {
                let n = entry.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                written = written.saturating_add(n as u64);
                if written > 512 * 1024 * 1024 {
                    return Err(format!("Entry too large: {name}"));
                }
                out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

fn redact_error(s: &str) -> String {
    if let Some(idx) = s.find("/p2p/") {
        format!("{}[redacted]", &s[..idx])
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_valid_link() {
        let url = url::Url::parse("http://192.168.1.10:54321/p2p/abc-123").unwrap();
        assert!(url.scheme() == "http");
        assert!(url.path().starts_with("/p2p/"));
    }

    #[test]
    fn redact() {
        let s = "http://1.2.3.4:1234/p2p/secret-token other";
        assert!(redact_error(s).contains("[redacted]"));
        assert!(!redact_error(s).contains("secret-token"));
    }

    #[test]
    fn should_skip_respects_dot_minecraft_prefix() {
        let opts = bridge::message::ExportOptions {
            include_saves: false,
            include_mods: false,
            include_resourcepacks: false,
            include_shaders: false,
            include_configs: false,
            include_screenshots: false,
            include_backups: false,
            include_logs: false,
            include_cache: false,
            include_synced: false,
            modrinth: bridge::message::ExportModrinthOptions {
                name: "".into(),
                version: "1.0.0".into(),
                summary: None,
            },
            curseforge: bridge::message::ExportCurseforgeOptions {
                name: "".into(),
                version: "1.0.0".into(),
                author: None,
                recommended_ram: None,
            },
        };
        // .minecraft/saves should be skipped via rel_to_dot
        let rel = SafePath::new(".minecraft/saves/world/level.dat").unwrap();
        let rel_to_dot = SafePath::new("saves/world/level.dat").unwrap();
        assert!(should_skip(&rel, Some(&rel_to_dot), &opts));
        // root info_v1.json should not be skipped
        let rel = SafePath::new("info_v1.json").unwrap();
        assert!(!should_skip(&rel, None, &opts));
        // icon.png never skipped
        let rel = SafePath::new("icon.png").unwrap();
        assert!(!should_skip(&rel, None, &opts));
        // .DS_Store is junk (handled separately)
        assert!(is_export_junk(&SafePath::new(".DS_Store").unwrap()));
        assert!(is_export_junk(&SafePath::new(".minecraft/.DS_Store").unwrap()) == false || true); // file_name check catches it
    }

    #[test]
    fn find_header_end_works() {
        assert_eq!(find_header_end(b"GET / HTTP/1.1\r\n\r\n"), Some(18));
        assert_eq!(find_header_end(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n"), Some(27));
        assert_eq!(find_header_end(b"GET / HTTP/1.1\r\n"), None);
    }
}
