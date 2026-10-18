"""Collect the locked browser dependency graph and its complete license texts."""
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
EXTRA = ROOT / ".github/rpg-runtime/licenses"


def dependency_graph():
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--format-version=1",
        "--filter-platform", "wasm32-unknown-unknown"], cwd=ROOT))
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    pending = [p["id"] for p in metadata["packages"] if p["name"] == "siglus_scene_vm"]
    reached = set()
    while pending:
        identity = pending.pop()
        if identity in reached:
            continue
        reached.add(identity)
        pending.extend(dep["pkg"] for dep in nodes[identity]["deps"]
                       if any(kind["kind"] != "dev" for kind in dep["dep_kinds"]))
    return sorted((p for p in metadata["packages"] if p["id"] in reached),
                  key=lambda p: (p["name"], p["version"]))


def write_notices(output):
    sections = [(ROOT / "RETROM-NOTICES.md").read_text()]
    texts = {}
    sources = json.loads((EXTRA / "sources.json").read_text())
    for item in sources:
        data = (EXTRA / item["filename"]).read_bytes()
        if hashlib.sha256(data).hexdigest() != item["sha256"]:
            raise ValueError("SIGLUS_LICENSE_INPUT_HASH_MISMATCH: " + item["filename"])
        texts["external/" + item["filename"]] = data.decode()
    sections.append("## Locked browser dependencies (normal and build graph)\n\n"
                    "Alternative licenses retain their original expression. The additional codec "
                    "and font notices below apply independently. Repeated license texts are stored once.\n")
    for package in dependency_graph():
        folder = Path(package["manifest_path"]).parent
        label = package["name"] + " " + package["version"]
        license_name = package["license"] or "MPL-2.0; see codec-specific notices"
        sections.append(f"### {label}\n\nLicense: {license_name}\n\n"
                        f"Source: {package['repository'] or 'this corresponding-source archive'}\n")
        licenses = [p for p in folder.rglob("*") if p.is_file() and
                    (p.name.upper().startswith(("LICENSE", "COPYING", "COPYRIGHT", "NOTICE", "UNLICENSE"))
                     or p.parent.name == "fonts" and p.suffix == ".txt") and
                    not any(part in (".git", "node_modules", "target") for part in p.parts)]
        if not licenses:
            if package["source"] is None or license_name == "MPL-2.0":
                licenses = [ROOT / "LICENSE-MPL-2.0"]
            elif not any(item["filename"].startswith(package["name"] + "-") for item in sources):
                raise ValueError("SIGLUS_LICENSE_TEXT_MISSING: " + label)
        for license_path in sorted(licenses):
            data = license_path.read_text()
            key = next((key for key, value in texts.items() if value == data), None)
            if key is None:
                key = label + "/" + license_path.relative_to(folder).as_posix() if license_path.is_relative_to(folder) else "MPL-2.0"
                texts[key] = data
            sections.append("License text: " + key + "\n")
    for name, data in sorted(texts.items()):
        sections.append(f"## License text: {name}\n\n```text\n{data.rstrip()}\n```\n")
    output.write_text("\n".join(sections))


if __name__ == "__main__":
    import sys
    write_notices(Path(sys.argv[1]))
