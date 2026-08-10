# Architecture

## Project

Pandora Launcher (Enhanced) is a Minecraft launcher for Linux, Windows, and macOS.
It is a fork of [Pandora Launcher](https://github.com/Moulberry/PandoraLauncher).
The UI is built on GPUI (Zed's GUI framework). The backend is async Tokio.
It uses Rust Edition 2024 and Cargo resolver 3.

Unique features of this fork:

- Offline and cracked accounts with a local skin server (authlib-injector).
- Extended instance export (shaders, screenshots, backups).
- Cross-instance file syncing.
- Mod install and update from Modrinth and CurseForge.
- Secure credential storage in the OS keyring.
- Custom game output window.

## High-level architecture

The launcher is a **single process**. The backend and the frontend run in the
same process on different Tokio tasks. They communicate through typed Rust
enum messages over in-process Tokio `mpsc` channels. There is no JSON protocol
over stdin/stdout.

- `crates/pandora_launcher/src/main.rs` is the entry point. It builds the
  channel pair (`bridge::handle::create_pair`), then starts the backend and
  the frontend.
- The backend runs one event loop: `BackendState::handle` (backend.rs:461).
  It dispatches frontend messages, filesystem events, and a 1-second tick.
- The frontend owns all GPUI state. It renders pages and sends requests to the
  backend. It receives updates as pushed messages.

Message types are shared in `crates/bridge/src/message.rs`:

- `MessageToBackend` (frontend to backend requests, ~66 variants).
- `MessageToFrontend` (backend to frontend updates, ~34 variants).

Dispatch is a plain `match` on the enum, not a request/response ID pattern.
Long-running operations share a `ModalAction` (progress plus cancel token).
One-shot queries embed a `tokio::sync::oneshot::Sender` in the message.
High-frequency UI refreshes use `Serial`-based deduplication.

The backend must never block its event loop. Long work goes into
`tokio::task::spawn`. CPU-heavy file work goes into `spawn_blocking`.

## Entry points

- `crates/pandora_launcher/src/main.rs:33` — `main`, single-instance lock,
  runtime, logging.
- `backend::start` — backend.rs:108.
- `BackendState::handle` — backend.rs:461 (event loop).
- `handle_message` — backend_handler.rs:65 (message dispatch).
- `Processor::process` — processor.rs:75 (frontend message dispatch).
- `Launcher::launch` — launch/mod.rs:124.
- `install_content` — install_content.rs:129.
- `create_pair` — bridge/handle.rs:13 (channel pair).
