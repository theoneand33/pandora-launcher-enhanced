# AGENTS.md

## Important

Read this section first. The rest of this file is the routing index.

### Tech stack

- Rust Edition 2024. Cargo resolver 3. Workspace of 11 crates.
- Frontend: GPUI (Zed) plus `gpui-component` (Longbridge). Single GPUI app.
- Backend: async Tokio, one event loop (`BackendState::handle`, backend.rs:461).
- IPC: in-process typed Rust enums over Tokio `mpsc` channels. No JSON
  stdin/stdout protocol.
- Binary: single process. `main` builds the channel pair
  (`bridge::handle::create_pair`, bridge/handle.rs:13), then starts the
  backend and the frontend.
- All builds are offline through the `vendor/` tree. Cargo fetches nothing
  else (`.cargo/config.toml`).
- Conventions: `parking_lot::RwLock`, `tokio::sync::Semaphore`,
  `rustc_hash::FxHashMap`, `Ustr`, `Arc<str>`, `thiserror` domain errors,
  `log` crate (never print secrets).

### Executable commands

Run `cargo check` after every change:

```
cargo check
```

Build (dev):

```
cargo build
```

Build (release, offline):

```
./vendor.sh && cargo build --release --offline
```

Format (max width 120, config in `rustfmt.toml`):

```
cargo fmt
```

After any `Cargo.toml` change: run `./vendor.sh` and commit `Cargo.lock`.
See "Vendoring and offline builds" in `docs/agents/conventions.md`.

### Critical rules

These are hard boundaries. Do not cross them.

- **Never touch `crates/ftree/src`.** It is a vendored third-party crate.
- **Never edit `vendor/` by hand.** Regenerate with `./vendor.sh`.
- **Never remove or alter `patches/`.** Required for `zbus` to link.
- **Never touch `wrapper/LaunchWrapper.jar`.** Rebuild it through
  `wrapper/build.sh`.
- **Never use colored gradients.** Non-colored gradients are allowed.
- **Never log or store access tokens.** `log_reader.rs` redaction is
  mandatory. The CurseForge key must never reach logs.
- **Never add a dependency that the user did not request.** Prefer the
  workspace dependencies in the root `Cargo.toml`.
- **Never let untrusted modpack paths escape the instance directory.** Use
  `SafePath` at every path trust boundary.
- **Never commit changes unless the user asks you to.**
- **Never add a test framework.** Tests are plain `assert`-based
  `#[cfg(test)]` modules.
- **Never add comments that do not earn their place.**
- **Do not edit `crates/ftree`, `vendor/`, or `patches/`.** Even to fix a
  build failure. Tell the user instead.

Full rules are repeated in `docs/agents/conventions.md` under "Prohibited".

## Read these files

The detailed reference lives in `docs/agents/`. This file is the index.
Read only the files that cover your task. Start with
`docs/agents/architecture.md` if you are new to the codebase.

| Task | Read |
|---|---|
| Any change to the repo | This file (the Important section) |
| Backend logic: instances, launching, content install, syncing, auth state, update, export | `docs/agents/backend.md` |
| Frontend UI: pages, modals, components, skins, game output window | `docs/agents/workspace.md` (frontend section) |
| High-level flow, IPC, message dispatch, entry points | `docs/agents/architecture.md` |
| Where a file lives: crate map and per-crate layout | `docs/agents/workspace.md` |
| Auth, OAuth flow, keyring credential storage | `docs/agents/workspace.md` (auth section) |
| Data models, version selection, loader selection | `docs/agents/workspace.md` (schema section) |
| i18n and locales (`t::` strings) | `docs/agents/workspace.md` (t section) |
| Process spawning, sandboxing, exit codes | `docs/agents/workspace.md` (command section) |
| NBT parsing | `docs/agents/workspace.md` (nbt section) |
| HTTP client, CurseForge key redaction | `docs/agents/workspace.md` (reqwest_client section) |
| Build, dependencies, vendoring, release, platform notes | `docs/agents/conventions.md` |
| Writing tests | `docs/agents/conventions.md` |

For backend or frontend work, also read `docs/agents/architecture.md` to see
how the two sides connect.
