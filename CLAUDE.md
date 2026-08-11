# CLAUDE.md — How to change this codebase

Read this before you edit. README is context. This file tells you how to act.

## Glossary — Use these names

- **You** = the agent reading this. You write, check, and format code.
- **We / maintainers** = the team that owns this repo. You report to us.
- **User** = the person who runs the launcher to play Minecraft.
- **Launcher** = one process, one GPUI app, two tasks: `backend` + `frontend`.
- **Instance** = one folder under `instances/<name>/`. Has `info_v1.json`, `stats_v1.json`, mods, saves. ID is `InstanceID { index, generation }`.
- **Content** = a mod, modpack, resource pack, shader. Lives in `contentlibrary/<sha1>` after `install_content`.
- **Bridge** = `crates/bridge`. Typed `MessageToBackend` / `MessageToFrontend` over `tokio::mpsc`. No JSON stdin/stdout.
- **SafePath** = validated relative path. Every untrusted modpack path must become a `SafePath` before use.

If you rename something, rename it everywhere. One name for one thing.

## What makes this project special — Do not compromise

1. **One process, typed IPC.** `main` at `crates/pandora_launcher/src/main.rs:33` creates the channel pair (`bridge::handle::create_pair` at `bridge/handle.rs:13`). Backend and frontend are Tokio tasks, not separate processes. Do not add a JSON protocol or second binary.
2. **Offline builds.** `.cargo/config.toml` replaces every registry and git source with `vendor/`. Cargo must not hit the network. If `vendor/` is stale, builds fail. This is intentional.
3. **Secrets never touch logs.** Access tokens, CurseForge key, keyring blobs must not be logged or stored in JSON. `log_reader.rs` and `reqwest_client/src/lib.rs: redact_error` enforce this.
4. **Path safety.** Untrusted zip content (modpacks) must go through `SafePath`. Never `Path::join(user_input)` into an instance.
5. **Fork that stays close to upstream.** Keep changes small and reviewable. No sweeping rewrites.

## How to make changes

### Architecture you must follow

- **Backend event loop never blocks.** `BackendState::handle` at `backend.rs:461` handles frontend messages, filesystem events, and a 1s tick. Do long work with `tokio::task::spawn`, CPU work with `spawn_blocking`.
- **Dispatch is a plain match.** Backend: `backend_handler.rs:65` matches `MessageToBackend` (~66 variants). Frontend: `processor.rs:75` matches `MessageToFrontend` (18 variants). Add a variant in `bridge/src/message.rs`, then add one arm in each match.
- **Long operations use `ModalAction`.** One `ModalAction` per operation gives progress + cancel. One-shot queries embed `tokio::sync::oneshot::Sender`. High-frequency refreshes use `Serial` dedup. Do not create a new progress system.
- **State is JSON on disk, not SQLite.** `Persistent<T>` at `persistent.rs:8` loads on demand and re-reads when the watcher marks it dirty. Writes are temp file + `sync_all` + rename. `rusqlite` is only for importing Modrinth's DB.

### Before you edit — Read the right file

| You are changing | Read first |
|---|---|
| Instances, launch, content install, syncing, auth state, update, export | `docs/agents/backend.md` |
| Pages, modals, components, skins, game output | `docs/agents/workspace.md` (Frontend) |
| Crate map and file layout | `docs/agents/workspace.md` (Workspace layout) |
| Auth / OAuth / keyring | `docs/agents/workspace.md` (Auth) |
| Data models, version/loader selection | `docs/agents/workspace.md` (Schema) |
| i18n (`t::` strings, `locales.toml`) | `docs/agents/workspace.md` (T) |
| Process spawn, sandboxing, exit codes | `docs/agents/workspace.md` (Command) |
| NBT | `docs/agents/workspace.md` (NBT) |
| HTTP client, CurseForge key redaction | `docs/agents/workspace.md` (Reqwest_client) |
| Build, vendoring, platform notes, tests | `docs/agents/workspace.md` (Build and platform) |

## Critical rules — Hard limits

- Never touch `crates/ftree`, `vendor/`, `patches/`. Tell the user. `ftree` is vendored.
- Never touch `wrapper/LaunchWrapper.jar` bytes. Rebuild with `wrapper/build.sh`.
- Never use colored gradients. Colorless gradients are fine.
- Never add a crate the user did not ask for. Use `workspace.dependencies` in root `Cargo.toml`.
- Never let an untrusted path escape the instance directory. Use `SafePath` at every trust boundary.
- Never log or store secrets. Redact in `log_reader.rs` and `redact_error`.
- Never commit unless the user asks you to commit.
- Never add a test framework. Use plain `assert` with `#[cfg(test)]`.
- Never add the `gradiants` package. Use native CSS `linear-gradient()` if you need a gradient.
- Never add a comment that does not earn its place. `ponytail:` marks a deliberate simplification and its ceiling.

## Failure modes we have hit — Do not repeat

These are from actual agent history. Each is a real revert.

1. **Swallowing network errors as 404.**
   BAD: `if err.status() == 404 { silent } else { silent }` — update check hid real failures.
   GOOD: Only silence 404 on launch. Surface other statuses with `send_error` and log detail.

2. **Hardcoded user strings.**
   BAD: `label: "Rename failed"`
   GOOD: `label: t::instance::rename_failed()` — add the key to `locales.toml`, `en` is id 0, build will fail if TOML is malformed.

3. **Blocking the event loop.**
   BAD: `std::fs::read_to_string(path)` inside `BackendState::handle`.
   GOOD: `tokio::task::spawn_blocking(move || std::fs::read_to_string(path))`.

4. **Editing generated or vendored code.**
   BAD: Edit `vendor/gpui/...` or `crates/ftree/...` to fix a UI bug.
   GOOD: Fix in `crates/frontend/...` or patch `patches/` and re-vendor with `./vendor.sh`.

5. **Logging secrets.**
   BAD: `log::error!("token {}", token)`
   GOOD: `log::error!("login failed")` — never interpolate token/key. Verify `log_reader.rs` redacts.

6. **Path escape.**
   BAD: `instance_dir.join(entry.path)` where `entry.path` comes from a zip.
   GOOD: `SafePath::new(&entry.path)?` then join.

7. **Over-scoped or under-scoped PRs.**
   BAD: One PR mixes update logic + instance rename + i18n + CI.
   GOOD: One topic per PR. Also do not stop early — if you touch 4 strings, localize all 4.

8. **Overbuilding.**
   BAD: Add a new HTTP client, new progress abstraction, or test framework to install one file.
   GOOD: Use `reqwest_client::ReqwestClient`, existing `ModalAction`, and `assert`.

9. **Breaking offline builds.**
   BAD: `cargo add foo` without running `./vendor.sh`.
   GOOD: Edit `Cargo.toml`, run `./vendor.sh`, commit `Cargo.lock`. Verify with `cargo build --offline`.

## Commands — Run these

After every change:

```
cargo check
cargo fmt
```

Before you say you are done:

```
cargo check --offline
cargo build   # or cargo build --offline if vendor is current
```

After any `Cargo.toml` change:

```
./vendor.sh
# then commit Cargo.lock
```

Release builds: `scripts/build_linux.sh vX.Y.Z`, `scripts/build_mac.sh`, `scripts/build_windows.sh`. CI runs them on `vX.Y.Z` tags.

## When to use skills — Trigger keywords

- **ponytail** — user says "ponytail", "simplest", "minimal", "YAGNI", "do less", or complains about bloat. Prefer stdlib / platform feature over new dep. One line over fifty. Ask if the task needs to exist at all.
- **agent-browser** — user says "screenshot", "open site", "check UI", "fill form", "scrape". Use for visual QA of frontend changes.
- **ste-writing** — you are writing docs, README, error messages, or comments. Use short common words, active voice, one instruction per sentence (max 20 words), no semicolons.

If the user disagrees with a preference in this file, the user wins. This file is overrideable.

## Tool and command gotchas

- Commands run offline from `vendor/`. Do not try `cargo fetch`. If `cargo check --offline` fails with missing crates, run `./vendor.sh`.
- Use `workdir` param, not `cd <dir> && cmd`.
- `main` locks single instance — do not run two launchers in tests.
- `portable` mode triggers when the executable file name contains `portable`.
- Windows stores one keyring credential per token (2560-byte limit). Do not store a blob.
- Let-chains are allowed (Edition 2024). Max width 120 per `rustfmt.toml`.

## Style — Keep it direct

We respond concise and direct, no fluff. Match that tone. Prefer facts and diffs over praise.

- Errors: `thiserror` for domain errors (`LaunchError`, `ContentInstallError`, `LoginError`). `anyhow` only inside a function body. In handlers, `send_error` to the user and `log::error!` the detail.
- Logging: `log::trace!` .. `log::error!`. Obey redaction.
- Concurrency: `parking_lot::RwLock`, `tokio::sync::Semaphore` (installs bounded at 8), `Arc<str>`, `Ustr`, `rustc_hash::FxHashMap`.
