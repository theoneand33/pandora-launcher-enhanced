# AGENTS.md — PandoraLauncher-Enhanced

## Vendored Sources

This repo vendors dependencies via `cargo vendor` and `vendor.sh`.

- `Cargo.toml:75` uses git deps for `gpui` (`zed-industries/zed`) and `gpui-component`.
- `.cargo/config.toml:39` sets `[source.vendored-sources] directory = "vendor"` and `replace-with = "vendored-sources"` for crates-io and git sources.
- `vendor.sh` runs `cargo vendor`, copies `gpui-component-assets` to `vendor/assets`, and trims vendored `gpui` crates (`vendor/gpui/Cargo.toml`, `vendor/gpui_linux/Cargo.toml`, `vendor/gpui_macos/Cargo.toml`, `vendor/gpui_windows/Cargo.toml`) to `image = { default-features = false, features = ["png","jpeg","bmp","gif","webp"] }` and updates each `.cargo-checksum.json` for `Cargo.toml`.

Do not edit files under `vendor/` manually. Use `vendor.sh` or `[patch.crates-io]` with a fork.

## Troubleshooting: Vendored Checksum Mismatch

### Symptom

CI / local build fails with:

```
error: the listed checksum of `.../vendor/gpui/examples/image/image.rs` has changed:
expected: 70687cf7...
actual:   1f9213f0...
directory sources are not intended to be edited, if modifications are required then it is recommended that `[patch]` is used with a forked copy of the source
```

Same pattern can occur for `vendor/gpui/Cargo.toml`, `README.md`, `examples/*`, `src/*`.

### Cause

`vendor/gpui/.cargo-checksum.json` does not match files on disk.

Known cause in this repo: `fc1df3113` merge kept the stale `30ee44a28` checksum (128 files, `image.rs = 70687cf7`) and discarded `274d193cc` / `9dc927ac7` (`Update dependencies`, 149 files, `image.rs = 1f9213f0`). Disk had 149 files, JSON listed 128 with 67 wrong hashes. `vendor.sh` also trims `Cargo.toml` (`4c4a462a` on disk vs `4cbcb858` in stale JSON).

### Diagnose

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

Verify offline build:

```bash
cargo check --offline 2>&1 | head -n 50
```

If the checksum error is gone, cargo proceeds past vendored source loading (other compile errors may remain).

### Fix

Option A — Restore correct JSON from the last good commit and re-apply the `Cargo.toml` trim hash (fast, offline, preserves `vendor.sh` trim):

```bash
python3 << 'PY'
import json, subprocess, hashlib, pathlib
old = json.loads(subprocess.check_output(["git","show","274d193cc:vendor/gpui/.cargo-checksum.json"], text=True))
old["files"]["Cargo.toml"] = hashlib.sha256(pathlib.Path("vendor/gpui/Cargo.toml").read_bytes()).hexdigest()
pathlib.Path("vendor/gpui/.cargo-checksum.json").write_text(json.dumps(old, indent=2, sort_keys=True) + "\n")
print("wrote", len(old["files"]), "entries")
PY
```

Option B — Regenerate vendor from scratch (requires network):

```bash
./vendor.sh
# or
cargo vendor
# then re-apply trim logic in vendor.sh for image features
```

Option C — Recompute all hashes from disk (if you edited `vendor/` intentionally):

```bash
python3 << 'PY'
import json, hashlib, pathlib
p = pathlib.Path("vendor/gpui/.cargo-checksum.json")
d = json.loads(p.read_text())
d["files"] = {k: hashlib.sha256((pathlib.Path("vendor/gpui")/k).read_bytes()).hexdigest() for k in d["files"]}
p.write_text(json.dumps(d, indent=2, sort_keys=True) + "\n")
PY
```

After fix, commit `vendor/gpui/.cargo-checksum.json`. Do not commit partial fixes that leave 128 vs 149 file count mismatch.

### Prevention

- After any merge that touches `Cargo.lock`, `vendor/`, or `.cargo/config.toml`, run `./vendor.sh` and commit the result.
- In conflicts on `vendor/*/.cargo-checksum.json`, take the side that matches `Cargo.lock` gpui rev (`git show <rev>:Cargo.lock | grep -A2 'name = "gpui"'`) and then re-run `vendor.sh`.
- CI uses `vendored-sources`; a stale checksum fails the build with `directory sources are not intended to be edited`. Keep `vendor/` and `.cargo-checksum.json` in sync.
