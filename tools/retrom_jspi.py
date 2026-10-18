"""Connect wasm-bindgen's pinned generated ABI to JS promise integration.

Downloads, ranges, cache and cancellation remain in the embedding's Content I/O.
"""
import re


def once(source, old, new):
    if source.count(old) != 1:
        raise ValueError("SIGLUS_JSPI_GLUE_MISMATCH: " + old)
    return source.replace(old, new)


def enable_jspi(source):
    for name in ["siglusReadFileRange", "siglusRenameFile"]:
        pattern = (r"(__wbg_" + name + r"_[a-z0-9]+): function\(\) \{ return handleError\(function \(([^)]*)\) \{\n"
                   r"            (?:const ret = )?" + name + r"\(([^\n]*)\);\n"
                   r"(?:            return addHeapObject\(ret\);\n)?        \}, arguments\); \},")
        convert = "addHeapObject" if name == "siglusReadFileRange" else "value => value"
        replacement = (r"\1: new WebAssembly.Suspending(function() { return handleError(function (\2) {\n"
                       r"            const ret = " + name + r"(\3), convert = " + convert + r";\n"
                       r"            return ret && typeof ret.then === 'function' ? ret.then(convert).catch(error => handleError(() => {throw error;}, [])) : convert(ret);\n"
                       r"        }, arguments); }),")
        source, count = re.subn(pattern, replacement, source)
        if count != 1:
            raise ValueError("SIGLUS_JSPI_IMPORT_MISMATCH: " + name)
    # Hover and key processing can load resources too. Every mutable host entry
    # must be a promise root, not just the frame loop and checkpoint writer.
    entries = [("static create(prepared)", "retromgame_create"),
               ("step(dt_ms)", "retromgame_step"), ("persist()", "retromgame_persist"),
               ("pointer(x, y, button, down)", "retromgame_pointer"), ("key(code, down)", "retromgame_key"),
               ("wheel(delta)", "retromgame_wheel"), ("text(text)", "retromgame_text"),
               ("volume(volume)", "retromgame_volume"), ("paused(paused)", "retromgame_paused"),
               ("stop()", "retromgame_stop"),
               ("export function siglus_wasm_vfs_read_len(path)", "siglus_wasm_vfs_read_len")]
    host = re.search(r"export class RetromGame \{[\s\S]*?\n\}", source)
    if host is None:
        raise ValueError("SIGLUS_JSPI_GLUE_MISMATCH: RetromGame")
    original_host = host.group()
    converted_host = original_host
    for signature, export in entries:
        target = source if signature.startswith("export ") else converted_host
        replacement = signature.replace("static ", "static async ") if signature.startswith("static ") else (
            signature.replace("export function ", "export async function ") if signature.startswith("export ") else "async " + signature)
        target = once(target, signature + " {", replacement + " {")
        target = once(target, "        wasm." + export + "(", "        await wasm." + export + "(")
        if signature.startswith("export "):
            source = target
        else:
            converted_host = target
    source = once(source, original_host, converted_host)
    exports = [export for _, export in entries]
    wrapped = ", ".join(name + ": WebAssembly.promising(instance.exports." + name + ")" for name in exports)
    source = once(source, "    wasm = instance.exports;", "    wasm = {...instance.exports, " + wrapped + "};")
    source = once(source, "function __wbg_get_imports() {", "function __wbg_get_imports() {\n"
                  "    if (typeof WebAssembly.Suspending !== 'function' || typeof WebAssembly.promising !== 'function') "
                  "throw new Error('SIGLUS_BROWSER_ASYNC_WASM_UNAVAILABLE');")
    return source
