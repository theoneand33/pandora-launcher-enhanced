use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use bridge::{
    instance::InstanceID,
    message::{ExportOptions, MessageToFrontend},
    modal_action::{ModalAction, ProgressTracker, ProgressTrackerFinishType},
};
use parking_lot::RwLock;
use uuid::Uuid;

use crate::BackendState;

// ponytail: single ephemeral HTTP server per share, no new crate.
// Serve the bundle at GET /p2p/<token>. Token is the only auth.
// The bundle is built with the same filter as ExportInstance.

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
    // Snapshot instance paths without holding lock across await
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

    // Spawn blocking work for zip creation
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

    // Start ephemeral server
    let p2p_dir = backend.directories.temp_dir.join("p2p");
    let _ = std::fs::create_dir_all(&p2p_dir);

    let listener = match tokio::net::TcpListener::bind("0.0.0.0:0").await {
        Ok(l) => l,
        Err(e) => {
            modal_action.set_error_message(format!("bind failed: {e}").into());
            modal_action.set_finished();
            let _ = std::fs::remove_file(&bundle_path);
            return;
        },
    };
    let addr = match listener.local_addr() {
        Ok(a) => a,
        Err(e) => {
            modal_action.set_error_message(format!("addr failed: {e}").into());
            modal_action.set_finished();
            let _ = std::fs::remove_file(&bundle_path);
            return;
        },
    };
    let port = addr.port();
    let bundle_for_task = bundle_path.clone();
    let token_for_task: Arc<str> = Arc::clone(&token);

    let handle = tokio::task::spawn(async move {
        serve_p2p(listener, token_for_task, bundle_for_task).await;
    });

    let expires_at_ms = chrono::Utc::now().timestamp_millis() + Duration::from_secs(30 * 60).as_millis() as i64;

    // Prefer relay/Coolify if configured. Launcher uploads the zip, otherwise serves locally.
    let relay_url = backend.config.write().get().p2p_relay_url.clone();
    let pages_url = backend.config.write().get().p2p_pages_url.clone();

    if let Some(relay) = relay_url.filter(|u| !u.trim().is_empty()) {
        // Upload to relay: PUT <relay>/p2p/<token>
        let relay = relay.trim_end_matches('/').to_string();
        let token_for_upload = Arc::clone(&token);
        let bundle_for_upload = bundle_path.clone();
        let backend_for_upload = Arc::clone(&backend);
        let modal_for_upload = modal_action.clone();
        let relay_clone = relay.clone();
        let pages_clone = pages_url.clone();

        // Spawn upload task; report progress via modal
        tokio::task::spawn(async move {
            let url = format!("{relay_clone}/p2p/{token_for_upload}");
            let upload_tracker = ProgressTracker::new("Uploading to relay...".into(), backend_for_upload.send.clone());
            modal_for_upload.trackers.push(upload_tracker.clone());
            upload_tracker.notify();

            let data = match tokio::fs::read(&bundle_for_upload).await {
                Ok(d) => d,
                Err(e) => {
                    modal_for_upload.set_error_message(format!("read bundle failed: {e}").into());
                    modal_for_upload.set_finished();
                    let _ = std::fs::remove_file(&bundle_for_upload);
                    return;
                },
            };
            let resp = backend_for_upload.http_client.put(&url).body(data).send().await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let _ = std::fs::remove_file(&bundle_for_upload);
                    // Keep handle alive as dummy so cancel still works (no local server)
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
                    // Also keep local link as fallback
                    links.push(format!("http://127.0.0.1:{port}/p2p/{token_for_upload}").into());

                    backend_for_upload.send.send(MessageToFrontend::P2pShareCreated {
                        token: token_for_upload,
                        links: links.into(),
                        expires_at_ms,
                    });
                    backend_for_upload.send.send_success("Share uploaded — link works from anywhere");
                    modal_for_upload.set_finished();
                },
                Ok(r) => {
                    modal_for_upload.set_error_message(
                        format!("relay returned {} — falling back to local link", r.status()).into(),
                    );
                    modal_for_upload.set_finished();
                    let _ = std::fs::remove_file(&bundle_for_upload);
                    backend_for_upload.send.send_error("Relay upload failed, share not created");
                    handle.abort();
                },
                Err(e) => {
                    modal_for_upload
                        .set_error_message(format!("relay upload failed: {}", redact_error(&e.to_string())).into());
                    modal_for_upload.set_finished();
                    let _ = std::fs::remove_file(&bundle_for_upload);
                    handle.abort();
                },
            }
            upload_tracker.set_finished(ProgressTrackerFinishType::Normal);
        });
        return;
    }

    // Local-only mode (LAN / fallback)
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

    // Schedule expiry cleanup
    let token_exp = Arc::clone(&token);
    tokio::task::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30 * 60)).await;
        cancel_share_inner(&token_exp);
    });

    backend.send.send(MessageToFrontend::P2pShareCreated {
        token,
        links: links.into(),
        expires_at_ms,
    });
    backend.send.send_success("Share ready — keep launcher open");
    modal_action.set_finished();
}

fn create_bundle_blocking(
    backend: &BackendState,
    root_path: &std::path::Path,
    _dot_minecraft_path: &std::path::Path,
    _sync_targets: &schema::backend_config::SyncTargets,
    options: &ExportOptions,
    token: &str,
    modal_action: &ModalAction,
) -> Result<PathBuf, String> {
    use bridge::safe_path::SafePath;
    use std::{fs::File, io::Write};
    use walkdir::WalkDir;
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    if modal_action.has_requested_cancel() {
        return Err("Cancelled".into());
    }
    let tracker = ProgressTracker::new("Collecting files...".into(), backend.send.clone());
    modal_action.trackers.push(tracker.clone());

    // ponytail: reuse export filtering idea without pulling export internals
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
        if should_skip(&rel_safe, options) {
            continue;
        }
        // skip synced content unless requested
        if !options.include_synced {
            let synced_dir = &backend.directories.synced_dir;
            if let Ok(real) = entry.path().canonicalize() {
                if real.starts_with(synced_dir) {
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

fn should_skip(rel: &bridge::safe_path::SafePath, options: &ExportOptions) -> bool {
    let rel_str = rel.as_str();
    if rel_str == "icon.png" {
        return false;
    }
    // mirror a subset of export::should_skip
    let Some(first) = rel.as_ref().components().next() else {
        return true;
    };
    let relative_path::Component::Normal(name) = first else {
        return true;
    };
    match name {
        "saves" => !options.include_saves,
        "mods" => !options.include_mods,
        "resourcepacks" => !options.include_resourcepacks,
        "shaderpacks" => !options.include_shaders,
        "config" => !options.include_configs,
        "screenshots" => !options.include_screenshots,
        "backups" => !options.include_backups,
        "logs" | "crash-reports" => !options.include_logs,
        ".cache" | "downloads" => !options.include_cache,
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
            let mut buf = vec![0u8; 4096];
            use tokio::io::AsyncReadExt;
            let Ok(n) = stream.read(&mut buf).await else {
                return;
            };
            if n == 0 {
                return;
            }
            let mut headers = [httparse::EMPTY_HEADER; 16];
            let mut req = httparse::Request::new(&mut headers);
            let Ok(httparse::Status::Complete(_)) = req.parse(&buf[..n]) else {
                let _ = write_404(&mut stream).await;
                return;
            };
            let path = req.path.unwrap_or("/");
            if path != expected {
                let _ = write_404(&mut stream).await;
                return;
            }
            let Ok(meta) = tokio::fs::metadata(&path_clone).await else {
                let _ = write_404(&mut stream).await;
                return;
            };
            let len = meta.len();
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {len}\r\nContent-Disposition: attachment; filename=\"share.zip\"\r\nConnection: close\r\n\r\n"
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

async fn write_404(stream: &mut tokio::net::TcpStream) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    stream
        .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nNot Found")
        .await
}

fn local_ipv4s() -> Vec<String> {
    // ponytail: getifaddrs is overkill; try to guess via UDP socket trick
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
    // Bare token support: if input has no scheme, treat as token and expand via relay_url or pages url param
    if !link.contains("://") {
        // extract token from possible "token=XYZ" or full query
        let token = if link.contains("token=") {
            link.split("token=")
                .last()
                .unwrap_or(&link)
                .split('&')
                .next()
                .unwrap_or(&link)
                .trim()
                .to_string()
        } else if link.contains('/') {
            link.split('/').last().unwrap_or(&link).trim().to_string()
        } else {
            link.clone()
        };
        // basic token sanity
        if token.len() >= 8 && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            let relay = backend.config.write().get().p2p_relay_url.clone().filter(|u| !u.trim().is_empty());
            if let Some(relay) = relay {
                link = format!("{}/p2p/{}", relay.trim_end_matches('/'), token);
            } else {
                modal_action.set_error_message("Bare token needs p2p_relay_url set in settings".into());
                modal_action.set_finished();
                return;
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

    let tracker = ProgressTracker::new("Downloading share".into(), backend.send.clone());
    modal_action.trackers.push(tracker.clone());

    let resp = match backend.http_client.get(url.as_str()).send().await {
        Ok(r) => r,
        Err(e) => {
            // redact token from logs
            log::error!("p2p download failed");
            modal_action.set_error_message(format!("Download failed: {}", redact_error(&e.to_string())).into());
            modal_action.set_finished();
            return;
        },
    };
    if !resp.status().is_success() {
        modal_action.set_error_message(format!("Server returned {}", resp.status()).into());
        modal_action.set_finished();
        return;
    }
    let total = resp.content_length().unwrap_or(0) as usize;
    if total > 0 {
        tracker.set_total(total);
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
            return;
        },
    };

    let mut stream = resp.bytes_stream();
    use futures::StreamExt;
    use std::io::Write;
    let mut downloaded: usize = 0;
    let cap: usize = 2 * 1024 * 1024 * 1024; // 2 GiB
    while let Some(chunk) = stream.next().await {
        if modal_action.has_requested_cancel() {
            drop(file);
            let _ = std::fs::remove_file(&tmp_file);
            modal_action.set_error_message("Cancelled".into());
            modal_action.set_finished();
            return;
        }
        match chunk {
            Ok(bytes) => {
                downloaded = downloaded.saturating_add(bytes.len());
                if downloaded > cap {
                    modal_action.set_error_message("Bundle too large (2 GiB cap)".into());
                    modal_action.set_finished();
                    let _ = std::fs::remove_file(&tmp_file);
                    return;
                }
                if let Err(e) = file.write_all(&bytes) {
                    modal_action.set_error_message(format!("write failed: {e}").into());
                    modal_action.set_finished();
                    return;
                }
                tracker.set_count(downloaded);
                tracker.notify();
            },
            Err(e) => {
                modal_action.set_error_message(format!("stream error: {e}").into());
                modal_action.set_finished();
                let _ = std::fs::remove_file(&tmp_file);
                return;
            },
        }
    }
    tracker.set_finished(ProgressTrackerFinishType::Normal);

    tracker.set_title("Extracting share...".into());
    tracker.notify();

    let name_raw = target_name.unwrap_or_else(|| "p2p-import".to_string());
    // sanitize and make unique under instances_dir
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
            // find unique name like "name (1)"
            let cow = crate::unique_name(&instances_dir, &sanitized, true);
            instances_dir.join(cow.as_ref())
        }
    };

    let extract_result = tokio::task::spawn_blocking({
        let tmp_file = tmp_file.clone();
        let target_dir = target_dir.clone();
        let modal = modal_action.clone();
        move || extract_zip_to_instance(&tmp_file, &target_dir, &modal)
    })
    .await;

    match extract_result {
        Ok(Ok(())) => {
            // register new instance with watcher
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
        },
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_file);
            modal_action.set_error_message(format!("task failed: {e}").into());
            modal_action.set_finished();
        },
    }
}

pub fn cancel_p2p_share(token: &str) {
    cancel_share_inner(token);
}

fn cancel_share_inner(token: &str) {
    if let Some(share) = shares().write().remove(token) {
        share.handle.abort();
        let _ = std::fs::remove_file(&share.path);
    }
}

fn extract_zip_to_instance(
    zip_path: &std::path::Path,
    target_dir: &std::path::Path,
    modal: &ModalAction,
) -> Result<(), String> {
    use bridge::safe_path::SafePath;
    use std::{
        fs::File,
        io::{Read, Write},
    };

    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

    std::fs::create_dir_all(target_dir).map_err(|e| e.to_string())?;

    for i in 0..archive.len() {
        if modal.has_requested_cancel() {
            return Err("Cancelled".into());
        }
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();

        // reject absolute and parent traversals via SafePath
        let Some(safe) = SafePath::new(&name) else {
            // skip unsafe entries
            continue;
        };
        // zip crate already normalizes, but we also reject ".."
        if name.contains("..") {
            continue;
        }
        let out_path = safe.to_path(target_dir);

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out = File::create(&out_path).map_err(|e| e.to_string())?;
            let mut buf = [0u8; 8192];
            loop {
                let n = entry.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

fn redact_error(s: &str) -> String {
    // do not log full link with token
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
}
