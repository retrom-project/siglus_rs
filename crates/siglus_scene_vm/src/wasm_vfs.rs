#![cfg(target_arch = "wasm32")]

//! Browser directory-backed virtual file access for the Siglus wasm port.
//!
//! The browser cannot open native paths from wasm. The JavaScript launcher must
//! let the user select the original game directory, build a path -> File object
//! index, and expose the synchronous functions imported below. Rust only passes
//! normalized Siglus relative paths such as `Gameexe.ini`, `Scene.pck`,
//! `bgm/BGM016.nwa`, or `g00/sys10_tt01.g00`.
//!
//! This module intentionally uses path-only requests. File ids, offsets, and
//! browser handles are private to the JavaScript side. It does not package the
//! game and does not read file contents during directory scanning.

use anyhow::{Context, Result, anyhow, bail};
use js_sys::{Array, Uint8Array};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = siglusFileExists)]
    fn siglus_file_exists(path: &str) -> bool;

    #[wasm_bindgen(js_name = siglusFileSize)]
    fn siglus_file_size(path: &str) -> f64;

    #[wasm_bindgen(catch, js_name = siglusReadFileRange)]
    fn siglus_read_file_range(
        path: &str,
        offset: f64,
        length: u32,
    ) -> std::result::Result<Uint8Array, JsValue>;

    #[wasm_bindgen(js_name = siglusListDir)]
    fn siglus_list_dir(path: &str) -> Array;

    #[wasm_bindgen(js_name = siglusKnownFileCount)]
    fn siglus_known_file_count() -> u32;

    #[wasm_bindgen(catch, js_name = siglusWriteFile)]
    fn siglus_write_file(path: &str, bytes: &[u8]) -> std::result::Result<(), JsValue>;

    #[wasm_bindgen(catch, js_name = siglusCreateDir)]
    fn siglus_create_dir(path: &str) -> std::result::Result<(), JsValue>;

    #[wasm_bindgen(catch, js_name = siglusRemoveFile)]
    fn siglus_remove_file(path: &str) -> std::result::Result<(), JsValue>;

    #[wasm_bindgen(catch, js_name = siglusRenameFile)]
    fn siglus_rename_file(from: &str, to: &str) -> std::result::Result<(), JsValue>;
}

/// Path-based filesystem surface used by the wasm port.
///
/// The browser supplies metadata without I/O. Reads use bounded ranges through
/// the embedding's content reader; JSPI suspends the current Wasm stack while
/// that reader fetches a missing block. Resource and save formats stay native;
/// this module has no downloader or filesystem byte cache.
pub trait SiglusVfs {
    fn exists(&self, path: &str) -> bool;
    fn read_all(&self, path: &str) -> Result<Vec<u8>>;
    fn list_dir(&self, path: &str) -> Result<Vec<String>>;
    /// Writes update the synchronous view immediately. The host must expose a
    /// flush barrier for its asynchronous durable storage before export/exit.
    fn write_all(&self, path: &str, bytes: &[u8]) -> Result<()>;
    fn create_dir_all(&self, path: &str) -> Result<()>;
    fn remove_file(&self, path: &str) -> Result<()>;
    fn rename_file(&self, from: &str, to: &str) -> Result<()>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WasmDirectoryVfs;

impl WasmDirectoryVfs {
    pub fn new() -> Self {
        Self
    }

    pub fn known_file_count(&self) -> u32 {
        siglus_known_file_count()
    }
}

impl SiglusVfs for WasmDirectoryVfs {
    fn write_all(&self, path: &str, bytes: &[u8]) -> Result<()> {
        siglus_write_file(&normalize_siglus_path(path), bytes).map_err(js_io_error)
    }

    fn create_dir_all(&self, path: &str) -> Result<()> {
        siglus_create_dir(&normalize_siglus_path(path)).map_err(js_io_error)
    }

    fn remove_file(&self, path: &str) -> Result<()> {
        siglus_remove_file(&normalize_siglus_path(path)).map_err(js_io_error)
    }

    fn rename_file(&self, from: &str, to: &str) -> Result<()> {
        siglus_rename_file(&normalize_siglus_path(from), &normalize_siglus_path(to))
            .map_err(js_io_error)
    }

    fn exists(&self, path: &str) -> bool {
        siglus_file_exists(&normalize_siglus_path(path))
    }

    fn read_all(&self, path: &str) -> Result<Vec<u8>> {
        let normalized = normalize_siglus_path(path);
        if normalized.is_empty() {
            bail!("empty wasm resource path");
        }
        if !siglus_file_exists(&normalized) {
            bail!("wasm resource not found: {normalized}");
        }

        let size = siglus_file_size(&normalized);
        if !size.is_finite() || size < 0.0 || size.fract() != 0.0 || size > usize::MAX as f64 {
            bail!("invalid browser resource size: {normalized}");
        }
        let len = size as usize;
        let mut out = Vec::new();
        out.try_reserve_exact(len)
            .context("browser resource is too large to decode as a whole file")?;
        out.resize(len, 0);
        for start in (0..len).step_by(256 * 1024) {
            let end = (start + 256 * 1024).min(len);
            let arr = siglus_read_file_range(&normalized, start as f64, (end - start) as u32)
                .map_err(js_io_error)?;
            if arr.length() as usize != end - start {
                bail!("truncated browser resource range: {normalized}");
            }
            arr.copy_to(&mut out[start..end]);
        }
        Ok(out)
    }

    fn list_dir(&self, path: &str) -> Result<Vec<String>> {
        let normalized = normalize_siglus_path(path);
        let array = siglus_list_dir(&normalized);
        let mut out = Vec::with_capacity(array.length() as usize);
        for value in array.iter() {
            let Some(s) = value.as_string() else {
                return Err(anyhow!(
                    "siglusListDir returned a non-string entry for {normalized}"
                ));
            };
            out.push(s);
        }
        Ok(out)
    }
}

fn js_io_error(error: JsValue) -> anyhow::Error {
    let message = error
        .as_string()
        .or_else(|| {
            js_sys::Reflect::get(&error, &JsValue::from_str("message"))
                .ok()?
                .as_string()
        })
        .unwrap_or_else(|| format!("{error:?}"));
    anyhow!("browser filesystem: {message}")
}

/// Convenience export for JavaScript-side diagnostics.
#[wasm_bindgen]
pub fn siglus_wasm_vfs_file_count() -> u32 {
    WasmDirectoryVfs::new().known_file_count()
}

/// Convenience export for JavaScript-side diagnostics.
#[wasm_bindgen]
pub fn siglus_wasm_vfs_exists(path: &str) -> bool {
    WasmDirectoryVfs::new().exists(path)
}

/// Convenience export for JavaScript-side diagnostics.
#[wasm_bindgen]
pub fn siglus_wasm_vfs_read_len(path: &str) -> std::result::Result<u32, JsValue> {
    let bytes = WasmDirectoryVfs::new()
        .read_all(path)
        .with_context(|| format!("read wasm resource {path}"))
        .map_err(|e| JsValue::from_str(&format!("{e:#}")))?;
    Ok(bytes.len() as u32)
}

/// Exercise the actual writable Rust seam, including an expected host Error.
#[cfg(feature = "compatibility-probe")]
#[wasm_bindgen]
pub fn compatibility_filesystem_probe() -> std::result::Result<(), JsValue> {
    use crate::resource;
    use std::path::Path;
    let run = || -> Result<()> {
        let first = Path::new(".compat-test/one.sav");
        let copied = Path::new(".compat-test/two.sav");
        let moved = Path::new(".compat-test/three.sav");
        resource::create_dir_all(".compat-test")?;
        resource::write_file_bytes(first, [0, 255, 128, 42])?;
        resource::copy_file(first, copied)?;
        resource::rename_file(copied, moved)?;
        anyhow::ensure!(
            !resource::game_file_exists(copied),
            "rename left its source visible"
        );
        anyhow::ensure!(
            resource::read_file_bytes(moved)? == [0, 255, 128, 42],
            "save bytes changed"
        );
        resource::remove_file(first)?;
        resource::remove_file(moved)?;
        anyhow::ensure!(
            resource::remove_file(moved).is_err(),
            "missing delete must return an error"
        );
        Ok(())
    };
    run().map_err(|e| JsValue::from_str(&format!("{e:#}")))
}

pub fn normalize_siglus_path(path: &str) -> String {
    path.replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/")
}
