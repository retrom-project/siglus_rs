"""Build and verify immutable browser assets and their complete rebuildable source."""
import gzip
import io
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tempfile
from build_retrom_candidate import ROOT, digest, font_inputs, git, main as build_candidate, source_digest

ASSETS = ("siglus.mjs", "siglus_bg.wasm", "LICENSE-MPL-2.0", "FONT-OFL.txt", "THIRD-PARTY-NOTICES.md")
REPOSITORY = "https://github.com/retrom-project/siglus_rs"
BASELINE = "5f358ef1360b0439e92aed2b749e8f25d8721954"


def source_archive(output, tag):
    vendor = ROOT / "dist/retrom-vendor"
    config = subprocess.check_output(["cargo", "vendor", "--locked", "--versioned-dirs", str(vendor)], cwd=ROOT)
    config = config.decode().replace(str(vendor), "dist/retrom-vendor").encode()
    files = git("ls-files", "-z").split("\0")
    files = sorted(name for name in files if name)
    if any(name.endswith("assets/fonts/default.ttf") for name in files):
        raise ValueError("SIGLUS_UNLICENSED_FONT_IN_SOURCE")
    font, font_license = font_inputs()
    identity = json.dumps({"tag": tag, "commit": git("rev-parse", "HEAD"), "branch": git("branch", "--show-current") or "HEAD",
                           "sourceTreeSha256": source_digest(), "files": files}, sort_keys=True, indent=2).encode() + b"\n"
    entries = {name: ROOT / name for name in files}
    entries.update({p.relative_to(ROOT).as_posix(): p for p in vendor.rglob("*") if p.is_file()})
    entries.update({"dist/retrom-inputs/font/NotoSansSC.ttf": font,
                    "dist/retrom-inputs/font/OFL.txt": font_license,
                    ".cargo/config.toml": config, ".retrom-source.json": identity})
    with output.open("wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w|", format=tarfile.PAX_FORMAT) as archive:
            for name, value in sorted(entries.items()):
                data = value if isinstance(value, bytes) else value.read_bytes()
                entry = tarfile.TarInfo("siglus-source/" + name)
                entry.size = len(data)
                entry.mode = 0o755 if isinstance(value, Path) and value.stat().st_mode & 0o111 else 0o644
                archive.addfile(entry, io.BytesIO(data))


def verify(output):
    metadata = json.loads((output / "rpg-runtime-release.json").read_text())
    expected = set(ASSETS) | {"siglus-source.tar.gz", "rpg-runtime-release.json"}
    if set(p.name for p in output.iterdir()) != expected:
        raise ValueError("SIGLUS_RELEASE_FILE_SET_INVALID")
    if (metadata["schemaVersion"] != 1 or metadata["repository"] != REPOSITORY or
            not re.fullmatch(r"retrom-core-g5f358ef1360b-r[1-9][0-9]*(-rc\.[1-9][0-9]*)?", metadata["tag"]) or
            metadata["adapterAbi"] != "siglus-host-v1" or metadata["sourceCommits"] != {"engine": BASELINE}):
        raise ValueError("SIGLUS_RELEASE_IDENTITY_INVALID")
    rows = metadata["files"] + [metadata["sourceArchive"]]
    if len(rows) != 6 or {r["filename"] for r in rows} != expected - {"rpg-runtime-release.json"}:
        raise ValueError("SIGLUS_RELEASE_MANIFEST_INVALID")
    for row in rows:
        path = output / row["filename"]
        if path.stat().st_size != row["sizeBytes"] or digest(path) != row["sha256"]:
            raise ValueError("SIGLUS_RELEASE_HASH_MISMATCH: " + row["filename"])
    with tarfile.open(output / "siglus-source.tar.gz") as archive:
        names = archive.getnames()
        if len(names) != len(set(names)) or any(".." in Path(name).parts or name.startswith("/") or
                not name.startswith("siglus-source/") or "assets/fonts/default.ttf" in name for name in names):
            raise ValueError("SIGLUS_SOURCE_ARCHIVE_INVALID")
        identity = json.load(archive.extractfile("siglus-source/.retrom-source.json"))
        if identity["commit"] != metadata["commit"] or identity["sourceTreeSha256"] != metadata["sourceTreeSha256"]:
            raise ValueError("SIGLUS_SOURCE_IDENTITY_MISMATCH")
        for required in ("Cargo.lock", ".cargo/config.toml", "dist/retrom-inputs/font/NotoSansSC.ttf", "RETROM.md"):
            if "siglus-source/" + required not in names:
                raise ValueError("SIGLUS_REBUILD_INPUT_MISSING: " + required)
    print(json.dumps({"verified": True, "tag": metadata["tag"], "commit": metadata["commit"],
                      "sourceArchiveSha256": metadata["sourceArchive"]["sha256"]}))


def main():
    output = Path(sys.argv[1]).resolve()
    if sys.argv[2:] == ["--verify"]:
        verify(output)
        return
    tag = sys.argv[2]
    if git("status", "--porcelain"):
        raise ValueError("SIGLUS_CLEAN_SOURCE_REQUIRED")
    if not re.fullmatch(r"retrom-core-g5f358ef1360b-r[1-9][0-9]*(-rc\.[1-9][0-9]*)?", tag):
        raise ValueError("SIGLUS_RELEASE_TAG_INVALID")
    subprocess.run(["git", "merge-base", "--is-ancestor", BASELINE, "HEAD"], cwd=ROOT, check=True)
    if "--check" not in sys.argv:
        if git("cat-file", "-t", "refs/tags/" + tag) != "tag" or git("rev-list", "-n", "1", tag) != git("rev-parse", "HEAD"):
            raise ValueError("SIGLUS_ANNOTATED_TAG_REQUIRED")
        subprocess.run(["git", "merge-base", "--is-ancestor", "HEAD", "retrom/g5f358ef1360b"], cwd=ROOT, check=True)
    sys.argv = [sys.argv[0], str(output)]
    build_candidate()
    candidate = json.loads((output / "retrom-core-candidate.json").read_text())
    for name in ("retrom-core-candidate.json", "WGPU-LICENSE-MIT", "WGPU-LICENSE-APACHE", "CPAL-LICENSE"):
        (output / name).unlink()
    source_archive(output / "siglus-source.tar.gz", tag)
    row = lambda name: {"filename": name, "sizeBytes": (output / name).stat().st_size, "sha256": digest(output / name)}
    metadata = {"schemaVersion": 1, "repository": REPOSITORY, "tag": tag, "commit": candidate["commit"],
                "adapterAbi": candidate["adapterAbi"], "sourceCommits": {"engine": BASELINE},
                "sourceTreeSha256": candidate["sourceTreeSha256"], "files": [row(name) for name in ASSETS],
                "sourceArchive": row("siglus-source.tar.gz"),
                "build": {"rust": "1.91.0", "wasmBindgen": "0.2.111", "node": "24.18.0", "esbuild": "0.27.0"}}
    (output / "rpg-runtime-release.json").write_text(json.dumps(metadata, sort_keys=True, indent=2) + "\n")
    verify(output)


if __name__ == "__main__":
    main()
