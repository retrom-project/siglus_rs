//! Controlled browser ABI. The embedding owns the frame loop and filesystem.
use crate::host::{SiglusHost, SiglusHostConfig};
use crate::render::Renderer;
use crate::runtime::input::VmMouseButton;
use std::path::PathBuf;
use wasm_bindgen::prelude::*;

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&format!("{error:#}"))
}

#[wasm_bindgen]
pub struct RetromRenderer {
    renderer: Renderer,
    canvas: web_sys::HtmlCanvasElement,
}

#[wasm_bindgen]
impl RetromRenderer {
    /// Browser device creation is asynchronous, before any filesystem access.
    pub async fn create(canvas: web_sys::HtmlCanvasElement) -> Result<RetromRenderer, JsValue> {
        console_error_panic_hook::set_once();
        let renderer = Renderer::new_canvas(canvas.clone())
            .await
            .map_err(js_error)?;
        Ok(Self { renderer, canvas })
    }
}

#[wasm_bindgen]
pub struct RetromGame {
    host: Option<Box<SiglusHost>>,
    global_hash: Option<u64>,
}

#[wasm_bindgen]
impl RetromGame {
    /// Kept on one Wasm stack so JSPI can suspend a synchronous filesystem read.
    pub fn create(prepared: RetromRenderer) -> Result<RetromGame, JsValue> {
        let RetromRenderer { renderer, canvas } = prepared;
        let config = SiglusHostConfig::new(PathBuf::from("."));
        let mut host = SiglusHost::new_with_renderer_sync(config, renderer).map_err(js_error)?;
        let (width, height) = host.logical_size();
        canvas.set_width(width);
        canvas.set_height(height);
        host.resize(width, height, 1.0);
        Ok(Self {
            host: Some(Box::new(host)),
            global_hash: None,
        })
    }

    pub fn step(&mut self, dt_ms: u32) -> Result<bool, JsValue> {
        let host = self
            .host
            .as_mut()
            .ok_or_else(|| js_error("SIGLUS_STOPPED"))?;
        host.step(dt_ms.min(100)).map_err(js_error)
    }

    pub fn persist(&mut self) -> Result<(), JsValue> {
        let host = self
            .host
            .as_mut()
            .ok_or_else(|| js_error("SIGLUS_STOPPED"))?;
        self.global_hash = Some(host.persist_global_if_changed(self.global_hash));
        Ok(())
    }

    pub fn width(&self) -> u32 {
        self.host.as_ref().map_or(0, |host| host.logical_size().0)
    }
    pub fn height(&self) -> u32 {
        self.host.as_ref().map_or(0, |host| host.logical_size().1)
    }

    pub fn pointer(&mut self, x: f64, y: f64, button: i32, down: bool) {
        if let Some(host) = self.host.as_mut() {
            host.mouse_move(x, y);
            let button = match button {
                0 => Some(VmMouseButton::Left),
                1 => Some(VmMouseButton::Middle),
                2 => Some(VmMouseButton::Right),
                _ => None,
            };
            if let Some(button) = button {
                if down {
                    host.mouse_down(button);
                } else {
                    host.mouse_up(button);
                }
            }
        }
    }

    pub fn wheel(&mut self, delta: i32) {
        if let Some(host) = self.host.as_mut() {
            host.mouse_wheel(delta);
        }
    }
    pub fn key(&mut self, code: i32, down: bool) {
        if let Some(host) = self.host.as_mut() {
            if down {
                host.key_down_code(code);
            } else {
                host.key_up_code(code);
            }
        }
    }
    pub fn text(&mut self, text: &str) {
        if let Some(host) = self.host.as_mut() {
            if host.editbox_accepts_direct_text() {
                host.text_input(text);
            }
        }
    }
    pub fn volume(&mut self, volume: f64) {
        if let Some(host) = self.host.as_mut() {
            host.vm_mut()
                .ctx
                .audio
                .set_host_volume(volume.clamp(0.0, 1.0));
        }
    }
    pub fn paused(&mut self, paused: bool) {
        crate::platform_time::set_browser_paused(paused);
    }
    pub fn stop(&mut self) {
        self.host.take();
        crate::platform_time::set_browser_paused(false);
    }
}

/// Recover from the original pair before installing a translated resource view.
/// No executable or patch DLL is ever run or needed by this ABI.
#[wasm_bindgen]
pub fn retrom_resource_key(game: &[u8], scene: &[u8]) -> Result<String, JsValue> {
    let mut config = String::from("override_string_encryption = \"mdl\"\n");
    if siglus_key_recovery::resources_require_exe_key(game, scene).map_err(js_error)? {
        let key = siglus_key_recovery::recover_key_from_resources(game, scene).map_err(js_error)?;
        config.push_str("key = [");
        for (index, byte) in key.iter().enumerate() {
            if index > 0 {
                config.push_str(", ");
            }
            config.push_str(&byte.to_string());
        }
        config.push_str("]\n");
    }
    Ok(config)
}
