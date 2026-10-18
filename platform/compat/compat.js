import {SiglusSaveStore, saveScope} from "/save-store.js";
// Standalone core experiment. This is not a Retrom Provider or product route.
const query = new URLSearchParams(location.search);
const translated = query.get("language") === "chs";
const state = globalThis.compatibility = {translated, reads: [], writes: [], error: null, started: false};
let latestProbe;
Object.defineProperty(globalThis, "__siglusCompatProbe", {
  configurable: true,
  get: () => latestProbe,
  set: value => {
    state.probeFrames = (state.probeFrames || 0) + 1;
    state.probeTime = performance.now();
    latestProbe = value;
  },
});
if (query.has("tracegl")) {
  state.graphics = [];
  const attach = WebGL2RenderingContext.prototype.framebufferTexture2D;
  WebGL2RenderingContext.prototype.framebufferTexture2D = function (...args) {
    const maximum = this.getParameter(this.MAX_COLOR_ATTACHMENTS);
    if (args[1] >= this.COLOR_ATTACHMENT0 && args[1] < this.COLOR_ATTACHMENT0 + 32 &&
        args[1] >= this.COLOR_ATTACHMENT0 + maximum && state.graphics.length < 8) {
      state.graphics.push({attachment: args[1], maximum, textureIsNull: args[3] === null, stack: new Error().stack});
    }
    return attach.apply(this, args);
  };
}
const files = await (await fetch("/project-index.json")).json();
const byName = new Map(files.map(file => [file.path.toLowerCase(), file]));
if (translated && (!byName.has("scene.chs") || !byName.has("gameexe.chs"))) {
  throw new Error("The explicit CHS profile requires both scene.chs and gameexe.chs");
}
const normalize = path => path.replaceAll("\\", "/").split("/").filter(p => p && p !== ".").join("/");
function resolve(path) {
  let name = normalize(path).toLowerCase();
  if (name === "key.toml") return {path: "key.toml", url: "/configuration/key.toml"};
  if (translated) {
    if (name === "scene.pck" && byName.has("scene.chs")) name = "scene.chs";
    if (name === "gameexe.dat" && byName.has("gameexe.chs")) name = "gameexe.chs";
    if (name.endsWith(".g00") && byName.has(name.slice(0, -4) + ".g01")) name = name.slice(0, -4) + ".g01";
  }
  const file = byName.get(name);
  return file ? {...file, url: "/file/" + file.path.split("/").map(encodeURIComponent).join("/")} : null;
}
globalThis.siglusFileExists = path => Boolean(resolve(path));
globalThis.siglusReadFile = path => {
  const file = resolve(path);
  if (!file) throw new Error("Missing project file: " + path);
  const xhr = new XMLHttpRequest();
  xhr.open("GET", file.url, false);
  xhr.overrideMimeType("text/plain; charset=x-user-defined");
  xhr.send();
  if (xhr.status !== 200) throw new Error("Project read failed: " + path + ", " + xhr.status);
  const text = xhr.responseText;
  const bytes = new Uint8Array(text.length);
  for (let i = 0; i < text.length; i++) bytes[i] = text.charCodeAt(i) & 255;
  state.reads.push({requested: path, resolved: file.path, bytes: bytes.length});
  return bytes;
};
globalThis.siglusListDir = path => {
  const prefix = normalize(path).toLowerCase();
  const children = new Set();
  for (const file of files) {
    const normalized = normalize(file.path);
    if (!prefix) children.add(normalized.split("/")[0]);
    else if (normalized.toLowerCase().startsWith(prefix + "/")) children.add(normalized.slice(prefix.length + 1).split("/")[0]);
  }
  if (!prefix) children.add("key.toml");
  return [...children];
};
globalThis.siglusKnownFileCount = () => files.length + 1;
const base = {exists: globalThis.siglusFileExists, read: globalThis.siglusReadFile, list: globalThis.siglusListDir};
const projectId = await (await fetch("/project-id")).text();
const scope = await saveScope(JSON.stringify({projectId, language: translated ? "chs" : "original"}));
const saves = await SiglusSaveStore.open(scope, base, error => {
  state.error = "Save persistence failed: " + error;
  document.querySelector("#status").textContent = state.error;
  console.error(state.error);
});
saves.bind();
const writeFile = globalThis.siglusWriteFile;
globalThis.siglusWriteFile = (path, bytes) => {
  writeFile(path, bytes);
  state.writes.push({path, bytes: bytes.length});
};
globalThis.siglusSaves = {
  flush: () => saves.flush(),
  export: () => saves.export(),
  import: snapshot => {
    if (state.started) throw new Error("Import saves before starting the engine");
    return saves.import(snapshot);
  },
  list: () => [...saves.files].filter(([, bytes]) => bytes !== null).map(([path, bytes]) => ({path, size: bytes.length})),
};
const module = await import("/pkg/game_launcher.js");
const {default: init, start_siglus_from_directory} = module;
const wasm = await init();
globalThis.createMovieProbe = () => new module.CompatibilityMovieProbe();
globalThis.runFilesystemProbe = () => module.compatibility_filesystem_probe();
Object.defineProperty(state, "wasmBytes", {enumerable: true, get: () => wasm.memory.buffer.byteLength});
const status = document.querySelector("#status");
const start = document.querySelector("#start");
status.textContent = `${translated ? "汉化资源" : "原版资源"} · ${files.length} files`;
start.disabled = false;
start.onclick = () => {
  try {
    document.querySelector("canvas").focus();
    start_siglus_from_directory("siglus-canvas", JSON.stringify(files));
    state.started = true;
    start.disabled = true;
    status.textContent += " · running";
  } catch (error) {
    state.error = String(error);
    status.textContent = state.error;
    throw error;
  }
};
