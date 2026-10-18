# Retrom browser maintenance

`main` mirrors upstream. Browser changes are maintained on
`retrom/g5f358ef1360b`, based on upstream commit
`5f358ef1360b0439e92aed2b749e8f25d8721954`. Work branches use `feat/`, `fix/`
or `build/` and merge into that maintenance branch, never into the mirror.
`retrom-fork.json` defines the ABI and immutable release names. Native upstream
workflows are retained as reference and disabled in the Retrom fork; the
`Retrom browser` workflow tests and releases the supported browser integration.

The host supplies directory metadata and bounded asynchronous reads through
public Content I/O. The core does not download or cache game directories.
JSPI is required. All mutable engine entries are promising roots and serialize
through one bridge queue, including voice decoding triggered by pointer input.
The save overlay exports real native files before checkpoint acknowledgement.
Movie decoders keep bounded queues; selected compressed resources may still be
assembled in memory by native resource decoders.

## Build and test

Use Rust 1.91.0 with target wasm32-unknown-unknown, Node 24.18.0, Python 3.11+
and wasm-bindgen-cli 0.2.111. Linux native tests also need ALSA and udev headers.
`platform/retrom/package-lock.json` fixes esbuild and its transitive inputs.

```sh
rustup target add wasm32-unknown-unknown --toolchain 1.91.0
cargo install wasm-bindgen-cli --version 0.2.111 --locked
python3 -c 'import sys; sys.path.insert(0,"tools"); from build_retrom_candidate import font_inputs; font_inputs()'
RUST_MIN_STACK=16777216 RUSTFLAGS='-A static_mut_refs' cargo test --locked -p siglus_scene_vm --lib -j6
npm ci --prefix platform/retrom
npm test --prefix platform/retrom
python3 tools/build_retrom_candidate.py /absolute/empty/output
```

The font is the pinned OFL Noto Sans SC input. Both its bytes and license are
verified before use; the default path is `dist/retrom-inputs/font/NotoSansSC.ttf`.
No proprietary font or game input is needed for a build or the synthetic tests.
`compatibility-probe` is test-only and is excluded from production Wasm.

## Release and corresponding source

After a reviewed PR passes the browser workflow, squash it into the maintenance
branch. Create an annotated `retrom-core-g5f358ef1360b-rN` tag on that branch.
The tag workflow publishes five runtime/license files, the complete corresponding
source archive and `rpg-runtime-release.json`. Never move a tag or replace its
assets. Verify all manifest sizes/hashes before pinning the release in runtime.

```sh
python3 tools/build_retrom_release.py /absolute/empty/output retrom-core-g5f358ef1360b-r1 --check
python3 tools/build_retrom_release.py /absolute/output --verify
```

`--check` exercises packaging on a clean PR commit without publishing. Omitting
it additionally requires an annotated tag at HEAD on the maintenance branch.
The source archive contains the exact repository sources, Cargo.lock, all Cargo
registry sources and replacement configuration, patched dependencies, default
font/license, and `.retrom-source.json` identifying the original commit and tree.
It excludes the upstream proprietary font, games, keys, build outputs and Git
history. Its deterministic gzip/tar headers allow packaging to be compared.

To modify and relink LGPL decoder components, extract `siglus-source.tar.gz`,
edit the desired Rust files and run `tools/build_retrom_candidate.py` as above.
Cargo resolves the included sources through `.cargo/config.toml`; use
`CARGO_NET_OFFLINE=true` to enforce offline Cargo resolution. The Node build
tool dependencies can be installed from the included lockfile. The script
recompiles/relinks the complete Wasm, regenerates JSPI glue and bundles JavaScript.
Replace `siglus.mjs` and `siglus_bg.wasm` together in an embedding, or pin a new
immutable fork release in `retrom-runtime/provider-sources.json` and rebuild its
Provider. This supports modification and debugging with a rebuilt decoder;
Retrom imposes no additional restriction on that use.
