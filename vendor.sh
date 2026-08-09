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

cargo_toml = pathlib.Path("vendor/gpui/Cargo.toml")
if cargo_toml.exists():
    text = cargo_toml.read_text()
    # Replace [dependencies.image] block with trimmed features, preserving version
    def repl(m):
        header = m.group(1)
        body = m.group(0)
        ver = re.search(r'version\s*=\s*"[^"]+"', body)
        ver_line = ver.group(0) if ver else 'version = "0.25.1"'
        return f'{header}\n{ver_line}\ndefault-features = false\nfeatures = ["png", "jpeg", "bmp", "gif", "webp"]\n'

    pattern = r'(\[dependencies\.image\]).*?(?=\n\[)'
    new_text, n = re.subn(pattern, repl, text, flags=re.DOTALL)

    if new_text != text:
        cargo_toml.write_text(new_text)
        # Update checksum so cargo does not error on vendored Cargo.toml mismatch
        checksum_path = pathlib.Path("vendor/gpui/.cargo-checksum.json")
        if checksum_path.exists():
            data = json.loads(checksum_path.read_text())
            sha = hashlib.sha256(new_text.encode()).hexdigest()
            data["files"]["Cargo.toml"] = sha
            # cargo vendor pretty-prints with indent; use pretty to avoid noisy diff
            checksum_path.write_text(json.dumps(data, indent=2) + "\n")

PY
