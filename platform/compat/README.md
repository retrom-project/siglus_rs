# Standalone Siglus compatibility experiment

This directory and the related `tools/*compat*` / `tools/run_localization.mjs`
scripts are diagnostic tools. They are not a Retrom Provider, a supported game
launcher, or evidence of whole-game acceptance. No private game, recovered key,
decoded script, screenshot, or generated WASM belongs in source control.

The explicit `?language=chs` profile requires `scene.chs` and `gameexe.chs`.
It maps requests for `Scene.pck` / `Gameexe.dat` to that pair, and `.g00` to an
existing same-stem `.g01`. It does not run Windows executables or DLLs. The
original profile leaves resource names unchanged. The host otherwise preserves
the upstream whole-file synchronous VFS behavior; it does not prove bounded I/O.

Use full private **copies** of game directories, never the original source
directories. Recover and inspect both resource variants with:

```sh
cargo run --locked -p siglus_key_recovery --example inspect_project -- \
  "$COPIED_GAME_ROOT" "$PRIVATE_DECODED_ROOT"
```

Build the actual browser core with a separately sourced, licensed CJK font:

```sh
SIGLUS_DEFAULT_FONT_PATH="$PFB_FONT_FILE" \
RUSTFLAGS='-A static_mut_refs' \
CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
cargo build --locked --release --target wasm32-unknown-unknown \
  -p game_launcher --lib --features compatibility-probe
wasm-bindgen --target web --out-dir dist/compat-readonly/pkg \
  target/wasm32-unknown-unknown/release/game_launcher.wasm
python3 tools/serve_compat.py --game-root "$COPIED_GAME_ROOT" \
  --config-root "$PRIVATE_DECODED_ROOT/chs" \
  --wasm-root dist/compat-readonly --port 8767
```

The upstream build uses the same `static_mut_refs` flag. Pin the toolchain,
wasm-bindgen version, font source/license, upstream commit, local diff and output
hash in the experiment record. The font override is optional; without it the
upstream embedded font remains, so this patch alone does not settle distribution.

The optional `compatibility-probe` feature publishes **read-only** VM scene,
line, visible message text and unknown-command notes. Never call
`debug_status_summary` or `is_blocked` from this observer: checking waits with
those methods can consume them and change game execution.

With Playwright and Chromium paths supplied through `RETROM_PLAYWRIGHT_MODULE`
and `RETROM_CHROME_EXECUTABLE`, run:

```sh
node tools/run_localization.mjs \
  'http://127.0.0.1:8767/?language=chs' "$PRIVATE_EVIDENCE_ROOT" \
  "$START_PAGE_X" "$START_PAGE_Y"
```

The script has a ten-minute opening budget and holds input through slow
software-rendered frames. It checks actual translated resource reads, dialogue
and script-location advancement, browser exceptions and WebGL errors. Inspect
the screenshots too. `LIMITED_DIALOGUE_PASS` only means that short probe passed;
warnings, save/restore, later scenes and other versions remain separate gates.

The first 2026-10-17 experiment stopped at the localization gate after a long
OMV trapped. The follow-up replaced Web OMV playback/preview with incremental
decoding and a bounded current/next-frame cache. Both copied games now display
and advance Chinese dialogue. `run_movie_probe.mjs` tests the actual MovieManager
through feature-gated diagnostic exports, including preview, all playback times,
loop restart, pixel changes and resource release.

The experiment uses the explicitly supplied OFL Noto Sans SC font (regular
variable weight), covering punctuation as well as Chinese characters. Original
inputs remain unchanged. The follow-up adds writable save storage and replaces
all three browser movie paths (OMV, ASF/WMV and MPEG) with bounded decoders.
`MovieAsset` full-film APIs fail explicitly on Web. Native diagnostic assets
continue using their existing preview behavior. This still adds no Retrom
Provider declaration or product acceptance.

## Save host contract

`platform/wasm/siglus_save_store.js` is shared by this experiment and the normal
WASM launcher. Initialize it **before** starting the engine:

```js
const saves = await SiglusSaveStore.open(stableGameIdentity, {
  exists: readOnlyFileExists,
  read: readOnlyFileBytes,
  list: readOnlyDirectoryChildren,
}, reportStorageError);
if (archive) await saves.import(archive);
saves.bind();
// Start the engine only after persisted files / import are available.
// Before checkpoint publication or explicit exit:
await saves.flush();
const snapshot = await saves.export();
```

The identity must be stable across launches and content updates, and distinct
for incompatible original / translated profiles. A Retrom adapter should supply
its content identity, not a launch ID. The standalone server uses the canonical
copied directory's hash plus the selected language; the regular launcher uses
its library root and language. Moving an installation requires retaining that
identity. Import rejects a different identity, invalid paths and duplicate
entries before replacing any data.

Rust keeps the native save format and routes local/global/read/config writes,
header updates, thumbnails, captures, copy, remove and rename through the shared
resource filesystem seam. The JS host provides `siglusWriteFile`,
`siglusCreateDir`, `siglusRemoveFile`, and `siglusRenameFile` as well as the
existing read/list/exists functions. Engine writes copy WASM bytes immediately
and update the synchronous view; IndexedDB transactions commit in order.
Rename updates its two paths in one transaction. Exports include original
read-only `savedata` files and the entire writable overlay, including persisted
deletions. None of these operations modify selected original game files.

A synchronous engine SAVE is not an asynchronous durability barrier. Hosts must
await `flush()` / `export()` before reporting durable success. A transaction
failure calls the error handler and rejects flush, later writes and export.
The normal launcher's explicit exit also waits for flush. Imports must finish
before the VM starts; changing its files while a VM is live is not supported.

These are original game saves: they resume the script's last SAVEPOINT, which
can be at the start of the current text block. They are not instant VM states.

```sh
node tools/run_save_store_probe.mjs "$URL" "$PRIVATE_OUTPUT"
node tools/run_save_restore.mjs "$URL" "$PRIVATE_OUTPUT" hatsukoi
node tools/run_save_restore.mjs "$URL" "$PRIVATE_OUTPUT" mashiro
```

The two UI profiles contain test coordinates only. The core contains no game
name checks. Original first-use help and confirmation dialogs are driven by
real mouse input. An optional fourth argument resumes only the restore phase
from an existing case directory. To test overwrite plus a full export/reload/
new-instance cycle starting with a previously created **native** save, pass `-`
as the fourth argument and the save archive path as the fifth. Use a case
directory archive with its companion `01-checkpoint.json`. No game script
or global flag is patched to skip an opening.

## Movie regression scope

`run_movie_probe.mjs` uses the production MovieManager through a diagnostic
export. It asserts preview size, frame/YUV queue bounds, the rejected full-film
API, all playback times, changing pixels, loop restart, idle eviction, three
reopens with stable memory, and stop release. It also handles duration discovery
at EOF, including headerless-duration ASF and duplicate Theora packets.

Compressed movies still use whole-file VFS reads. Seekable readers share that
allocation instead of copying it into an Arc slice. The presentation buffers
are independent of film length: OMV retains at most two RGBA pictures; WMV/MPEG
retain the current RGBA picture and bounded YUV reordering pictures (up to eight
for WMV B frames), in addition to codec reference buffers. Previews use one
frame and at most eight cached entries. Idle decoders are removed after 120
unvisited engine frames; stop clears their buffers and preview cache.

The Web audio path preserves embedded audio with a 64 MiB static PCM limit per
movie, explicitly failing on overflow. It is not streaming audio. Range I/O,
all title/codec variants, full gameplay, Retrom input/checkpoint/content cache
contracts and formal licensed release assets remain separate acceptance work.

## JSPI dialogue voice regression

`tools/run_jspi_voice_probe.mjs` exercises the real `KoeEngine` with a generated
PCM WAV, without game data or a Retrom server. Build `siglus_scene_vm` with
`--features compatibility-probe` using the same release settings as the core
candidate, run the pinned wasm-bindgen 0.2.111 with `--target web`, and apply
`tools/retrom_jspi.py`'s `enable_jspi` to the generated JS. Then run:

```sh
node tools/run_jspi_voice_probe.mjs "$GENERATED_PKG_ROOT" "$PRIVATE_RESULT_JSON"
```

Supply `RETROM_PLAYWRIGHT_MODULE` and `RETROM_CHROME_EXECUTABLE` as for the other
browser probes. The script serves only the generated module and Wasm on a local
ephemeral port and closes its server and browser. It checks all mutable host
promise roots, three bounded reads per successful voice, event-loop progress,
an asynchronous read rejection, and successful playback setup after that
failure. A voice read dispatched with `spawn_local` reproduces `SuspendError`
in a later wasm-bindgen callback despite the original host entry being wrapped.
Browser voice reads therefore stay on the serialized JSPI engine stack; native
voice decoding retains its worker thread. CPU decoding on Web still runs on
the browser thread, as it did with `spawn_local`.
