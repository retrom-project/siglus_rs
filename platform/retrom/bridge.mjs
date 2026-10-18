import {SiglusSaveStore} from "../wasm/siglus_save_store.js";
import {projectView} from "./project.mjs";

export const abi = "siglus-host-v1";
export async function create(options) {
  const {canvas, files, language, identity, restore, onFailure, onExit} = options;
  let revision = 0, acknowledged = restore ? 0 : -1, frames = 0, stopped = false, paused = false, request = 0;
  const audio = audioContexts();
  let game, store;
  let tail = Promise.resolve(), closing;
  const serial = operation => {
    const result = tail.then(operation); tail = result.catch(() => {}); return result;
  };
  const releases = [];
  const listen = (target, type, handler, options) => {
    target.addEventListener(type, handler, options);
    releases.push(() => target.removeEventListener(type, handler, options));
  };
  const pressed = new Set();
  const buttons = new Set();
  let position = {x: 0, y: 0};
  const releaseInput = async () => {
    try {
      for (const key of pressed) await game?.key(key, false);
      for (const button of buttons) await game?.pointer(position.x, position.y, button, false);
    } finally {pressed.clear(); buttons.clear();}
  };
  const close = () => {
    if (closing) return closing;
    stopped = true; cancelAnimationFrame(request);
    for (const release of releases) release();
    return closing = serial(async () => {
      try {await releaseInput(); await game?.persist(); await store?.close();}
      finally {
        try {await game?.stop();}
        finally {try {game?.free();} finally {await audio.close();}}
      }
    });
  };
  const fail = error => {if (!stopped) {void close().catch(onFailure); onFailure(error);}};
  const input = operation => {void serial(() => {if (!stopped && !paused) return operation();}).catch(fail);};
  try {
    // wasm-bindgen caches AudioContext while evaluating its module. Install
    // lifecycle tracking first, including for its temporary startup context.
    const {default: init, RetromRenderer, RetromGame, retrom_resource_key} = await import("./pkg/siglus_scene_vm.js");
    await init({module_or_path: options.wasmURL});
    const view = projectView(files, language, options.readFile);
    if (!view.exists("key.toml")) {
      let config = "override_string_encryption = \"mdl\"\n";
      if (view.exists("Gameexe.dat")) config = retrom_resource_key(await view.original("Gameexe.dat"), await view.original("Scene.pck"));
      view.set("key.toml", new TextEncoder().encode(config));
    }
    store = SiglusSaveStore.transient(identity, view, onFailure);
    if (restore) await store.import(restore);
    store.bind(); globalThis.siglusKnownFileCount = () => view.count();
    const write = globalThis.siglusWriteFile;
    globalThis.siglusWriteFile = (path, bytes) => {write(path, bytes); revision++;};
    const remove = globalThis.siglusRemoveFile;
    globalThis.siglusRemoveFile = path => {remove(path); revision++;};
    const rename = globalThis.siglusRenameFile;
    globalThis.siglusRenameFile = (from, to) => {
      const result = rename(from, to);
      if (result && typeof result.then === "function") return result.then(() => {revision++;});
      revision++;
    };
    canvas.getContext("webgl2", {preserveDrawingBuffer: true, alpha: false, antialias: false});
    const renderer = await RetromRenderer.create(canvas);
    game = await RetromGame.create(renderer);
    canvas.width = game.width(); canvas.height = game.height();
    const move = event => {
      const rect = canvas.getBoundingClientRect();
      position = {x: (event.clientX - rect.left) * canvas.width / Math.max(1, rect.width),
        y: (event.clientY - rect.top) * canvas.height / Math.max(1, rect.height)};
      if (event.type === "pointerdown") buttons.add(event.button);
      if (event.type === "pointerup") buttons.delete(event.button);
      const {x, y} = position, button = event.type === "pointermove" ? -1 : event.button, down = event.type === "pointerdown";
      input(() => game.pointer(x, y, button, down));
    };
    for (const type of ["pointermove", "pointerdown", "pointerup"]) listen(canvas, type, event => {
      if (stopped || paused) return;
      if (type === "pointerdown") {
        canvas.focus();
        if (event.isTrusted) canvas.setPointerCapture?.(event.pointerId);
        void audio.resume().catch(onFailure);
      }
      move(event); event.preventDefault();
    });
    listen(canvas, "contextmenu", event => event.preventDefault());
    listen(canvas, "wheel", event => {const delta = Math.sign(event.deltaY); input(() => game.wheel(delta)); event.preventDefault();}, {passive: false});
    const key = event => {
      if (paused || stopped || event.isComposing) return;
      const code = keyCode(event.code);
      const down = event.type === "keydown", text = event.key;
      if (code !== null) {input(() => game.key(code, down)); if (down) pressed.add(code); else pressed.delete(code); event.preventDefault();}
      if (down && text.length === 1 && !event.ctrlKey && !event.metaKey) input(() => game.text(text));
      if (event.type === "keydown") void audio.resume().catch(onFailure);
    };
    listen(window, "keydown", key); listen(window, "keyup", key); listen(window, "blur", () => input(releaseInput));
    const tick = () => {
      if (stopped || paused) return;
      void serial(() => game.step(16)).then(exited => {
        frames++;
        if (stopped || paused) return;
        if (exited) {void onExit?.(); return;}
        request = requestAnimationFrame(tick);
      }).catch(fail);
    };
    await serial(() => game.step(16)); frames++; request = requestAnimationFrame(tick);
    const progress = () => {
      const visit = path => store.list(path).some(child => {
        const name = path + "/" + child;
        return store.exists(name) ? /(?:^|\/)\d+\.sav$/i.test(name) && store.size(name) > 0 : visit(name);
      });
      return visit("savedata");
    };
    return {
      frames: () => frames,
      status: () => ({progress: progress(), revision: String(revision), changed: revision !== acknowledged}),
      acknowledge: captured => {if (String(revision) === captured) acknowledged = revision;},
      export: () => serial(async () => {await game.persist(); return {snapshot: await store.export(), revision: String(revision)};}),
      pause: async () => {
        if (stopped || paused) return;
        paused = true; cancelAnimationFrame(request); await audio.pause();
        await serial(async () => {await releaseInput(); await game.paused(true);});
      },
      resume: async () => {
        if (stopped || !paused) return;
        await serial(async () => {await audio.resume(); await game.paused(false);});
        if (!stopped) {paused = false; request = requestAnimationFrame(tick);}
      },
      volume: value => {void serial(() => {if (!stopped) return game.volume(value);}).catch(fail);},
      close,
    };
  } catch (error) {await close(); throw error;}
}

function keyCode(code) {
  const keys = {Enter: 13, NumpadEnter: 13, Escape: 27, Space: 32, Backspace: 8, Tab: 9, Delete: 46,
    ArrowLeft: 37, ArrowUp: 38, ArrowRight: 39, ArrowDown: 40, Home: 36, End: 35,
    ShiftLeft: 16, ShiftRight: 16, ControlLeft: 17, ControlRight: 17};
  if (code in keys) return keys[code];
  if (/^Key[A-Z]$/.test(code)) return code.charCodeAt(3);
  if (/^Digit[0-9]$/.test(code)) return code.charCodeAt(5);
  if (/^F(?:[1-9]|1[0-2])$/.test(code)) return 111 + Number(code.slice(1));
  return null;
}

function audioContexts() {
  const Original = globalThis.AudioContext, contexts = new Set();
  if (Original) globalThis.AudioContext = new Proxy(Original, {construct(target, args) {
    const context = Reflect.construct(target, args); contexts.add(context); return context;
  }});
  const action = async method => {
    for (const context of contexts) if (context.state !== "closed") await context[method]();
  };
  return {pause: () => action("suspend"), resume: () => action("resume"), close: async () => {
    try {await action("close");} finally {if (Original) globalThis.AudioContext = Original; contexts.clear();}
  }};
}

globalThis.retromSiglus = {abi, create};
