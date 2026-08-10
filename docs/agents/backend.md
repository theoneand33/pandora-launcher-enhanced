# Backend

This file covers the launcher logic in `crates/backend`. Read
`docs/agents/architecture.md` first to see how the backend connects to the
frontend.

## Data and persistence

There is **no SQLite** for launcher state. `rusqlite` reads only the Modrinth
app's profile DB during import.

All launcher state is JSON. `Persistent<T>` (persistent.rs:8) loads on demand,
re-reads from disk when a filesystem watcher marks the file dirty, and writes
atomically via temp file, `sync_all`, and rename.

Files under the launcher directory:

- `config.json` — `Persistent<BackendConfig>`.
- `accounts.json` — `Persistent<BackendAccountInfo>`. Tokens live in the
  keyring, never here.
- `instances/<name>/info_v1.json` — `Persistent<InstanceConfiguration>`.
- `instances/<name>/stats_v1.json` — `Persistent<InstanceStats>`.
- `interface.json` — frontend UI preferences.
- `contentlibrary/<prefix>/<sha1>[.ext]` — content-addressed file store.
- `contentmeta/` — mod source tracking and update data.
- `metadata/` — cached remote metadata.
- `skins/` — the skin library.
- `temp/` — natives and the wrapper jar.
- `panic-reports/` — panic reports from the panic hook.

## Instances

An instance is one folder under `instances/`. `Instance` (instance.rs:48)
holds `Persistent<InstanceConfiguration>` and `Persistent<InstanceStats>`,
process handles, and cached worlds/servers/content summaries. Each instance
gets a generational `InstanceID { index, generation }` in an `IdSlab`.

`Instance::status` is one of `Running`, `Stopping`, `Launching`, `NotRunning`.

## Launching Minecraft

Flow from `StartInstance`:

1. `start_instance` (backend_handler.rs:2288) guards against double launch.
2. `get_login_info` runs the login flow, with offline fallback.
3. `prelaunch` (backend.rs:774) applies syncing, then `prelaunch_setup_mods`
   renames the live `mods/` to `original_mods/` and writes the frozen mods
   from the content library.
4. `Launcher::launch` (launch/mod.rs:124) downloads assets, libraries, and
   Java, then spawns the game.
5. When the game stops, `restore_mods_folder_if_stopped` restores
   `original_mods/`.

The wrapper jar (`wrapper/LaunchWrapper.jar`) is the first classpath entry.
Its main class is `com.moulberry.pandora.LaunchWrapper`. Game arguments are
not passed on the command line. They stream to the child's stdin as
`arg\n<value>\n...launch\n<mainclass>\n`.

Offline mode starts the local skin server and injects
`-javaagent:authlib-injector=<yggdrasil-url>`.

## Content pipeline

`install_content` (install_content.rs:129) installs Modrinth, CurseForge,
direct URL, and local file downloads. Files download into the content library
with sha1 and size verification under a cross-process `Lockfile`. Installs are
bounded by a semaphore of 8. Required dependencies resolve recursively.

`mod_metadata.rs` fingerprints every mod by sha1 and parses zip internals into
a `ContentSummary`. Recognized metadata, in priority order: `mcmod.info`,
`fabric.mod.json`, `mods.toml`, NeoForge `mods.toml`, jarjar,
`MANIFEST.MF`, `pack.mcmeta`, `modrinth.index.json`, CurseForge
`manifest.json`.

Update checks batch against Modrinth and CurseForge with a semaphore of 8.

## Cross-instance syncing

`syncing.rs` shares files and folders across instances. File targets copy the
newest version. Folder targets replace the instance folder with a symlink
(unix) or junction (windows) into `synced/`. `options.txt` merges across all
instances.

## Authentication state

The backend runs a login state machine (`login`, backend.rs:581) with progress
and cancel support. Credentials go to the OS keyring through `auth`.

## Logging

The `log` crate everywhere. `main.rs` configures fern once:

- Colored stdout plus `launcher.log` (rotated to `.log.old`).
- `pandora_launcher`, `auth`, `backend`, `frontend`, `bridge`, `command` at
  Debug. Everything else at Warn.

The panic hook logs every panic and persists a report to `panic-reports/`.
A deadlock detector polls `parking_lot::deadlock::check_deadlock` every 10
seconds.

`log_reader.rs` redacts access tokens and paths from game logs.
