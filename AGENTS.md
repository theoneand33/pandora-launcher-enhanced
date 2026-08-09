# AGENTS.md — PandoraLauncher-Enhanced

## Vendored Sources

This repository vendors dependencies. It uses `cargo vendor` and `vendor.sh`.

- `Cargo.toml:75` declares git dependencies for `gpui` (`zed-industries/zed`) and `gpui-component`.
- `.cargo/config.toml:39` sets `[source.vendored-sources]` to `directory = "vendor"`. It sets `replace-with = "vendored-sources"` for crates-io and git sources.
- `vendor.sh` runs `cargo vendor`. It copies `gpui-component-assets` to `vendor/assets`. It trims vendored `gpui` crates (`vendor/gpui/Cargo.toml`, `vendor/gpui_linux/Cargo.toml`, `vendor/gpui_macos/Cargo.toml`, `vendor/gpui_windows/Cargo.toml`) to `image = { default-features = false, features = ["png","jpeg","bmp","gif","webp"] }`. It updates each `.cargo-checksum.json` for `Cargo.toml`.

Do not edit files under `vendor/` manually. Use `vendor.sh` or `[patch.crates-io]` with a fork.

## Troubleshooting: Vendored Checksum Mismatch

### Symptom

The local build or CI fails. It shows this error:

```
error: the listed checksum of `.../vendor/gpui/examples/image/image.rs` has changed:
expected: 70687cf7...
actual:   1f9213f0...
directory sources are not intended to be edited, if modifications are required then it is recommended that `[patch]` is used with a forked copy of the source
```

This pattern can also occur for `vendor/gpui/Cargo.toml`, `README.md`, `examples/*`, and `src/*`.

### Cause

`vendor/gpui/.cargo-checksum.json` does not match the files on disk.

In this repository, merge `fc1df3113` kept the stale checksum from `30ee44a28`. That checksum listed 128 files and `image.rs = 70687cf7`. The merge discarded checksums `274d193cc` and `9dc927ac7` from `Update dependencies`. Those checksums listed 149 files and `image.rs = 1f9213f0`. On disk, there were 149 files. The `.cargo-checksum.json` file listed 128 files and contained 67 incorrect hashes. `vendor.sh` also trims `Cargo.toml`. The file on disk had hash `4c4a462a`. The stale `.cargo-checksum.json` file had hash `4cbcb858`.

### Diagnose

Do these steps to diagnose the mismatch:

1. Check how many hashes mismatch in each vendored crate.
2. Compare the hash of `image.rs` on disk with the hash in `.cargo-checksum.json`.
3. Check the `gpui` revision in `Cargo.lock`.
4. Test the offline build.

```bash
python3 << 'PY'
import hashlib, pathlib, json
for name in ["gpui","gpui_linux","gpui_macos","gpui_windows"]:
    p = pathlib.Path(f"vendor/{name}/.cargo-checksum.json")
    d = json.loads(p.read_text())
    bad = [k for k,v in d["files"].items() if hashlib.sha256((pathlib.Path(f"vendor/{name}")/k).read_bytes()).hexdigest() != v]
    print(name, len(d["files"]), "mismatches:", len(bad), bad[:3])
PY
sha256sum vendor/gpui/examples/image/image.rs
python3 -c "import json; print(json.load(open('vendor/gpui/.cargo-checksum.json'))['files']['examples/image/image.rs'])"
grep -A2 'name = "gpui"' Cargo.lock | head -n 20
```

Verify the offline build:

```bash
cargo check --offline 2>&1 | head -n 50
```

If the checksum error is gone, cargo continues past vendored source loading. Other compile errors can still occur.

### Fix

Use one of these options to fix the mismatch.

#### Option A - Restore the correct `.cargo-checksum.json` file and update the `Cargo.toml` hash (fast, offline, keeps the `vendor.sh` trim)

This option is fast and works offline. It keeps the `vendor.sh` trim.

1. Restore `.cargo-checksum.json` from commit `274d193cc`.
2. Update the `Cargo.toml` hash to match the file on disk.

```bash
python3 << 'PY'
import json, subprocess, hashlib, pathlib
old = json.loads(subprocess.check_output(["git","show","274d193cc:vendor/gpui/.cargo-checksum.json"], text=True))
old["files"]["Cargo.toml"] = hashlib.sha256(pathlib.Path("vendor/gpui/Cargo.toml").read_bytes()).hexdigest()
pathlib.Path("vendor/gpui/.cargo-checksum.json").write_text(json.dumps(old, indent=2, sort_keys=True) + "\n")
print("wrote", len(old["files"]), "entries")
PY
```

#### Option B - Regenerate the vendor directory from scratch (requires network)

1. Run `./vendor.sh`.
2. If you run `cargo vendor` directly, re-apply the trim logic from `vendor.sh` for image features.

```bash
./vendor.sh
# or
cargo vendor
# then re-apply trim logic in vendor.sh for image features
```

#### Option C - Recompute all hashes from disk (use only if you edited `vendor/` intentionally)

1. Recompute all hashes from the files on disk.

```bash
python3 << 'PY'
import json, hashlib, pathlib
p = pathlib.Path("vendor/gpui/.cargo-checksum.json")
d = json.loads(p.read_text())
d["files"] = {k: hashlib.sha256((pathlib.Path("vendor/gpui")/k).read_bytes()).hexdigest() for k in d["files"]}
p.write_text(json.dumps(d, indent=2, sort_keys=True) + "\n")
PY
```

After you fix the checksum, commit `vendor/gpui/.cargo-checksum.json`. Do not commit a partial fix. A partial fix leaves 128 files in `.cargo-checksum.json` when the disk has 149 files.

### Prevention

Do these steps to prevent this error:

1. If a merge touches `Cargo.lock`, `vendor/`, or `.cargo/config.toml`, run `./vendor.sh` and commit the result.
2. If a conflict occurs in `vendor/*/.cargo-checksum.json`, take the side that matches the `gpui` revision in `Cargo.lock`. Then run `vendor.sh` again.
3. Keep `vendor/` and `.cargo-checksum.json` in sync. CI uses `vendored-sources`. A stale checksum fails the build. The build shows `directory sources are not intended to be edited`.
