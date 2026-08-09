#!/usr/bin/env bash
set -e
cargo vendor
cp -r ./vendor/gpui-component-assets ./vendor/assets

# Patch vendored gpui to keep image trimmed (cargo vendor overwrites vendor/gpui/Cargo.toml).
# Only png/jpeg/bmp/gif/webp are hot paths (skin/icon loads are ImageFormat::Png). Default pulls
# avif/rav1e (~90 crates), exr, tiff. Trim saves ~30-40% image compile time.
# See Cargo.toml workspace image comment and review feedback vendor/gpui fix.
python3 << 'PY'
import json, hashlib, pathlib, re

# Patch all vendored gpui crates that depend on image: gpui, gpui_linux, gpui_macos, gpui_windows.
# cargo vendor overwrites vendor/gpui*/Cargo.toml, so re-apply trimmed features after vendor.
# Only png/jpeg/bmp/gif/webp are hot paths (skin/icon loads are ImageFormat::Png). Default pulls
# avif/rav1e (~90 crates), exr, tiff, etc. Trim saves ~30-40% image compile time.
# See Cargo.toml workspace image comment.

trimmed_features = 'features = ["png", "jpeg", "bmp", "gif", "webp"]'
targets = [
    "vendor/gpui/Cargo.toml",
    "vendor/gpui_linux/Cargo.toml",
    "vendor/gpui_macos/Cargo.toml",
    "vendor/gpui_windows/Cargo.toml",
]

# Match both [dependencies.image] and [target.'cfg(...)'.dependencies.image] (non-greedy inside brackets)
pattern = r'(\[[^\]]*dependencies\.image[^\]]*\]).*?(?=\n\[|\Z)'

def repl(m):
    header = m.group(1)
    body = m.group(0)
    # Idempotent: already trimmed to exactly the 5 cheap features
    if 'default-features = false' in body and '"png"' in body and '"exr"' not in body and '"tiff"' not in body:
        return m.group(0)
    ver = re.search(r'version\s*=\s*"[^"]+"', body)
    ver_line = ver.group(0) if ver else 'version = "0.25.1"'
    return f'{header}\n{ver_line}\ndefault-features = false\n{trimmed_features}\n'

for cargo_path_str in targets:
    cargo_toml = pathlib.Path(cargo_path_str)
    if not cargo_toml.exists():
        continue
    text = cargo_toml.read_text()
    new_text, n = re.subn(pattern, repl, text, flags=re.DOTALL)
    if new_text != text:
        cargo_toml.write_text(new_text)
        # Update checksum so cargo does not error on vendored Cargo.toml mismatch
        checksum_name = cargo_toml.parent.name  # gpui, gpui_linux, etc.
        checksum_path = cargo_toml.parent / ".cargo-checksum.json"
        if checksum_path.exists():
            data = json.loads(checksum_path.read_text())
            sha = hashlib.sha256(new_text.encode()).hexdigest()
            if data["files"].get("Cargo.toml") != sha:
                data["files"]["Cargo.toml"] = sha
                checksum_path.write_text(json.dumps(data, indent=2) + "\n")
                print(f"patched {cargo_path_str} ({n} table(s))")

PY
