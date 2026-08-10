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
See "Vendoring and offline builds" below.

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

Full rules are repeated at the bottom of this file under "Prohibited".

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

Conventions apply to every change. They live in this file, not in a separate
reference. See the sections below: Testing, Code conventions, Adding a
dependency, Vendoring and offline builds, Commands, Platform notes, and
Prohibited.

For backend or frontend work, also read `docs/agents/architecture.md` to see
how the two sides connect.

## Conventions

These sections apply to every change. This is not a reference you read on
demand. Follow it always.

## Testing

Tests are sparse by design.

- `backend/src/skin_server.rs:194` has three `#[tokio::test]` tests.
- `reqwest_client` has two tests for proxy URL handling.
- `ftree` has a table-driven unit suite.
- The `t` crate treats its `locales.toml` compile-error path as its test.

Do not add a test framework. Use plain `assert`-based `#[cfg(test)]` modules.

## Code conventions

- **Errors**: `thiserror` for domain errors (`LaunchError`, `ContentInstallError`,
  `LoginError`, `InstanceLoadError`). `anyhow` only inside functions. Handler
  methods send the user-facing error with `send_error` and log the detail.
- **Logging**: `log::trace!` through `log::error!`. Never print secrets.
- **Concurrency**: `parking_lot::RwLock` for shared state. `tokio::sync::Semaphore`
  for bounded work. `Arc<str>`/`Arc<[T]>` to avoid clones. `Ustr` for interned
  strings. `rustc_hash::FxHashMap`/`FxHashSet` for hash maps.
- **Style**: let-chains are fine (Edition 2024). `cargo fmt` uses
  `rustfmt.toml`, max width 120.
- Do not add comments unless they earn their place. `ponytail:` comments mark
  a deliberate simplification and name its ceiling.
- New code does not need a `ponytail:` comment.

## Adding a dependency

- Prefer the workspace dependencies in the root `Cargo.toml`.
- Do not add new dependencies without need.
- Keep the `image` crate features minimal: `png`, `jpeg`, `bmp`, `gif`, `webp`
  only. The vendored `gpui` crates are patched to the same set.
- After changing dependencies, run `./vendor.sh` and commit `Cargo.lock`.

## Vendoring and offline builds

`.cargo/config.toml` replaces crates.io and all git sources with the `vendor/`
directory. Cargo will not fetch anything unless the vendor tree is up to date.

After any `Cargo.toml` change, refresh the vendor tree:

```
./vendor.sh
```

`vendor.sh` runs `cargo vendor`, copies `gpui-component-assets`, and re-applies
the trimmed `image` features to the vendored gpui crates.

The `patches/` folder contains `zbus-lockstep` and its macros, wired through
`[patch.crates-io]`.

## Commands

See "Executable commands" in the Important section for the daily commands.

Release builds per platform use `scripts/build_linux.sh`,
`scripts/build_mac.sh`, and `scripts/build_windows.sh`. These scripts require
a version argument. CI runs them through `.github/workflows/cd.yml` on tags
matching `vX.Y.Z`.

`cargo check` after every change.

## Platform notes

- The backend builds `rusqlite` bundled on Windows and system on
  Linux/macOS. System sqlite needs `libsqlite3-dev` (Linux) or the equivalent.
- Windows-only deps (`junction`, `mslnk`, `windows`) stay behind
  `cfg(target_os = "windows")`.
- `build.rs` in `pandora_launcher` embeds the Windows icon via `winresource`.
- `crates/backend/assets/authlib-injector.jar` is embedded with
  `include_bytes!` for offline accounts.
- Portable mode activates when the executable filename contains "portable".

## Prohibited

Full rules. These are hard boundaries.

- Do not use colored gradients. Non-colored gradients are allowed.
- Do not log or store access tokens. Redaction is mandatory in logs.
- Do not add unrequested dependencies.
- Do not let untrusted modpack paths escape the instance directory. Use
  `SafePath` at every path trust boundary.
- Do not edit `crates/ftree`, `vendor/`, or `patches/`. Even to fix a build
  failure. Tell the user instead.
- Do not touch `wrapper/LaunchWrapper.jar`. Rebuild it through
  `wrapper/build.sh`.
- Do not commit changes unless the user asks you to.
- Do not add a test framework.
- Do not add comments that do not earn their place.
