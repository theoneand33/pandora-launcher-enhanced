# P2P Instance Sync — Design

## Goal

Share an instance (or parts of it: mods, config, resourcepacks, shaderpacks, saves) between two computers with a link. One computer hosts the data. The other computer fetches the data with the link. No central storage. The link works for LAN, VPN, and with a domain reverse proxy.

Non-goal: continuous background sync. Non-goal: DHT or libp2p.

## Constraints from this codebase

* One process, typed IPC. Add variants to `bridge/src/message.rs`, then one arm in each `match` (`backend_handler.rs`, `processor.rs`).
* Offline builds. Do not add crates. Use `tokio`, `reqwest`, `zip`, `sha1`/`sha2` that already exist. `httparse` already in `Cargo.lock`.
* `SafePath` at every trust boundary. The receiver must validate every entry path from the bundle.
* Secrets never touch logs. Tokens and URLs are secrets. Redact them.
* `ModalAction` for progress and cancel. Long work runs with `tokio::task::spawn`.

## Choice — local HTTP bundle + relay for domain

* Bundle via `p2p_sync.rs:create_bundle_blocking` (same filter as `export.rs`, `follow_links(false)`, skip symlinks). A share is a temporary zip built from an instance with an `ExportOptions` filter. The host either serves the zip over an ephemeral HTTP server on a random port (LAN) or uploads it to a relay.
* ponytail ceiling: two modes. LAN mode needs no server. Domain mode uses the separate website (this doc's split). The launcher never requires the domain to be a reverse proxy to the host; that was V1. Correct split: the domain website is independent.
* Relay mode (separate website): launcher does `PUT /p2p/<token>` to `p2p_relay_url` (Coolify). The relay stores the zip 30 min. Link becomes `https://relay.example.com/p2p/<token>` and (optionally) `https://pages.example.com/?token=<token>` (GitHub Pages). The Pages site is static and only fetches from the relay. No binary touches GitHub.

## Link format

```
http://<host>:<port>/p2p/<token>
```

* `<host>` is a LAN IP or a domain that proxies to the host. The launcher lists all non-loopback IPv4 addresses and shows them with the link.
* `<token>` is `Uuid::new_v4()` hex (122 bits). Unpredictable. No bearer header required. The token is the path. Possession of the link is authorization.
* Token is valid for 30 minutes. The host serves the bundle for the full window (multiple downloads allowed) and deletes it on expiry. The host can revoke early with Cancel. Single-use is not enforced.

Future: `pandora-sync://` custom scheme that opens the launcher via OS URL handler. Not required for V1.

## Data flow

### Share (host)

1. Frontend modal: pick Instance, tick parts (mods, config, resourcepacks, shaderpacks, saves, screenshots, etc). Reuse the same check boxes as `ExportInstance` (8 toggles plus `include_synced`).
2. Frontend sends `MessageToBackend::CreateP2pShare { id, options, modal_action }`.
3. Backend `p2p_sync::create_p2p_share`:
   a. Lock `instance_state`, snapshot `InstanceConfiguration` and root paths. Release lock.
   b. `create_bundle_blocking` on `spawn_blocking` (same filter as export, `follow_links(false)`, skip symlinks). Write filtered files to `temp/p2p/<token>.zip`. Report progress through `ModalAction` trackers.
   c. Bind `tokio::net::TcpListener` to `0.0.0.0:0`. Get port. Store entry `(token -> PathBuf)` in a process-wide `RwLock<HashMap>` with expiry task.
   d. Spawn `serve_p2p` task: loop `accept` with backoff on error, parse request head (8 KiB) with `httparse`, check `GET /p2p/<token>`. On match, stream the file with `tokio::fs::File`. On mismatch, 404. Log only token prefix (first 8 chars).
   e. Build link(s): one per local IPv4 (`local_ipv4s()`) plus `http://127.0.0.1:<port>/p2p/<token>` for local test. Send `MessageToFrontend::P2pShareCreated { token, links, expires_at }`. Also show notification "Share ready. Link expires in 30 min. Keep launcher open."
4. Frontend shows copyable links and Cancel button. Cancel sends `MessageToBackend::CancelP2pShare { token }`. QR is deferred.

### Join (peer)

1. Frontend modal: paste link (or token), optional instance name for new instance. Deferred: update of existing instance and merge vs replace toggle.
2. Frontend sends `MessageToBackend::JoinP2pShare { link, target_name, modal_action }`.
3. Backend `p2p_sync::join_p2p_share`:
   a. Validate link is http/https, parse with `url::Url`. Enforce `SafePath` on zip entry paths later; reject absolute paths. `SafePath` already rejects `..` traversal.
   b. Download with `backend.http_client` streaming to `temp/p2p/download/<token>.zip` via `tokio::fs::File`. Support cancel via `modal_action.has_requested_cancel()`. Show ProgressTracker total from Content-Length.
   c. Verify file size limit (2 GiB hard cap) to avoid OOM. Unpack via `spawn_blocking` to a new instance dir `instances/<name>/.minecraft` (create folder). Zip bomb guards: 100k entry cap, 4 GiB uncompressed cap.
   d. Finish `ModalAction`, send success notification, send `Refresh`.

## Storage and lifecycle

* Host bundle lives under `directories.temp_dir.join("p2p")`. No `contentlibrary` deduplication. Deleted on serve complete, cancel, expiry, or launcher exit.
* In-memory map: `parking_lot::RwLock<FxHashMap<Arc<str>, P2pShare>>`. Guard protects the `serve_p2p` task handle; `CancelP2pShare` aborts it and removes file.

## Security

* Token in path, not logged. `log_reader::redact` already strips `key=`; add redaction for `/p2p/<token>` in that file.
* Entry path validation: every file path from zip becomes `SafePath::new`. Reject if invalid. Never `Path::join(user_input)` into instance.
* Host binds to all interfaces only while a share is active. No persistent listener.
* Optional auth (V2): `Authorization: Bearer <token>` plus path token to avoid referrer leaks. V1 keeps path-only for simplicity and QR friendliness.

## Frontend

* Add modal `modals/p2p_sync.rs` (share) and `modals/p2p_join.rs` (join). Reuse check boxes from `export_instance.rs`.
* Add entry points: instance page overflow menu "Share via link", global sidebar "Join from link".
* `processor.rs` stores last `P2pShareCreated` in a new entity so the share modal can render links without polling.

## Domain use — separate website (GitHub Pages vs Coolify)

The domain website lives outside the launcher repo and has two deployment targets.

* **GitHub Pages (`p2p_pages_url`):** static only. Deploy `~/pandora-sync/index.html` to `username.github.io/pandora-sync`. Set `<meta name="p2p-relay" content="https://relay.example.com">`. No storage on GitHub. The page resolves `?token=<token>` or a pasted bare token to `GET https://relay.example.com/p2p/<token>`.
* **Coolify (`p2p_relay_url`):** self-hosted relay that stores zips. Spec: `PUT /p2p/<token>` stores (2 GiB cap, TTL 30 min), `GET /p2p/<token>` returns `application/zip`, `DELETE /p2p/<token>` optional. Implement as one binary (Axum) or Caddy + File API. See `~/pandora-sync/README.md` and `~/pandora-sync/relay/` for `docker-compose.yml`.

Launcher config (`BackendConfig`):

```json
{ "p2p_relay_url": "https://relay.example.com", "p2p_pages_url": "https://username.github.io/pandora-sync" }
```
* `p2p_relay_url` empty → LAN mode (ephemeral `0.0.0.0:0` server, link `http://<lan-ip>:<port>/p2p/<token>`).
* `p2p_relay_url` set → launcher uploads via `PUT`, shows both `https://relay.../p2p/<token>` and `https://pages.../?token=<token>` plus a local fallback link.

## Steps to ship

1. `crates/bridge/src/message.rs`: `CreateP2pShare`, `JoinP2pShare { link, target_name }`, `CancelP2pShare`, and `P2pShareCreated { token, links, expires_at }`.
2. `crates/backend/src/p2p_sync.rs`: `create_p2p_share`, `join_p2p_share`, `cancel_p2p_share`, `serve_p2p` (httparse + tokio fs) plus relay upload branch.
3. `crates/schema/src/backend_config.rs`: `p2p_relay_url` + `p2p_pages_url`.
4. `~/pandora-sync/` (separate git repo): `index.html` + `README.md` + `relay/` (Coolify). Deploy `index.html` to Pages, `relay/` to Coolify.
5. `crates/frontend/src/modals/p2p_sync.rs` + `processor.rs` plumbing.
6. Tests: plain `assert` with `#[cfg(test)]` in `p2p_sync.rs` for `is_valid_link`, `extract_token`, and zip path rejection. No new test framework.

## Verification

* `cargo check --offline` before merge. `cargo fmt`.
* Manual: launch two launchers on same LAN. Share mods-only of an instance. Join on peer with new name. Check `mods/` hashes match, `.aux.json` preserved, and launch succeeds offline.
* Manual NAT test: share with domain reverse proxy, join from outside network.

## Deferred (ponytail debt)

* Delta sync (rsync-style) for saves between peers. Current bundle re-sends full filtered files; acceptable for mods (100–300 MiB). Document ceiling: large `saves/` (2+ GiB) will be slow on upload-limited links.
* End-to-end encryption (age). Current link secrecy is the only protection. Document it.
* Relay auth (upload token) if the relay is public.
