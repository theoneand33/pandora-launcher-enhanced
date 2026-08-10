# AGENTS.md

## Important

Read this file first for every change. It gives the tech stack, the hard rules, and the core architecture. Use the routing table at the end to find the file for your task. Read that file before you edit code.

### Tech stack

- Rust Edition 2024 with Cargo resolver 3. The workspace has 11 crates.
- Frontend: GPUI from Zed plus `gpui-component` from Longbridge. The app has one GPUI app.
- Backend: async Tokio with one event loop (`BackendState::handle` at backend.rs:461).
- IPC: typed Rust enums over Tokio `mpsc` channels in the same process. The app does not use a JSON stdin/stdout protocol.
- Binary: one process. `main` creates the channel pair (`bridge::handle::create_pair` at bridge/handle.rs:13) and starts the backend and the frontend.
- Builds run offline from the `vendor/` tree. Cargo fetches nothing else (see `.cargo/config.toml`).
- Conventions: `parking_lot::RwLock`, `tokio::sync::Semaphore`, `rustc_hash::FxHashMap`, `Ustr`, `Arc<str>`, `thiserror` for domain errors, `log` crate.

### Critical rules

These rules are hard limits. Do not cross them.

- Never touch `crates/ftree`, `vendor/`, or `patches/`. Tell the user instead.
- Never touch `wrapper/LaunchWrapper.jar`. Rebuild it with `wrapper/build.sh`.
- Never use colored gradients. Gradients without color are allowed.
- Never log or store access tokens. `log_reader.rs` must redact them. The CurseForge key must never reach logs.
- Never add a dependency that the user did not request. Use the workspace dependencies in the root `Cargo.toml`.
- Never let an untrusted modpack path escape the instance directory. Use `SafePath` at each trust boundary.
- Never commit changes unless the user asks you to commit.
- Never add a test framework. Tests use plain `assert` with `#[cfg(test)]`.
- Never add a comment that does not earn its place.

### Architecture

The launcher runs as one process. The backend and the frontend run on different Tokio tasks. They exchange typed messages over in-process channels.

- Message types live in `crates/bridge/src/message.rs`: `MessageToBackend` (~66 variants) and `MessageToFrontend` (18 variants). Dispatch uses a plain `match` on the enum.
- The backend runs one event loop: `BackendState::handle` at backend.rs:461. It handles frontend messages, file-system events, and a 1-second tick. The loop must not block. Spawn long work with `tokio::task::spawn` and CPU-heavy file work with `spawn_blocking`.
- Long operations share a `ModalAction` with progress and cancel support. One-shot queries embed a `tokio::sync::oneshot::Sender` in the message. High-frequency refreshes use `Serial` deduplication.
- `crates/pandora_launcher/src/main.rs:33` is the entry point. It locks the single instance, sets up logging, and starts the runtime.

### Commands

Run these commands after you change code:

```
cargo check
cargo fmt
```

Other commands:

```
cargo build
./vendor.sh && cargo build --release --offline
./vendor.sh
```

Run `./vendor.sh` after you change `Cargo.toml`. Then commit `Cargo.lock`. For release scripts, vendoring details, and platform notes, see `docs/agents/workspace.md`.

### Code conventions

- Errors: Use `thiserror` for domain errors (`LaunchError`, `ContentInstallError`, `LoginError`, `InstanceLoadError`). Use `anyhow` only inside functions. Handler methods send the user error with `send_error` and log the detail.
- Logging: Use `log::trace!` through `log::error!`. Obey the secret redaction rules in Critical rules.
- Concurrency: Use `parking_lot::RwLock` for shared state and `tokio::sync::Semaphore` for bounded work. Use `Arc<str>` or `Arc<[T]>` to avoid clones. Use `Ustr` for interned strings and `rustc_hash::FxHashMap` or `FxHashSet` for maps.
- Style: Let-chains are allowed (Edition 2024). Format with `cargo fmt` per `rustfmt.toml` (max width 120).
- Comments: See Critical rules. `ponytail:` marks a deliberate simplification and its ceiling. New code does not need a `ponytail:` comment.

## Routing

Read the file that matches your task before you edit code.

| Task | Read |
|---|---|
| Backend: instances, launch, content install, syncing, auth state, update, export | `docs/agents/backend.md` |
| Frontend: pages, modals, components, skins, game output | `docs/agents/workspace.md` (Frontend section) |
| Crate map and file layout | `docs/agents/workspace.md` (Workspace layout) |
| Auth, OAuth flow, keyring storage | `docs/agents/workspace.md` (Auth section) |
| Data models, version and loader selection | `docs/agents/workspace.md` (Schema section) |
| i18n and locales (`t::` strings) | `docs/agents/workspace.md` (T section) |
| Process spawn, sandboxing, exit codes | `docs/agents/workspace.md` (Command section) |
| NBT parse | `docs/agents/workspace.md` (NBT section) |
| HTTP client, CurseForge key redaction | `docs/agents/workspace.md` (Reqwest_client section) |
| Build, vendoring, offline builds, platform notes, release scripts, tests | `docs/agents/workspace.md` (Build and platform, Tests) |
