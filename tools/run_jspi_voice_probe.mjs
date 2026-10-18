// Build siglus_scene_vm with compatibility-probe and apply retrom_jspi.py first.
// Uses generated PCM only; no game files or Retrom server are needed.
import assert from "node:assert/strict";
import {readFile, writeFile} from "node:fs/promises";
import {createServer} from "node:http";
import {resolve} from "node:path";
import {pathToFileURL} from "node:url";

const [pkgRoot, output] = process.argv.slice(2);
assert.ok(pkgRoot && output, "usage: run_jspi_voice_probe.mjs <generated-pkg> <result.json>");
const resources = new Map(await Promise.all([
  ["/siglus_scene_vm.js", "text/javascript"],
  ["/siglus_scene_vm_bg.wasm", "application/wasm"],
].map(async ([path, type]) => [path, {type, bytes: await readFile(resolve(pkgRoot, path.slice(1)))}])));
const server = createServer((request, response) => {
  if (request.url === "/") return response.end("<!doctype html><title>JSPI voice regression</title>");
  const resource = resources.get(request.url);
  if (!resource) {response.writeHead(404); return response.end();}
  response.writeHead(200, {"Content-Type": resource.type}); response.end(resource.bytes);
});
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
const {chromium} = await import(pathToFileURL(process.env.RETROM_PLAYWRIGHT_MODULE).href);
let browser;
const errors = [];
try {
  browser = await chromium.launch({executablePath: process.env.RETROM_CHROME_EXECUTABLE, headless: true, args: ["--no-sandbox"]});
  const page = await browser.newPage();
  page.on("pageerror", error => errors.push({message: error.message, stack: error.stack}));
  await page.goto(`http://127.0.0.1:${server.address().port}`);
  const result = await page.evaluate(async () => {
    // Large enough to require three bounded VFS reads, with real PCM to decode.
    const pcmBytes = 600000, wav = new Uint8Array(44 + pcmBytes), header = new DataView(wav.buffer);
    const text = (offset, value) => wav.set(new TextEncoder().encode(value), offset);
    text(0, "RIFF"); header.setUint32(4, wav.length - 8, true); text(8, "WAVE"); text(12, "fmt ");
    header.setUint32(16, 16, true); header.setUint16(20, 1, true); header.setUint16(22, 1, true);
    header.setUint32(24, 16000, true); header.setUint32(28, 32000, true);
    header.setUint16(32, 2, true); header.setUint16(34, 16, true);
    text(36, "data"); header.setUint32(40, pcmBytes, true);
    for (let i = 0; i < pcmBytes / 2; i++) header.setInt16(44 + i * 2, Math.sin(i * Math.PI / 20) * 2000, true);
    const voice = path => path.replace(/^\.\//, "") === "koe/0000/z000000001.wav";
    const reads = [];
    let rejectRead = false;
    globalThis.siglusFileExists = voice;
    globalThis.siglusFileSize = path => voice(path) ? wav.length : -1;
    globalThis.siglusReadFileRange = async (path, offset, length) => {
      reads.push({path, offset, length}); await new Promise(resolve => setTimeout(resolve, 5));
      if (rejectRead) throw Error("controlled voice Content I/O failure");
      assertVoice(path); return wav.slice(offset, offset + length);
    };
    function assertVoice(path) {if (!voice(path)) throw Error("unexpected fixture read: " + path);}
    globalThis.siglusListDir = () => [];
    let exports;
    for (const name of ["instantiate", "instantiateStreaming"]) {
      const original = WebAssembly[name];
      WebAssembly[name] = async (...args) => {const result = await original(...args); exports = (result.instance ?? result).exports; return result;};
    }
    const wrapped = new Set(), promising = WebAssembly.promising;
    WebAssembly.promising = fn => {wrapped.add(fn); return promising(fn);};
    const module = await import("/siglus_scene_vm.js");
    await module.default({module_or_path: "/siglus_scene_vm_bg.wasm"});
    let ticks = 0;
    const timer = setInterval(() => ticks++, 1);
    const roots = Object.fromEntries(["create", "step", "persist", "pointer", "key", "wheel", "text", "volume", "paused", "stop"]
      .map(name => [name, wrapped.has(exports["retromgame_" + name])]));
    const probe = WebAssembly.promising(exports.compatibility_koe_decode_probe);
    const started = await probe();
    // Old code reads in a later spawn_local callback, outside the promise root.
    await new Promise(resolve => setTimeout(resolve, 80));
    let rejectedVoice, afterFailure;
    if (started) {
      rejectRead = true; rejectedVoice = await probe();
      rejectRead = false; afterFailure = await probe();
    }
    clearInterval(timer);
    return {started: Boolean(started), rejectedVoice, afterFailure: Boolean(afterFailure), reads, ticks, roots};
  });
  const passed = result.started && result.rejectedVoice === 0 && result.afterFailure &&
    errors.length === 0 && result.ticks > 0 && Object.values(result.roots).every(Boolean) &&
    result.reads.length === 7 && result.reads.every(read => read.length <= 262144);
  const proof = {status: passed ? "PASS" : "FAIL", ...result, errors};
  await writeFile(resolve(output), JSON.stringify(proof, null, 2) + "\n");
  console.log(JSON.stringify(proof));
  assert.ok(passed, "dialogue voice decode must stay inside the promising engine entry");
} finally {
  await browser?.close();
  await new Promise(resolve => server.close(resolve));
}
