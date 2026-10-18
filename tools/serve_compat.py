"""Serve a private copied game and pinned core on loopback for compatibility checks."""
import argparse
import http.server
import hashlib
import json
from pathlib import Path
from urllib.parse import unquote, urlsplit


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--game-root", required=True, type=Path)
    parser.add_argument("--config-root", required=True, type=Path)
    parser.add_argument("--wasm-root", required=True, type=Path)
    parser.add_argument("--port", type=int, default=8765)
    args = parser.parse_args()
    game = args.game_root.resolve(strict=True)
    config = args.config_root.resolve(strict=True)
    wasm = args.wasm_root.resolve(strict=True)
    frontend = Path(__file__).resolve().parents[1] / "platform" / "compat"
    index = [
        {"path": p.relative_to(game).as_posix(), "size": p.stat().st_size}
        for p in sorted(game.rglob("*")) if p.is_file()
    ]

    class Handler(http.server.SimpleHTTPRequestHandler):
        def do_GET(self):
            path = unquote(urlsplit(self.path).path)
            if path == "/project-id":
                data = hashlib.sha256(str(game).encode()).hexdigest().encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)
                return
            if path == "/project-index.json":
                data = json.dumps(index, ensure_ascii=False).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)
                return
            super().do_GET()

        def translate_path(self, path):
            path = unquote(urlsplit(path).path)
            if path == "/save-store.js":
                return str(frontend.parent / "wasm" / "siglus_save_store.js")
            for prefix, root in [("/file/", game), ("/configuration/", config), ("/pkg/", wasm / "pkg")]:
                if path.startswith(prefix):
                    target = (root / path[len(prefix):]).resolve()
                    return str(target) if target.is_relative_to(root) else str(frontend / "not-found")
            target = (frontend / (path.lstrip("/") or "index.html")).resolve()
            return str(target) if target.is_relative_to(frontend) else str(frontend / "not-found")

        def end_headers(self):
            self.send_header("Cross-Origin-Opener-Policy", "same-origin")
            self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
            self.send_header("Cache-Control", "no-store")
            super().end_headers()

    server = http.server.ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"Compatibility server: http://127.0.0.1:{args.port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
