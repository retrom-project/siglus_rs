"""Build a self-contained, reproducible Siglus browser candidate; never game data."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import urllib.request
from retrom_jspi import enable_jspi
from retrom_licenses import write_notices

ROOT = Path(__file__).resolve().parents[1]
FONT_COMMIT = "a85815a42757630ce188fdad368c2dfc444d4773"
FONT_SHA = "a3041811a78c361b1de50f953c805e0244951c21c5bd412f7232ef0d899af0da"
FONT_LICENSE_SHA = "1c05c68c34f9708415aada51f17e1b0092d2cea709bf4a94cd38114f9e73d7d9"


def run(*args, **kwargs):
    return subprocess.run(args, cwd=ROOT, check=True, **kwargs)


def git(*args):
    if not (ROOT / ".git").exists():
        identity = json.loads((ROOT / ".retrom-source.json").read_text())
        return {("branch", "--show-current"): identity["branch"],
                ("rev-parse", "HEAD"): identity["commit"],
                ("status", "--porcelain"): ""}[args]
    return subprocess.check_output(["git", *args], cwd=ROOT).decode().strip()


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_digest():
    if (ROOT / ".git").exists():
        files = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=ROOT).decode().split("\0")
    else:
        files = json.loads((ROOT / ".retrom-source.json").read_text())["files"]
    rows = []
    for name in sorted(set(files) - {""}, key=lambda name: name.encode()):
        path = ROOT / name
        if not path.exists() and not path.is_symlink():
            continue
        content = os.readlink(path).encode() if path.is_symlink() else path.read_bytes()
        mode = "120000" if path.is_symlink() else "100755" if path.stat().st_mode & 0o111 else "100644"
        rows.append({"path": name, "mode": mode, "sha256": hashlib.sha256(content).hexdigest()})
    return hashlib.sha256(json.dumps(rows, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()


def font_inputs():
    cache = ROOT / "dist/retrom-inputs/font"
    cache.mkdir(parents=True, exist_ok=True)
    font = Path(os.environ.get("SIGLUS_DEFAULT_FONT_PATH", str(cache / "NotoSansSC.ttf"))).resolve()
    if not font.exists():
        urllib.request.urlretrieve(f"https://raw.githubusercontent.com/google/fonts/{FONT_COMMIT}/ofl/notosanssc/NotoSansSC%5Bwght%5D.ttf", font)
    if digest(font) != FONT_SHA:
        raise ValueError("SIGLUS_FONT_HASH_MISMATCH")
    license_path = cache / "OFL.txt"
    if not license_path.exists():
        urllib.request.urlretrieve(f"https://raw.githubusercontent.com/google/fonts/{FONT_COMMIT}/ofl/notosanssc/OFL.txt", license_path)
    if digest(license_path) != FONT_LICENSE_SHA:
        raise ValueError("SIGLUS_FONT_LICENSE_HASH_MISMATCH")
    return font, license_path


def main():
    output = Path(sys.argv[1])
    if not output.is_absolute() or output.is_symlink() or output.exists() and any(output.iterdir()):
        raise ValueError("SIGLUS_EMPTY_ABSOLUTE_OUTPUT_REQUIRED")
    output.mkdir(parents=True, exist_ok=True)
    font, font_license = font_inputs()
    bindgen = os.environ.get("WASM_BINDGEN", "wasm-bindgen")
    version = subprocess.check_output([bindgen, "--version"]).decode().strip()
    if version != "wasm-bindgen 0.2.111":
        raise ValueError("SIGLUS_WASM_BINDGEN_VERSION_MISMATCH")
    run("npm", "ci", "--prefix", "platform/retrom")
    run("npm", "test", "--prefix", "platform/retrom")
    source = source_digest()
    environment = dict(os.environ, SIGLUS_DEFAULT_FONT_PATH=str(font), RUSTFLAGS="-A static_mut_refs",
                       CARGO_PROFILE_RELEASE_LTO="false", CARGO_PROFILE_RELEASE_CODEGEN_UNITS="16")
    run("cargo", "build", "--locked", "--release", "--target", "wasm32-unknown-unknown",
        "-p", "siglus_scene_vm", "--lib", "-j", os.environ.get("SIGLUS_BUILD_JOBS", "6"), env=environment)
    with tempfile.TemporaryDirectory(prefix="retrom-stage-", dir=ROOT / "dist") as temporary:
        staging = Path(temporary)
        run(bindgen, str(ROOT / "target/wasm32-unknown-unknown/release/siglus_scene_vm.wasm"),
            "--target", "web", "--out-dir", str(staging / "retrom/pkg"))
        glue = staging / "retrom/pkg/siglus_scene_vm.js"
        glue.write_text(enable_jspi(glue.read_text()))
        for name in ("bridge.mjs", "project.mjs"):
            shutil.copyfile(ROOT / "platform/retrom" / name, staging / "retrom" / name)
        (staging / "wasm").mkdir()
        shutil.copyfile(ROOT / "platform/wasm/siglus_save_store.js", staging / "wasm/siglus_save_store.js")
        run(str(ROOT / "platform/retrom/node_modules/.bin/esbuild"), str(staging / "retrom/bridge.mjs"),
            "--bundle", "--format=esm", "--target=es2022", "--legal-comments=inline", "--minify",
            "--banner:js=/*! Siglus: MPL-2.0; codec and dependency licenses: THIRD-PARTY-NOTICES.md. Corresponding source: https://github.com/retrom-project/siglus_rs/releases */",
            f"--outfile={output / 'siglus.mjs'}")
        shutil.copyfile(staging / "retrom/pkg/siglus_scene_vm_bg.wasm", output / "siglus_bg.wasm")
    shutil.copyfile(ROOT / "LICENSE-MPL-2.0", output / "LICENSE-MPL-2.0")
    shutil.copyfile(font_license, output / "FONT-OFL.txt")
    write_notices(output / "THIRD-PARTY-NOTICES.md")
    for source_name, output_name in [("wgpu-hal-0.21.1/LICENSE.MIT", "WGPU-LICENSE-MIT"),
                                     ("wgpu-hal-0.21.1/LICENSE.APACHE", "WGPU-LICENSE-APACHE"),
                                     ("cpal-0.15.3/LICENSE", "CPAL-LICENSE")]:
        shutil.copyfile(ROOT / "vendor" / source_name, output / output_name)
    if source_digest() != source:
        raise ValueError("SIGLUS_SOURCE_CHANGED_DURING_BUILD")
    descriptor = {"schemaVersion": 1, "kind": "RETROM_CORE_CANDIDATE_V1", "coreId": "siglus",
                  "repository": "https://github.com/retrom-project/siglus_rs",
                  "branch": git("branch", "--show-current") or "HEAD", "commit": git("rev-parse", "HEAD"),
                  "dirty": bool(git("status", "--porcelain")), "sourceTreeSha256": source,
                  "adapterAbi": "siglus-host-v1", "files": [
                      {"filename": path.name, "sizeBytes": path.stat().st_size, "sha256": digest(path)}
                      for path in sorted(output.iterdir())]}
    (output / "retrom-core-candidate.json").write_text(json.dumps(descriptor, sort_keys=True, indent=2) + "\n")


if __name__ == "__main__":
    main()
