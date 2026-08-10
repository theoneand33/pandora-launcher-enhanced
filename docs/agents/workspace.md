# Workspace layout

The root `Cargo.toml` defines all shared dependencies and the build profiles.
The default member is `crates/pandora_launcher`. The workspace uses four git
dependencies: `gpui` and `gpui_platform` from Zed, and `gpui-component` and
`gpui-component-assets` from Longbridge.

| Crate | Role |
|---|---|
| `pandora_launcher` | Binary. Entry point, CLI, logging, single-instance lock, panic hooks, deadlock detector. |
| `frontend` | GPUI UI. Pages, modals, components, skin renderer, message processor. |
| `backend` | All launcher logic. Instances, content, launching, syncing, auth state, update, skins. |
| `bridge` | Shared message enums and channel handles. No logic. |
| `auth` | Microsoft OAuth login flow and OS keyring credential storage. |
| `command` | Process spawning layer. Normal, elevated, and sandboxed spawn. |
| `schema` | Serde data models for Mojang, Modrinth, CurseForge, Forge, Fabric APIs. |
| `nbt` | Minecraft NBT binary format encode/decode. |
| `t` | Compile-time internationalization. Generates code from `locales.toml`. |
| `ftree` | Vendored Fenwick tree crate (prefix sums for virtualized lists). |
| `reqwest_client` | Adapter that exposes the GPUI HTTP client to the rest of the app. |

## crates/frontend

`start` (lib.rs:75) sets up GPUI, fonts, theme, and key bindings. The
`Processor` (processor.rs:75) matches every `MessageToFrontend` variant and
routes it to entity stores. `LauncherRoot` (root.rs:29) is the top render.
`LauncherUI` (ui.rs:42) holds page state and navigation history.

Notable files:

- `pages/` — one file per page: instances, Modrinth browse and project pages,
  CurseForge, skins, syncing, import.
- `pages/instance/` — instance page with subpages: content, settings, logs,
  quickplay.
- `modals/` — create instance, install, export, settings, accounts.
- `entity/` — frontend-side stores for instances, accounts, metadata.
- `component/` — reusable UI components.
- `game_output/mod.rs` — the Minecraft console window.
- `skin_renderer.rs` — software 3D skin renderer (no GPU).
- `skin_thumbnail_cache.rs` — async thumbnail cache for skins.
- `interface_config.rs` — persisted UI preferences (`interface.json`).
- `icon.rs` — icons declared with the `icon_named!` macro over
  `assets/icons/*.svg`.

## crates/backend

`lib.rs` re-exports `backend::*`. The only public function is `backend::start`.
Submodules include `launch`, `launcher_import`, and `metadata`.

Core files:

- `backend.rs` — state, event loop, instance CRUD, login, prelaunch.
- `backend_handler.rs` — the giant `match` that dispatches `MessageToBackend`.
- `backend_filesystem.rs` — converts `notify` events into dirty marks.
- `instance.rs` — the instance model and its file loading.
- `install_content.rs` — the content download and install pipeline.
- `mod_metadata.rs` — fingerprinting mods and parsing their metadata.
- `launch/mod.rs` — the full Minecraft launch pipeline.
- `persistent.rs` — JSON file persistence.
- `directories.rs` — the launcher directory layout.
- `syncing.rs` — cross-instance file sync.
- `skin_server.rs` — local HTTP skin and Yggdrasil server for offline play.
- `skin_manager.rs` — download, cache, and dedupe account skins.
- `update.rs` — self-update with signature verification.
- `export.rs` — instance export to `.mrpack` or CurseForge `.zip`.
- `log_reader.rs` — log tailing and secret redaction.
- `server_list_pinger.rs` — Minecraft server list ping.
- `shortcut.rs` — desktop shortcut creation per platform.

## crates/bridge

Pure data types shared by both sides.

- `message.rs` — the two message enums.
- `handle.rs` — channel-pair factory and `FrontendHandle`/`BackendHandle`.
- `modal_action.rs` — shared progress/cancel handle for long operations.
- `instance.rs` — shared instance types (`InstanceID`, summaries, `ContentType`).
- `safe_path.rs` — validated relative paths. Trust boundary for untrusted
  modpack paths.
- `serial.rs` — monotonic serial for deduplicating notifications.
- `quit.rs` — `QuitCoordinator` for coordinated shutdown.
- `keep_alive.rs`, `notify_signal.rs` — liveness and coalescing signals.
- `import.rs` — other-launcher import metadata.

## crates/command

Replaces `std::process` for the launcher.

- `spawner.rs` — three spawn modes: normal, elevated, sandboxed. All spawns go
  through one dedicated OS thread.
- `process.rs` — `PandoraProcess` with graceful/forceful termination.
- `exit_status.rs` — decodes raw exit codes (unix wait status, Windows NTSTATUS).
- `command.rs` — `PandoraCommand` builder.
- `path_cache.rs` — TTL'd PATH lookup.

The backend builds the Java launch command into a `PandoraCommand`
(launch/mod.rs:2396) and spawns it normal or sandboxed. Sandboxing uses
`bwrap` plus `xdg-dbus-proxy` on Linux.

## crates/auth

Microsoft consumer OAuth2 plus Xbox Live plus Minecraft services.

The chain in `authenticator.rs`:

1. `create_authorization` builds the authorize URL with PKCE and CSRF state.
2. `serve_redirect::start_server` catches the redirect on `127.0.0.1:3160`.
3. `finish_authorization` exchanges the code for MSA tokens.
4. `authenticate_xbox`, `obtain_xsts`, `authenticate_minecraft` climb the chain.
5. `get_minecraft_profile` fetches UUID, skins, and capes.

`AccountCredentials::stage` returns the highest still-valid token, so a stale
app redoes only the broken leg of the chain.

`secret.rs` stores credentials in the OS keyring. It falls back to a JSON file
when the keyring is unavailable. Windows stores one credential per token
because Windows Credential Manager has a 2560-byte blob limit.

## crates/schema

Serde data models. Mojang-remote types carry
`#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]`.

Key types and logic:

- `version.rs` — the full Minecraft version JSON.
- `version_manifest.rs` — `version_manifest_v2.json`.
- `instance.rs` — `InstanceConfiguration` plus loader version selection:
  `determine_fabric_loader_version`, `determine_neoforge_loader_version`,
  `determine_forge_loader_version`.
- `forge.rs`, `fabric_launch.rs` — installer profiles and launch meta.
- `loader.rs` — the `Loader` enum (Vanilla, Fabric, Forge, NeoForge).
- `modrinth.rs`, `curseforge.rs` — API models. The CurseForge key is hardcoded.
- `mrpack.rs` — `modrinth.index.json` inside a `.mrpack`.
- `java_runtimes.rs`, `java_runtime_component.rs` — Mojang JRE manifests.
- `text_component.rs` — flattens Minecraft text-component JSON into a plain
  string with style runs.
- `unique_bytes.rs` — interned, refcounted byte slices for dedup.

## crates/nbt

Minecraft Named Binary Tag. `NBT` is an arena of nodes. `decode.rs` has
`read_named` and `read_protocol`. Decode protects against nesting depth
(cap 512) and total size (cap ~2 MiB). Duplicate compound keys are rejected.

## crates/t

Zero-dependency i18n. `build.rs` parses `locales.toml` and writes generated
Rust to `OUT_DIR`. The generated code exposes one function per locale key,
such as `t::modrinth::category::fabric()` and `t::instance::incompatible(n)`.
Interpolated keys take typed arguments. `_short`-suffixed keys become
`(short: bool)` variants.

Adding a locale means editing `locales.toml` only. Current locales: en, de,
hu, sv, ru. `en` is hardcoded to language id 0. A TOML structure error makes
`build.rs` write a `compile_error!` into `src/lib.rs`, which fails the build.

## crates/ftree

Vendored `ftree` crate v1.2.0. Do not edit it. The frontend uses it for
virtualized list heights.

## crates/reqwest_client

`ReqwestClient` implements GPUI's `http_client::HttpClient` trait. All HTTP
goes through the GPUI `reqwest` fork. TLS trusts the OS root store via
`rustls_platform_verifier`. `redact_error` strips `key=...` from request URLs
so the CurseForge key never reaches logs.

## Build and platform

This section covers vendoring, offline builds, platform details, and release builds.

### Vendoring and offline builds

`.cargo/config.toml` replaces crates.io and all git sources with the `vendor/` directory. Cargo does not fetch from the network when the vendor tree is current.

After any `Cargo.toml` change, refresh the vendor tree and commit `Cargo.lock`:

```
./vendor.sh
```

`vendor.sh` runs `cargo vendor`, copies `gpui-component-assets`, and reapplies the trimmed `image` features to the vendored `gpui` crates. The `patches/` directory holds `zbus-lockstep` and its macros through `[patch.crates-io]`.

### Dependencies

Do not add a dependency unless the user requests it. Use workspace dependencies from the root `Cargo.toml`.

Keep `image` crate features minimal: `png`, `jpeg`, `bmp`, `gif`, and `webp` only. The vendored `gpui` crates use the same set.

### Commands

Daily commands:

```
cargo check
cargo fmt
cargo build
./vendor.sh && cargo build --release --offline
```

Release builds use per-platform scripts with a version argument: `scripts/build_linux.sh`, `scripts/build_mac.sh`, and `scripts/build_windows.sh`. CI runs these scripts through `.github/workflows/cd.yml` on tags that match `vX.Y.Z`.

### Platform notes

- The backend builds `rusqlite` bundled on Windows and as a system library on Linux and macOS. Linux needs `libsqlite3-dev` or the equivalent.
- Windows-only dependencies (`junction`, `mslnk`, `windows`) stay behind `cfg(target_os = "windows")`.
- `crates/pandora_launcher/build.rs` embeds the Windows icon through `winresource`.
- `crates/backend/assets/authlib-injector.jar` is embedded with `include_bytes!` for offline accounts.
- Portable mode activates when the executable file name contains `portable`.

### Entry points

- `crates/pandora_launcher/src/main.rs:33` — `main`, single-instance lock, runtime, logging.
- `backend::start` — backend.rs:108.
- `BackendState::handle` — backend.rs:461, the event loop.
- `handle_message` — backend_handler.rs:65, message dispatch.
- `Processor::process` — processor.rs:75, frontend message dispatch.
- `Launcher::launch` — launch/mod.rs:124.
- `install_content` — install_content.rs:129.
- `create_pair` — bridge/handle.rs:13, channel pair.

## Tests

Tests are sparse by design. Use plain `assert` with `#[cfg(test)]`. Do not add a test framework.

- `backend/src/skin_server.rs:194` has three `#[tokio::test]` tests.
- `reqwest_client` has two tests for proxy URL handling.
- `ftree` has a table-driven unit suite.
- The `t` crate uses its `locales.toml` compile-error path as its test. A structure error in `locales.toml` makes `build.rs` write `compile_error!` into `src/lib.rs` and the build fails.
