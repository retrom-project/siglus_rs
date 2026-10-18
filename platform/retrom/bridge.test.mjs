import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import {test} from "node:test";
import vm from "node:vm";

const source = await readFile(new URL("./bridge.mjs", import.meta.url), "utf8");

async function fixture(failure, hooks = {}) {
  const contexts = [];
  const events = new Map(), frames = new Map(), calls = [];
  let nextFrame = 0;
  const target = {
    addEventListener(type, handler) {events.set(type, handler);},
    removeEventListener(type) {events.delete(type);},
  };
  class AudioContext {
    state = "running";
    constructor() {contexts.push(this);}
    async suspend() {this.state = "suspended";}
    async resume() {this.state = "running";}
    async close() {this.state = "closed";}
  }
  const context = vm.createContext({AudioContext, Set, Blob, failure, hooks, calls,
    window: target,
    requestAnimationFrame: callback => {frames.set(++nextFrame, callback); return nextFrame;},
    cancelAnimationFrame: id => frames.delete(id),
  });
  const modules = new Map();
  const module = (name, code) => {
    const result = new vm.SourceTextModule(code, {context, identifier: name});
    modules.set(name, result); return result;
  };
  // web-sys caches the platform constructor when its generated module is
  // evaluated. Preserve that timing: replacing the global afterwards is late.
  module("./pkg/siglus_scene_vm.js", `
    const CachedAudioContext = AudioContext;
    export default async () => {if (failure === "init") throw Error("init failed");};
    export function retrom_resource_key() {return "";}
    export class RetromRenderer {static async create() {return new RetromRenderer();}}
    export class RetromGame {
      static async create() {
        new CachedAudioContext();
        if (failure === "create") throw Error("create failed");
        return new RetromGame();
      }
      width() {return 1280;} height() {return 720;}
      async step() {calls.push("step"); return hooks.step ? await hooks.step() : false;}
      persist() {calls.push("persist");} stop() {calls.push("stop"); hooks.stop?.();}
      free() {calls.push("free");} paused() {calls.push("paused");}
      key() {calls.push("key");} text() {calls.push("text");} volume() {calls.push("volume");}
      pointer(...args) {calls.push("pointer"); return hooks.pointer?.(...args);}
    }
  `);
  module("./project.mjs", `export const projectView = () => ({exists: () => true, count: () => 0});`);
  module("../wasm/siglus_save_store.js", `
    export class SiglusSaveStore {
      static transient() {return new SiglusSaveStore();}
      bind() {} async close() {} list() {return [];} async export() {return {files: []};}
    }
  `);
  const load = async name => {
    const result = modules.get(name);
    if (!result) throw Error("unexpected import: " + name);
    if (result.status === "unlinked") await result.link(load);
    if (result.status === "linked") await result.evaluate();
    return result;
  };
  const bridge = new vm.SourceTextModule(source, {context, identifier: "bridge.mjs", importModuleDynamically: load});
  await bridge.link(load); await bridge.evaluate();
  const create = () => bridge.namespace.create({canvas: {
    getContext() {}, focus() {}, getBoundingClientRect() {return {left: 0, top: 0, width: 1280, height: 720};}, ...target,
  }, files: [], language: "chs", identity: "test", onFailure: error => {throw error;}});
  return {create, contexts, context, AudioContext, calls, events, frames,
    frame() {const [id, callback] = frames.entries().next().value; frames.delete(id); callback();}};
}

test("pause, resume and close control the context created by cached wasm imports", async () => {
  const f = await fixture();
  const session = await f.create();
  assert.equal(f.contexts.length, 1);
  await session.pause(); assert.equal(f.contexts[0].state, "suspended");
  await session.resume(); assert.equal(f.contexts[0].state, "running");
  await session.close(); assert.equal(f.contexts[0].state, "closed");
  assert.equal(f.context.AudioContext, f.AudioContext);
  await session.close();
});

test("initialization failure restores the browser constructor", async () => {
  const f = await fixture("init");
  await assert.rejects(f.create(), /init failed/);
  assert.equal(f.context.AudioContext, f.AudioContext);
});

test("failed game creation closes audio allocated before the error", async () => {
  const f = await fixture("create");
  await assert.rejects(f.create(), /create failed/);
  assert.equal(f.contexts[0].state, "closed");
  assert.equal(f.context.AudioContext, f.AudioContext);
});

test("input, pause and checkpoint wait for the suspended engine step", async () => {
  const hooks = {}, f = await fixture(undefined, hooks), session = await f.create();
  let finish;
  hooks.step = () => new Promise(resolve => {finish = resolve;});
  f.calls.length = 0;
  f.frame(); await Promise.resolve();
  f.events.get("keydown")({type: "keydown", code: "Enter", key: "Enter", preventDefault() {}});
  const pause = session.pause(), checkpoint = session.export();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(f.calls, ["step"]);
  finish(false); await pause; await checkpoint;
  assert.deepEqual(f.calls.slice(1).sort(), ["key", "paused", "persist"]);
  assert.equal(f.frames.size, 0);
  hooks.step = undefined;
  await session.resume(); assert.equal(f.frames.size, 1);
  await session.close();
});

test("closing a suspended step waits for unwind without rescheduling or reporting an abort", async () => {
  const hooks = {}, f = await fixture(undefined, hooks), session = await f.create();
  let reject;
  hooks.step = () => new Promise((_, failed) => {reject = failed;});
  f.calls.length = 0;
  f.frame(); await Promise.resolve();
  const closing = session.close();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(f.calls, ["step"]);
  assert.equal(f.contexts[0].state, "running");
  reject(new Error("Content I/O cancelled"));
  await closing;
  assert.deepEqual(f.calls, ["step", "persist", "stop", "free"]);
  assert.equal(f.contexts[0].state, "closed");
  assert.equal(f.frames.size, 0);
  assert.equal(f.events.size, 0);
});

test("a pointer callback can suspend for content before checkpoint or shutdown enters Wasm", async () => {
  const hooks = {}, f = await fixture(undefined, hooks), session = await f.create();
  let finish;
  const reading = new Promise(resolve => {finish = resolve;});
  hooks.pointer = () => reading;
  f.calls.length = 0;
  f.events.get("pointerdown")({type: "pointerdown", clientX: 10, clientY: 20, button: 0, preventDefault() {}});
  await new Promise(resolve => setImmediate(resolve));
  const checkpoint = session.export();
  try {
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(f.calls, ["pointer"]);
  } finally {finish(); await checkpoint; await session.close();}
});

test("shutdown still releases Wasm and audio when stopping the engine throws", async () => {
  const hooks = {stop() {throw Error("stop failed");}}, f = await fixture(undefined, hooks), session = await f.create();
  await assert.rejects(session.close(), /stop failed/);
  assert.equal(f.contexts[0].state, "closed");
  assert.ok(f.calls.includes("free"));
  assert.equal(f.context.AudioContext, f.AudioContext);
});
