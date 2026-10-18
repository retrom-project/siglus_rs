import assert from "node:assert/strict";
import {test} from "node:test";
import {projectView} from "./project.mjs";
import {SiglusSaveStore} from "../wasm/siglus_save_store.js";

test("metadata-only CHS mount routes selected ranges to original indexed paths", async () => {
  const paths = ["Scene.pck", "Gameexe.dat", "Scene.chs", "Gameexe.chs", "G00/UI.g00", "G00/UI.g01", "BGM/unused.nwa", "SiglusEngine.exe.bak"];
  const calls = [];
  const view = projectView(paths.map(path => ({path, sizeBytes: 600000})), "chs", async (...args) => {
    calls.push(args); return new Uint8Array(args[2]);
  });
  assert.equal(view.size("SCENE.PCK"), 600000);
  assert.equal(view.exists("g00/ui.g00"), true);
  assert.ok(view.list("").includes("bgm"));
  assert.deepEqual(calls, []);
  await view.readRange("G00\\UI.G00", 20, 40);
  assert.deepEqual(calls.splice(0), [["G00/UI.g01", 20, 40]]);
  assert.equal((await view.original("Scene.pck")).length, 600000);
  assert.deepEqual(calls.splice(0), [["Scene.pck", 0, 262144], ["Scene.pck", 262144, 262144], ["Scene.pck", 524288, 75712]]);
  await view.readRange("scene.pck", 0, 8);
  assert.deepEqual(calls, [["Scene.chs", 0, 8]]);
  assert.throws(() => view.readRange("scene.pck", 599999, 2), /BOUNDS/);
  assert.throws(() => view.readRange("scene.pck", -1, 1), /BOUNDS/);
  assert.throws(() => view.readRange("../scene.pck", 0, 1), /PATH_INVALID/);
});

test("original-language reads and short reads preserve source and error semantics", async () => {
  const view = projectView([{path: "Scene.pck", sizeBytes: 4}, {path: "Scene.chs", sizeBytes: 4}], "original",
    async path => {assert.equal(path, "Scene.pck"); return new Uint8Array(3);});
  await assert.rejects(view.read("SCENE.PCK"), /TRUNCATED/);
});

test("save ranges stay local while portable export awaits only original save files", async () => {
  const calls = [];
  const base = projectView([{path: "SaveData/0001.sav", sizeBytes: 4}, {path: "BGM/unused.nwa", sizeBytes: 100}], "original",
    async (path, offset, length) => {calls.push([path, offset, length]); return Uint8Array.of(1, 2, 3, 4).slice(offset, offset + length);});
  const store = SiglusSaveStore.transient("game:original", base);
  store.write("savedata/0002.sav", Uint8Array.of(5, 6, 7));
  assert.deepEqual(store.readRange("savedata/0002.sav", 1, 2), Uint8Array.of(6, 7));
  assert.equal(store.size("savedata/0001.sav"), 4);
  assert.deepEqual(calls, []);
  const snapshot = await store.export();
  assert.deepEqual(calls, [["SaveData/0001.sav", 0, 4]]);
  const restored = SiglusSaveStore.transient("game:original", base);
  await restored.import(snapshot);
  assert.deepEqual(restored.readRange("savedata/0001.sav", 0, 4), Uint8Array.of(1, 2, 3, 4));
  await store.close(); await restored.close();
});

test("renaming a base save commits only after its asynchronous read succeeds", async () => {
  let finish, failed;
  const base = projectView([{path: "savedata/0001.sav", sizeBytes: 3}], "original",
    () => new Promise((resolve, reject) => {finish = resolve; failed = reject;}));
  const store = SiglusSaveStore.transient("game:original", base);
  const first = store.rename("savedata/0001.sav", "savedata/0002.sav");
  assert.equal(store.exists("savedata/0001.sav"), true);
  assert.equal(store.exists("savedata/0002.sav"), false);
  failed(new Error("range failed")); await assert.rejects(first, /range failed/);
  assert.equal(store.exists("savedata/0001.sav"), true);
  assert.equal(store.exists("savedata/0002.sav"), false);
  const second = store.rename("savedata/0001.sav", "savedata/0002.sav");
  finish(Uint8Array.of(7, 8, 9)); await second;
  assert.equal(store.exists("savedata/0001.sav"), false);
  assert.deepEqual(store.readRange("savedata/0002.sav", 0, 3), Uint8Array.of(7, 8, 9));
  await store.close();
});
