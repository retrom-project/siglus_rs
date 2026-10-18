// Synchronous engine view, asynchronous durable commit. Hosts must await flush()
// before reporting an export/close as successful. No selected game file is changed.
export function savePath(path) {
  const parts = String(path).replaceAll("\\", "/").split("/").filter(p => p && p !== ".");
  if (parts.some(p => p === ".." || p.includes(":")) || String(path).startsWith("/")) {
    throw new Error("Save path must be relative to the game: " + path);
  }
  return parts.join("/").replace(/[A-Z]/g, c => c.toLowerCase());
}

function transaction(db, mode, action) {
  return new Promise((resolve, reject) => {
    const tx = db.transaction("files", mode);
    let result;
    try { result = action(tx.objectStore("files")); }
    catch (error) { tx.abort(); reject(error); return; }
    tx.oncomplete = () => resolve(result?.result);
    tx.onabort = tx.onerror = () => reject(tx.error || new Error("Save transaction aborted"));
  });
}

export class SiglusSaveStore {
  // Provider hosts own durable checkpoints and user isolation. A fresh runtime
  // must receive only its authorized restore, never another user's local files.
  static transient(scope, base, onError = console.error) {
    return new SiglusSaveStore(scope, null, base, onError);
  }
  static async open(scope, base, onError = console.error) {
    if (!scope) throw new Error("A stable game identity is required for saves");
    const db = await new Promise((resolve, reject) => {
      const request = indexedDB.open("siglus-saves", 1);
      request.onupgradeneeded = () => request.result.createObjectStore("files");
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
      request.onblocked = () => reject(new Error("Save database is blocked by another page"));
    });
    const store = new SiglusSaveStore(scope, db, base, onError);
    const range = IDBKeyRange.bound([scope, ""], [scope, "\uffff"]);
    const rows = await transaction(db, "readonly", table => table.getAll(range));
    for (const row of rows) store.files.set(row.path, row.bytes);
    return store;
  }

  constructor(scope, db, base, onError) {
    this.scope = scope;
    this.db = db;
    this.base = base;
    this.onError = onError;
    this.files = new Map(); // null hides a removed base file, including after reload
    this.directories = new Set();
    this.pending = Promise.resolve();
    this.failure = null;
    this.closed = false;
  }

  check() {
    if (this.failure) throw this.failure;
    if (this.closed) throw new Error("Save store is closed");
  }

  commit(changes) {
    this.check();
    for (const [path, bytes] of changes) this.files.set(path, bytes);
    // Capture copies now: a later overwrite or wasm memory growth must not
    // change bytes belonging to an earlier transaction.
    this.pending = this.pending.then(() => {
      this.check();
      if (!this.db) return;
      return transaction(this.db, "readwrite", table => {
        for (const [path, bytes] of changes) table.put({path, bytes}, [this.scope, path]);
      });
    }).catch(error => {
      this.failure = error;
      this.onError(error);
    });
  }

  exists(path) {
    path = savePath(path);
    return this.files.has(path) ? this.files.get(path) !== null : this.base.exists(path);
  }

  read(path) {
    path = savePath(path);
    if (!this.exists(path)) throw new Error("File not found: " + path);
    return this.files.has(path) ? this.files.get(path) : this.base.read(path);
  }

  size(path) {
    path = savePath(path);
    if (!this.exists(path)) throw new Error("File not found: " + path);
    return this.files.has(path) ? this.files.get(path).length : this.base.size(path);
  }

  readRange(path, offset, length) {
    path = savePath(path);
    const size = this.size(path);
    if (!Number.isSafeInteger(offset) || !Number.isSafeInteger(length) || offset < 0 || length < 0 || offset > size - length) {
      throw new Error("Invalid filesystem range");
    }
    return this.files.has(path) ? this.files.get(path).slice(offset, offset + length) : this.base.readRange(path, offset, length);
  }

  write(path, bytes) {
    path = savePath(path);
    if (!path) throw new Error("Empty save filename");
    this.commit([[path, new Uint8Array(bytes)]]);
  }

  mkdir(path) { this.check(); this.directories.add(savePath(path)); }

  remove(path) {
    path = savePath(path);
    if (!this.exists(path)) throw new Error("File not found: " + path);
    this.commit([[path, null]]);
  }

  rename(from, to) {
    from = savePath(from); to = savePath(to);
    if (!to) throw new Error("Empty save filename");
    const move = data => {if (from !== to) this.commit([[to, new Uint8Array(data)], [from, null]]);};
    const data = this.read(from);
    return data && typeof data.then === "function" ? data.then(move) : move(data);
  }

  list(path) {
    path = savePath(path);
    const prefix = path ? path + "/" : "";
    const children = new Set(this.base.list(path));
    for (const [file, bytes] of this.files) {
      if (!file.startsWith(prefix)) continue;
      const child = file.slice(prefix.length).split("/")[0];
      if (bytes !== null) children.add(child);
      else if (file === prefix + child) children.delete(child);
    }
    for (const directory of this.directories) {
      if (directory.startsWith(prefix) && directory !== path) children.add(directory.slice(prefix.length).split("/")[0]);
    }
    return [...children];
  }

  async flush() {
    let pending;
    do { pending = this.pending; await pending; } while (pending !== this.pending);
    this.check();
  }

  async export() {
    await this.flush();
    const snapshotFiles = new Map(this.files);
    // Include original PC save files when they are mounted read-only with the
    // game. Export must be portable, not just a delta over that installation.
    const visit = async directory => {
      for (const child of this.base.list(directory)) {
        const path = savePath(directory + "/" + child);
        if (this.base.exists(path)) {
          if (!snapshotFiles.has(path)) snapshotFiles.set(path, new Uint8Array(await this.base.read(path)));
        } else await visit(path);
      }
    };
    await visit("savedata");
    return {format: "siglus-save-files", version: 1, scope: this.scope,
      files: [...snapshotFiles].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([path, bytes]) => {
        if (bytes === null) return {path, data: null};
        let binary = "";
        for (let i = 0; i < bytes.length; i += 8192) binary += String.fromCharCode(...bytes.subarray(i, i + 8192));
        return {path, data: btoa(binary)};
      })};
  }

  // Replace the overlay in one transaction; validate everything before touching
  // live data. Import is only allowed before the engine starts (enforced by host).
  async import(snapshot) {
    await this.flush();
    if (snapshot.format !== "siglus-save-files" || snapshot.version !== 1 || snapshot.scope !== this.scope || !Array.isArray(snapshot.files)) {
      throw new Error("Save archive belongs to another game or has an invalid format");
    }
    const replacement = new Map();
    for (const {path, data} of snapshot.files) {
      const normalized = savePath(path);
      if (!normalized || normalized !== path || replacement.has(path) || !(data === null || typeof data === "string")) throw new Error("Invalid save archive entry");
      replacement.set(path, data === null ? null : Uint8Array.from(atob(data), c => c.charCodeAt(0)));
    }
    if (this.db) await transaction(this.db, "readwrite", table => {
      table.delete(IDBKeyRange.bound([this.scope, ""], [this.scope, "\uffff"]));
      for (const [path, bytes] of replacement) table.put({path, bytes}, [this.scope, path]);
    });
    this.files = replacement;
    this.directories.clear();
  }

  async close() { await this.flush(); this.closed = true; this.db?.close(); }

  bind(target = globalThis) {
    target.siglusFileExists = path => this.exists(path);
    target.siglusReadFile = path => this.read(path);
    target.siglusFileSize = path => this.size(path);
    target.siglusReadFileRange = (path, offset, length) => this.readRange(path, offset, length);
    target.siglusListDir = path => this.list(path);
    target.siglusWriteFile = (path, bytes) => this.write(path, bytes);
    target.siglusCreateDir = path => this.mkdir(path);
    target.siglusRemoveFile = path => this.remove(path);
    target.siglusRenameFile = (from, to) => this.rename(from, to);
  }
}

export async function saveScope(identity) {
  const bytes = new TextEncoder().encode(identity);
  const hash = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(hash)].map(b => b.toString(16).padStart(2, "0")).join("");
}
