# Conventions

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

See "Executable commands" in the root `AGENTS.md` for the daily commands.

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
