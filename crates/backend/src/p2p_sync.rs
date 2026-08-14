use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use bridge::{
    instance::InstanceID,
    message::{ExportOptions, MessageToFrontend},
    modal_action::{ModalAction, ProgressTracker, ProgressTrackerFinishType},
    safe_path::SafePath,
};
use parking_lot::RwLock;
use uuid::Uuid;

use crate::{
    BackendState,
    export::{SyncTargetPaths, is_export_junk, matches_sync_target, should_skip},
};

// ponytail: single ephemeral HTTP server per share, no new crate.
// Serve the bundle at GET /p2p/<token>. Token is the only auth.
// The bundle is built with the same filter as ExportInstance.

const DEFAULT_RELAY: &str = "https://relay.theoneand33.dev";

// ponytail: ≤64 MiB per part, safely under Cloudflare's 100 MB free limit. No config knob.
const PART_SIZE: u64 = 64 * 1024 * 1024;

// ponytail: configured relay wins, default is the shared fallback on both create and join.
fn effective_relay(backend: &BackendState) -> String {
    backend
        .config
        .write()
        .get()
        .p2p_relay_url
        .clone()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_RELAY.to_string())
}

#[derive(Debug)]
struct P2pShare {
    path: Option<PathBuf>,
    handle: Option<tokio::task::JoinHandle<()>>,
    relay_url: Option<Arc<str>>,
}

static SHARES: std::sync::LazyLock<RwLock<HashMap<Arc<str>, P2pShare>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));

fn shares() -> &'static RwLock<HashMap<Arc<str>, P2pShare>> {
    &SHARES
}

struct LimitedWriter<W> {
    inner: W,
    written: u64,
    limit: u64,
}

impl<W> LimitedWriter<W> {
    fn new(inner: W, limit: u64) -> Self {
        Self {
            inner,
            written: 0,
            limit,
        }
    }
}

impl<W: std::io::Write + std::io::Seek> std::io::Write for LimitedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let pos = self.inner.stream_position().unwrap_or(self.written);
        if pos.saturating_add(buf.len() as u64) > self.limit || self.written > self.limit {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "Bundle too large (2 GiB cap)"));
        }
        let n = self.inner.write(buf)?;
        let end = pos.saturating_add(n as u64);
        if end > self.written {
            self.written = end;
        }
        if self.written > self.limit {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "Bundle too large (2 GiB cap)"));
        }
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl<W: std::io::Seek> std::io::Seek for LimitedWriter<W> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        // Resolve the absolute target first so a seek past the cap is rejected
        // before the underlying stream moves.
        let target: i128 = match pos {
            std::io::SeekFrom::Start(off) => off as i128,
            std::io::SeekFrom::Current(delta) => self.inner.stream_position()? as i128 + delta as i128,
            std::io::SeekFrom::End(delta) => self.written as i128 + delta as i128,
        };
        if target < 0 || target > self.limit as i128 {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "Bundle too large (2 GiB cap)"));
        }
        let p = self.inner.seek(std::io::SeekFrom::Start(target as u64))?;
        if p > self.written {
            self.written = p;
        }
        Ok(p)
    }
}

pub async fn create_p2p_share(
    backend: Arc<BackendState>,
    id: InstanceID,
    options: ExportOptions,
    modal_action: ModalAction,
    use_relay: bool,
) {
    let (root_path, dot_minecraft_path) = {
        let guard = backend.instance_state.read();
        let Some(inst) = guard.instances.get(id) else {
            modal_action.set_error_message(t::instance::p2p::unknown_instance().into());
            modal_action.set_finished();
            return;
        };
        (Arc::clone(&inst.root_path), Arc::clone(&inst.dot_minecraft_path))
    };
    let sync_targets = backend.config.write().get().sync_targets.clone();

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

    // ponytail: checkbox drives relay use; configured or default relay otherwise
    let relay_url: Option<String> = if use_relay {
        Some(effective_relay(&backend))
    } else {
        None
    };
    let pages_url: Option<String> = backend.config.write().get().p2p_pages_url.clone();

    if let Some(relay) = relay_url.filter(|u| !u.trim().is_empty()) {
        let relay: Arc<str> = Arc::from(relay.trim_end_matches('/').to_string());
        let token_for_upload = Arc::clone(&token);
        let bundle_for_upload = bundle_path.clone();
        let backend_for_upload = Arc::clone(&backend);
        let modal_for_upload = modal_action.clone();
        let relay_clone = Arc::clone(&relay);
        let pages_clone = pages_url.clone();

        tokio::task::spawn(async move {
            let url = format!("{relay_clone}/p2p/{token_for_upload}");
            let upload_tracker =
                ProgressTracker::new(t::instance::p2p::uploading_to_relay().into(), backend_for_upload.send.clone());
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
                modal_for_upload.set_error_message(t::instance::p2p::bundle_too_large().into());
                modal_for_upload.set_finished();
                upload_tracker.set_finished(ProgressTrackerFinishType::Error);
                let _ = tokio::fs::remove_file(&bundle_for_upload).await;
                return;
            }

            // Chunked upload: open once, PUT ≤64 MiB parts with part headers; relay assembles on the last part.
            // Compatible fallback: no part headers is the old single-PUT path, so missing headers still work.
            let total_parts = file_len.div_ceil(PART_SIZE).max(1) as usize;
            let mut file = match tokio::fs::File::open(&bundle_for_upload).await {
                Ok(f) => f,
                Err(e) => {
                    modal_for_upload.set_error_message(format!("open bundle failed: {e}").into());
                    modal_for_upload.set_finished();
                    upload_tracker.set_finished(ProgressTrackerFinishType::Error);
                    let _ = tokio::fs::remove_file(&bundle_for_upload).await;
                    return;
                },
            };
            upload_tracker.set_total(file_len as usize);

            use tokio::io::AsyncReadExt;
            let mut buf = vec![0u8; PART_SIZE as usize];
            let mut uploaded: u64 = 0;
            let mut failure: Option<String> = None;
            for part in 0..total_parts {
                if failure.is_some() {
                    break;
                }
                let expected = if part + 1 == total_parts {
                    if file_len == 0 {
                        0
                    } else {
                        let rem = file_len % PART_SIZE;
                        if rem == 0 { PART_SIZE as usize } else { rem as usize }
                    }
                } else {
                    PART_SIZE as usize
                };
                let mut filled = 0usize;
                while filled < expected {
                    match file.read(&mut buf[filled..expected]).await {
                        Ok(0) => {
                            failure = Some(t::instance::p2p::upload_failed(redact_error("unexpected EOF")).to_string());
                            break;
                        },
                        Ok(n) => filled += n,
                        Err(e) => {
                            failure = Some(t::instance::p2p::upload_failed(redact_error(&e.to_string())).to_string());
                            break;
                        },
                    }
                }
                if failure.is_some() {
                    break;
                }
                let n = filled;
                if uploaded + n as u64 > file_len {
                    failure = Some(t::instance::p2p::upload_failed(redact_error("size mismatch")).to_string());
                    break;
                }
                let resp = backend_for_upload
                    .http_client
                    .put(&url)
                    .header("content-type", "application/zip")
                    .header("content-length", n.to_string())
                    .header("x-part-index", part.to_string())
                    .header("x-total-parts", total_parts.to_string())
                    .body(buf[..n].to_vec())
                    .send()
                    .await;
                match resp {
                    Ok(r) if r.status().is_success() => {
                        uploaded = uploaded.saturating_add(n as u64);
                        upload_tracker.set_count(uploaded as usize);
                        upload_tracker.notify();
                    },
                    Ok(r) => {
                        failure = Some(t::instance::p2p::upload_failed_status(r.status().to_string()).to_string());
                        break;
                    },
                    Err(e) => {
                        failure = Some(t::instance::p2p::upload_failed(redact_error(&e.to_string())).to_string());
                        break;
                    },
                }
            }
            if failure.is_none() && uploaded != file_len {
                failure = Some(
                    t::instance::p2p::upload_failed(redact_error(&format!("incomplete upload {uploaded}/{file_len}")))
                        .to_string(),
                );
            }
            drop(file);

            match failure {
                None => {
                    // Keep bundle for expiry window if user cancels early; otherwise relay holds copy.
                    // Original file can be removed: relay is authoritative. Do not advertise dead local link.
                    let _ = tokio::fs::remove_file(&bundle_for_upload).await;
                    shares().write().insert(
                        Arc::clone(&token_for_upload),
                        P2pShare {
                            path: None,
                            handle: None,
                            relay_url: Some(relay_clone.clone()),
                        },
                    );
                    let token_exp = Arc::clone(&token_for_upload);
                    let backend_exp = Arc::clone(&backend_for_upload);
                    tokio::task::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(30 * 60)).await;
                        let share = cancel_share_inner(&token_exp).await;
                        if let Some(s) = share
                            && let Some(relay) = s.relay_url
                        {
                            // ponytail: natural expiry is housekeeping, log-only. User-triggered cancels surface errors.
                            let _ = delete_from_relay(&backend_exp, &relay, &token_exp).await;
                        }
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
                    backend_for_upload.send.send_success(t::instance::p2p::share_uploaded());
                    modal_for_upload.set_finished();
                    upload_tracker.set_finished(ProgressTrackerFinishType::Normal);
                },
                Some(failure) => {
                    let warning = t::instance::p2p::using_local_link(failure.clone()).to_string();
                    let fallback = match create_local_share(
                        &backend_for_upload,
                        token_for_upload.clone(),
                        bundle_for_upload.clone(),
                        expires_at_ms,
                    )
                    .await
                    {
                        Ok(links) => links,
                        Err(bind_err) => {
                            let detail =
                                t::instance::p2p::relay_local_bind_failed(failure.clone(), bind_err).to_string();
                            modal_for_upload.set_error_message(detail.into());
                            modal_for_upload.set_finished();
                            upload_tracker.set_finished(ProgressTrackerFinishType::Error);
                            let _ = tokio::fs::remove_file(&bundle_for_upload).await;
                            backend_for_upload
                                .send
                                .send_error(t::instance::p2p::local_fallback_failed(failure.clone()).to_string());
                            return;
                        },
                    };
                    backend_for_upload.send.send_warning(warning);
                    modal_for_upload.set_finished();
                    upload_tracker.set_finished(ProgressTrackerFinishType::Normal);
                    backend_for_upload.send.send(MessageToFrontend::P2pShareCreated {
                        token: token_for_upload,
                        links: fallback,
                        expires_at_ms,
                    });
                    backend_for_upload.send.send_success(t::instance::p2p::share_ready_local());
                },
            }
        });
        return;
    }

    // Local-only mode
    let links = match create_local_share(&backend, Arc::clone(&token), bundle_path, expires_at_ms).await {
        Ok(l) => l,
        Err(e) => {
            modal_action.set_error_message(t::instance::p2p::bind_failed(e).into());
            modal_action.set_finished();
            return;
        },
    };
    backend.send.send(MessageToFrontend::P2pShareCreated {
        token,
        links,
        expires_at_ms,
    });
    backend.send.send_success(t::instance::p2p::share_ready());
    modal_action.set_finished();
}

async fn create_local_share(
    backend: &BackendState,
    token: Arc<str>,
    bundle_path: PathBuf,
    _expires_at_ms: i64,
) -> Result<Arc<[Arc<str>]>, String> {
    let p2p_dir = backend.directories.temp_dir.join("p2p");
    let _ = tokio::fs::create_dir_all(&p2p_dir).await;

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

    shares().write().insert(
        Arc::clone(&token),
        P2pShare {
            path: Some(bundle_path),
            handle: Some(handle),
            relay_url: None,
        },
    );

    let token_exp = Arc::clone(&token);
    tokio::task::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30 * 60)).await;
        cancel_share_inner(&token_exp).await;
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
    let tracker = ProgressTracker::new(t::instance::p2p::collecting_files().into(), backend.send.clone());
    modal_action.trackers.push(tracker.clone());

    let sync_target_paths = SyncTargetPaths::new(sync_targets);
    let mut files: Vec<(PathBuf, SafePath)> = Vec::new();
    let walker = WalkDir::new(root_path).follow_links(false);
    for entry in walker.into_iter() {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => return Err(format!("walk failed: {e}")),
        };
        if entry.file_type().is_dir() {
            continue;
        }
        if entry.path_is_symlink() {
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

    let write_tracker = ProgressTracker::new(t::instance::p2p::writing_bundle().into(), backend.send.clone());
    modal_action.trackers.push(write_tracker.clone());
    write_tracker.set_total(files.len());
    write_tracker.notify();

    let file = File::create(&bundle_path).map_err(|e| e.to_string())?;
    let limited = LimitedWriter::new(file, 2 * 1024 * 1024 * 1024);
    let mut zip = ZipWriter::new(limited);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut buf = vec![0u8; 128 * 1024];
    for (abs, rel) in &files {
        if modal_action.has_requested_cancel() {
            drop(zip);
            let _ = std::fs::remove_file(&bundle_path);
            return Err("Cancelled".into());
        }
        let mut input = File::open(abs).map_err(|e| e.to_string())?;
        if let Err(e) = zip.start_file(rel.as_str(), opts) {
            let msg = e.to_string();
            if msg.contains("Bundle too large") {
                drop(zip);
                let _ = std::fs::remove_file(&bundle_path);
                return Err(t::instance::p2p::bundle_too_large().to_string());
            }
            drop(zip);
            let _ = std::fs::remove_file(&bundle_path);
            return Err(msg);
        }
        loop {
            use std::io::Read;
            let n = input.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            if let Err(e) = zip.write_all(&buf[..n]) {
                let msg = e.to_string();
                if msg.contains("Bundle too large") {
                    drop(zip);
                    let _ = std::fs::remove_file(&bundle_path);
                    return Err(t::instance::p2p::bundle_too_large().to_string());
                }
                drop(zip);
                let _ = std::fs::remove_file(&bundle_path);
                return Err(msg);
            }
        }
        write_tracker.add_count(1);
        write_tracker.notify();
    }
    if let Err(e) = zip.finish() {
        let msg = e.to_string();
        if msg.contains("Bundle too large") {
            let _ = std::fs::remove_file(&bundle_path);
            return Err(t::instance::p2p::bundle_too_large().to_string());
        }
        let _ = std::fs::remove_file(&bundle_path);
        return Err(msg);
    }
    write_tracker.set_finished(ProgressTrackerFinishType::Normal);
    Ok(bundle_path)
}

async fn serve_p2p(listener: tokio::net::TcpListener, token: Arc<str>, bundle_path: PathBuf) {
    let expected_path = format!("/p2p/{token}");
    let semaphore = Arc::new(tokio::sync::Semaphore::new(16));
    let mut join_set: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            accept_res = listener.accept() => {
                let (mut stream, _addr) = match accept_res {
                    Ok(v) => v,
                    Err(e) => {
                        log::warn!("p2p accept failed: {e}");
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        continue;
                    },
                };
                let permit = match semaphore.clone().try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => {
                        let _ = write_status(&mut stream, 503).await;
                        continue;
                    },
                };
                let expected = expected_path.clone();
                let path_clone = bundle_path.clone();
                join_set.spawn(async move {
                    let _permit = permit;
                    let mut buf = vec![0u8; 8192];
                    use tokio::io::AsyncReadExt;
                    // Read until end of headers (\r\n\r\n) or 8k cap with 5s timeout
                    let header_end: usize = match tokio::time::timeout(Duration::from_secs(5), async {
                        let mut total = 0usize;
                        loop {
                            if total >= buf.len() {
                                let _ = write_status(&mut stream, 404).await;
                                return None::<usize>;
                            }
                            let n = match stream.read(&mut buf[total..]).await {
                                Ok(n) => n,
                                Err(_) => return None,
                            };
                            if n == 0 {
                                return None;
                            }
                            total += n;
                            if let Some(pos) = find_header_end(&buf[..total]) {
                                return Some(pos);
                            }
                            if total == buf.len() {
                                let _ = write_status(&mut stream, 404).await;
                                return None;
                            }
                        }
                    })
                    .await
                    {
                        Ok(Some(pos)) => pos,
                        Ok(None) => return,
                        Err(_) => return,
                    };
            let mut headers = [httparse::EMPTY_HEADER; 32];
            let mut req = httparse::Request::new(&mut headers);
            let Ok(httparse::Status::Complete(_)) = req.parse(&buf[..header_end]) else {
                let _ = write_status(&mut stream, 404).await;
                return;
            };
            // Only allow GET
            if req.method != Some("GET") {
                let _ = write_status(&mut stream, 405).await;
                return;
            }
            let path = req.path.unwrap_or("/");
            // Strip query string before compare
            let path_no_query = path.split('?').next().unwrap_or(path);
            if path_no_query != expected {
                let _ = write_status(&mut stream, 404).await;
                return;
            }
            let Ok(meta) = tokio::fs::metadata(&path_clone).await else {
                let _ = write_status(&mut stream, 404).await;
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
            },
            _ = join_set.join_next(), if !join_set.is_empty() => {},
        }
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

async fn write_status(stream: &mut tokio::net::TcpStream, code: u16) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    let resp = match code {
        405 => {
            b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 18\r\nConnection: close\r\n\r\nMethod Not Allowed"
                as &[u8]
        },
        503 => {
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 19\r\nConnection: close\r\n\r\nService Unavailable"
                as &[u8]
        },
        _ => b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nNot Found",
    };
    stream.write_all(resp).await
}

fn local_ipv4s() -> Vec<String> {
    use std::collections::HashSet;
    use std::net::UdpSocket;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for target in ["8.8.8.8:80", "1.1.1.1:80", "8.8.4.4:80"] {
        if let Ok(sock) = UdpSocket::bind("0.0.0.0:0") {
            let _ = sock.connect(target);
            if let Ok(addr) = sock.local_addr() {
                let ip = addr.ip().to_string();
                if ip != "0.0.0.0" && !ip.starts_with("127.") && seen.insert(ip.clone()) {
                    out.push(ip);
                }
            }
        }
    }
    out
}

// ponytail: expand a bare token against the configured relay.
fn extract_token_from_query(link: &str) -> String {
    if let Some(idx) = link.find("token=") {
        link[idx + 6..]
            .split('&')
            .next()
            .unwrap_or("")
            .split('#')
            .next()
            .unwrap_or("")
            .trim()
            .to_string()
    } else {
        String::new()
    }
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
        // Direct token= query without host, e.g. "token=abc-123&foo=bar"
        if link.starts_with("token=") || link.contains("?token=") || link.contains("&token=") {
            let token = extract_token_from_query(&link);
            if token.len() >= 8 {
                let relay = effective_relay(&backend);
                link = format!("{}/p2p/{}", relay.trim_end_matches('/'), token);
            }
        } else {
            let token = if link.contains('/') {
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
            let looks_like_token = token.len() >= 8
                && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '=');
            // If the whole input looks like a bare token, expand via relay
            if looks_like_token && link == token {
                let relay = effective_relay(&backend);
                link = format!("{}/p2p/{}", relay.trim_end_matches('/'), token);
            } else if link.contains('/') || link.contains(':') {
                // Preserve host/path inputs such as LAN links, add scheme when missing
                if !link.contains("://") {
                    link = format!("http://{}", link.trim_start_matches('/'));
                }
            }
        }
    }
    // pages URL like https://pages.example.com/?token=XYZ -> rewrite to relay
    if let Ok(parsed) = url::Url::parse(&link)
        && let Some(token) = parsed.query_pairs().find(|(k, _)| k == "token").map(|(_, v)| v.to_string())
    {
        let token = token.trim().to_string();
        if token.len() >= 8 {
            let relay = effective_relay(&backend);
            link = format!("{}/p2p/{}", relay.trim_end_matches('/'), token);
        }
    }

    let url = match url::Url::parse(&link) {
        Ok(u) => u,
        Err(e) => {
            modal_action.set_error_message(t::instance::p2p::bad_link(e.to_string()).into());
            modal_action.set_finished();
            return;
        },
    };
    if url.scheme() != "http" && url.scheme() != "https" {
        modal_action.set_error_message(t::instance::p2p::link_scheme_invalid().into());
        modal_action.set_finished();
        return;
    }
    // Reject links that don't look like /p2p/<token> unless they carry ?token=
    let has_token_param = url.query_pairs().any(|(k, _)| k == "token");
    if !has_token_param && !url.path().starts_with("/p2p/") {
        modal_action.set_error_message(t::instance::p2p::link_shape_invalid().into());
        modal_action.set_finished();
        return;
    }

    let tracker = ProgressTracker::new(t::instance::p2p::downloading_share().into(), backend.send.clone());
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
            modal_action.set_error_message(t::instance::p2p::bundle_too_large().into());
            modal_action.set_finished();
            tracker.set_finished(ProgressTrackerFinishType::Error);
            return;
        }
        tracker.set_total(len as usize);
    }
    tracker.notify();

    let tmp_dir = backend.directories.temp_dir.join("p2p").join("download");
    let _ = tokio::fs::create_dir_all(&tmp_dir).await;
    let tmp_file = tmp_dir.join(format!("{}.zip", Uuid::new_v4()));
    let mut file = match tokio::fs::File::create(&tmp_file).await {
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
    use tokio::io::AsyncWriteExt;
    let mut downloaded: usize = 0;
    let cap: usize = 2 * 1024 * 1024 * 1024;
    while let Some(chunk) = stream.next().await {
        if modal_action.has_requested_cancel() {
            drop(file);
            let _ = tokio::fs::remove_file(&tmp_file).await;
            modal_action.set_error_message("Cancelled".into());
            modal_action.set_finished();
            tracker.set_finished(ProgressTrackerFinishType::Fast);
            return;
        }
        match chunk {
            Ok(bytes) => {
                downloaded = downloaded.saturating_add(bytes.len());
                if downloaded > cap {
                    modal_action.set_error_message(t::instance::p2p::bundle_too_large().into());
                    modal_action.set_finished();
                    tracker.set_finished(ProgressTrackerFinishType::Error);
                    drop(file);
                    let _ = tokio::fs::remove_file(&tmp_file).await;
                    return;
                }
                if let Err(e) = file.write_all(&bytes).await {
                    modal_action.set_error_message(format!("write failed: {e}").into());
                    modal_action.set_finished();
                    tracker.set_finished(ProgressTrackerFinishType::Error);
                    drop(file);
                    let _ = tokio::fs::remove_file(&tmp_file).await;
                    return;
                }
                tracker.set_count(downloaded);
                tracker.notify();
            },
            Err(e) => {
                modal_action.set_error_message(format!("stream error: {e}").into());
                modal_action.set_finished();
                tracker.set_finished(ProgressTrackerFinishType::Error);
                drop(file);
                let _ = tokio::fs::remove_file(&tmp_file).await;
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

    let extract_tracker = ProgressTracker::new(t::instance::p2p::extracting_share().into(), backend.send.clone());
    modal_action.trackers.push(extract_tracker.clone());
    extract_tracker.notify();

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
            extract_tracker.set_finished(ProgressTrackerFinishType::Normal);
            modal_action.set_finished();
            backend.send.send_success(t::instance::p2p::import_done(
                target_dir.file_name().unwrap_or_default().to_string_lossy().to_string(),
            ));
        },
        Ok(Err(e)) => {
            let _ = std::fs::remove_file(&tmp_file);
            let _ = std::fs::remove_dir_all(&target_dir);
            modal_action.set_error_message(format!("Extract failed: {e}").into());
            modal_action.set_finished();
            extract_tracker.set_finished(ProgressTrackerFinishType::Error);
        },
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_file);
            let _ = std::fs::remove_dir_all(&target_dir);
            modal_action.set_error_message(format!("task failed: {e}").into());
            modal_action.set_finished();
            extract_tracker.set_finished(ProgressTrackerFinishType::Error);
        },
    }
}

pub async fn cancel_p2p_share_with_backend(backend: Arc<BackendState>, token: Arc<str>) {
    let share = cancel_share_inner(&token).await;
    let Some(share) = share else {
        return;
    };
    let Some(relay) = share.relay_url else {
        return;
    };
    if let Err(e) = delete_from_relay(&backend, &relay, &token).await {
        match e {
            RelayDeleteError::Status(status) => {
                backend.send.send_error(t::instance::p2p::delete_failed_status(status.to_string()));
            },
            RelayDeleteError::Transport => {
                backend.send.send_error(t::instance::p2p::delete_failed());
            },
        }
    }
}

async fn delete_from_relay(backend: &BackendState, relay: &str, token: &str) -> Result<(), RelayDeleteError> {
    let url = format!("{}/p2p/{}", relay.trim_end_matches('/'), token);
    match backend.http_client.delete(&url).send().await {
        Ok(r) if r.status().is_success() || r.status() == reqwest::StatusCode::NOT_FOUND => Ok(()),
        Ok(r) => {
            log::error!("failed to delete p2p share from relay: relay returned {}", r.status());
            Err(RelayDeleteError::Status(r.status()))
        },
        Err(e) => {
            log::error!("failed to delete p2p share from relay: {}", redact_error(&e.to_string()));
            Err(RelayDeleteError::Transport)
        },
    }
}

enum RelayDeleteError {
    Status(reqwest::StatusCode),
    Transport,
}

async fn cancel_share_inner(token: &str) -> Option<P2pShare> {
    let share = { shares().write().remove(token) };
    if let Some(share) = share {
        if let Some(handle) = share.handle {
            handle.abort();
        }
        if let Some(path) = &share.path {
            let _ = tokio::fs::remove_file(path).await;
        }
        log::trace!("p2p share {} cancelled", &token.chars().take(8).collect::<String>());
        Some(P2pShare {
            path: share.path,
            handle: None,
            relay_url: share.relay_url,
        })
    } else {
        None
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

    let mut total_written: u64 = 0;
    for i in 0..archive.len() {
        if modal.has_requested_cancel() {
            return Err("Cancelled".into());
        }
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();

        let Some(safe) = SafePath::new(&name) else {
            continue;
        };
        let out_path = safe.to_path(target_dir);

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
                total_written = total_written.saturating_add(n as u64);
                if written > 512 * 1024 * 1024 {
                    return Err(format!("Entry too large: {name}"));
                }
                if total_written > 4 * 1024 * 1024 * 1024 {
                    return Err("Uncompressed size too large (4 GiB cap)".into());
                }
                out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

fn redact_error(s: &str) -> String {
    if let Some(idx) = s.find("/p2p/") {
        return format!("{}[redacted]", &s[..idx]);
    }
    if let Some(idx) = s.find("?token=") {
        return format!("{}[redacted]", &s[..idx]);
    }
    if let Some(idx) = s.find("&token=") {
        return format!("{}[redacted]", &s[..idx]);
    }
    s.to_string()
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
        assert!(is_export_junk(&SafePath::new(".minecraft/.DS_Store").unwrap()));
    }

    #[test]
    fn find_header_end_works() {
        assert_eq!(find_header_end(b"GET / HTTP/1.1\r\n\r\n"), Some(18));
        assert_eq!(find_header_end(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n"), Some(27));
        assert_eq!(find_header_end(b"GET / HTTP/1.1\r\n"), None);
    }

    #[test]
    fn part_count_boundary() {
        let count = |len: u64| len.div_ceil(PART_SIZE).max(1) as usize;
        assert_eq!(count(0), 1);
        assert_eq!(count(1), 1);
        assert_eq!(count(PART_SIZE), 1);
        assert_eq!(count(PART_SIZE + 1), 2);
        assert_eq!(count(2 * PART_SIZE), 2);
    }

    #[test]
    fn limited_writer_caps_total() {
        use std::io::Write;
        let mut w = LimitedWriter::new(std::io::Cursor::new(Vec::new()), 10);
        assert!(w.write_all(&[0u8; 5]).is_ok());
        assert!(w.write_all(&[0u8; 5]).is_ok());
        assert!(w.write_all(&[0u8; 1]).is_err());
    }

    #[test]
    fn limited_writer_seek_rejects_past_cap_before_moving() {
        use std::io::{Seek, SeekFrom};
        let mut w = LimitedWriter::new(std::io::Cursor::new(vec![0u8; 8]), 10);
        assert!(w.seek(SeekFrom::Start(10)).is_ok());
        // Past the cap: rejected, and the inner stream must not move.
        assert!(w.seek(SeekFrom::Start(11)).is_err());
        assert_eq!(w.inner.stream_position().unwrap(), 10);
        // Backwards seeks within the file are valid.
        assert!(w.seek(SeekFrom::Current(-3)).is_ok());
        // Seeking before the start is invalid.
        assert!(w.seek(SeekFrom::Current(-8)).is_err());
    }

    #[test]
    fn extract_token_from_query_works() {
        assert_eq!(extract_token_from_query("token=abc-123&foo=bar"), "abc-123");
        assert_eq!(extract_token_from_query("?token=xyz#frag"), "xyz");
        assert_eq!(extract_token_from_query("foo?token=a&b"), "a");
        assert_eq!(extract_token_from_query("token="), "");
        assert_eq!(extract_token_from_query("no token here"), "");
    }
}
