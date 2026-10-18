const normalize = path => {
  const parts = String(path).replaceAll("\\", "/").split("/").filter(p => p && p !== ".");
  if (parts.some(p => p === ".." || p.includes(":"))) throw new Error("SIGLUS_PROJECT_PATH_INVALID");
  return parts.join("/").replace(/[A-Z]/g, c => c.toLowerCase());
};

// Only filenames and sizes live here. The embedding's Content I/O reader owns
// all bytes, including after an explicit complete preload.
export function projectView(files, language, readFile) {
  const index = new Map();
  for (const {path, sizeBytes} of files) {
    const name = normalize(path);
    if (!name || index.has(name) || !Number.isSafeInteger(sizeBytes) || sizeBytes < 0) throw new Error("SIGLUS_PROJECT_INVALID");
    index.set(name, {path, sizeBytes});
  }
  const generated = new Map();
  if (language !== "original" && language !== "chs") throw new Error("SIGLUS_LANGUAGE_INVALID");
  if (language === "chs" && (!index.has("scene.chs") || !index.has("gameexe.chs"))) {
    throw new Error("SIGLUS_TRANSLATION_MISSING");
  }
  const resolve = path => {
    let name = normalize(path);
    if (language === "chs") {
      if (name === "scene.pck") name = "scene.chs";
      if (name === "gameexe.dat") name = "gameexe.chs";
      if (name.endsWith(".g00") && index.has(name.slice(0, -4) + ".g01")) name = name.slice(0, -4) + ".g01";
    }
    return name;
  };
  const size = name => {
    if (generated.has(name)) return generated.get(name).length;
    const file = index.get(name);
    if (!file) throw new Error("SIGLUS_FILE_MISSING");
    return file.sizeBytes;
  };
  const range = (name, offset, length) => {
    const total = size(name);
    if (!Number.isSafeInteger(offset) || !Number.isSafeInteger(length) || offset < 0 || length < 0 || offset > total - length) {
      throw new Error("SIGLUS_FILE_BOUNDS");
    }
    if (generated.has(name)) return generated.get(name).slice(offset, offset + length);
    return readFile(index.get(name).path, offset, length);
  };
  const read = async name => {
    const bytes = new Uint8Array(size(name));
    for (let offset = 0; offset < bytes.length; offset += 256 * 1024) {
      const length = Math.min(256 * 1024, bytes.length - offset);
      const part = await range(name, offset, length);
      if (part.length !== length) throw new Error("SIGLUS_FILE_TRUNCATED");
      bytes.set(part, offset);
    }
    return bytes;
  };
  return {
    count: () => index.size + generated.size,
    set: (path, bytes) => generated.set(normalize(path), new Uint8Array(bytes)),
    original: path => read(normalize(path)),
    exists: path => index.has(resolve(path)) || generated.has(resolve(path)),
    size: path => size(resolve(path)),
    read: path => read(resolve(path)),
    readRange: (path, offset, length) => range(resolve(path), offset, length),
    list: path => {
      const prefix = normalize(path), start = prefix ? prefix + "/" : "";
      return [...new Set([...index.keys(), ...generated.keys()].filter(name => name.startsWith(start))
        .map(name => name.slice(start.length).split("/")[0]).filter(Boolean))];
    },
  };
}
