# AGENTS.md

## Project
This is a Rust workspace. It contains Pandora Launcher (Enhanced), a Minecraft launcher built on GPUI/Zed. It uses Rust Edition 2024 and Cargo resolver 3.

## Workspace
The `crates/` folder has these members: `pandora_launcher` (binary), `backend`, `frontend`, `bridge`, `auth`, `command`, `nbt`, `schema`, `reqwest_client`, `ftree`, `t`. The root `Cargo.toml` defines the shared dependencies and the build profiles.

## Commands
- Check the code: `cargo check`
- Build (dev): `cargo build`
- Build (release, offline): `./vendor.sh && cargo build --release --offline`
- Format the code: `cargo fmt` (config: `rustfmt.toml`, max_width 120)
- Platform scripts: `scripts/build_linux.sh`, `scripts/build_mac.sh`, `scripts/build_windows.sh`

## Conventions
- Do not add new dependencies without need. Use the workspace dependencies in the root `Cargo.toml`.
- Keep the `image` crate features minimal (`png`, `jpeg`, `bmp`, `gif`, `webp` only).
- Vendor builds use `./vendor.sh`. Commit the `Cargo.lock` changes.

## Prohibited
- Do not use the `gradiants` package. Use native CSS gradients (`linear-gradient()`, `conic-gradient()`).
