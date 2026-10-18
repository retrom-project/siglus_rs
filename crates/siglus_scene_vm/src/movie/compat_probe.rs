//! Feature-gated diagnostics exercising the same MovieManager as the game.
use super::MovieManager;
use std::path::PathBuf;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct CompatibilityMovieProbe {
    manager: MovieManager,
}

#[wasm_bindgen]
impl CompatibilityMovieProbe {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            manager: MovieManager::new(PathBuf::from(".")),
        }
    }

    pub fn poll(&mut self, file: &str, timer_ms: u32, loop_flag: bool) -> Result<JsValue, JsValue> {
        let frame = self
            .manager
            .poll_global_movie_frame_with_loop(file, timer_ms as u64, loop_flag)
            .map_err(|e| JsValue::from_str(&format!("{e:#}")))?;
        let stats = self.manager.debug_memory_stats();
        let value = js_sys::Object::new();
        let mut fields = vec![
            ("videoBytes", stats.video_bytes as f64),
            ("videoFrames", stats.video_frames as f64),
            ("audioBytes", stats.audio_pcm_bytes as f64),
            ("queuedVideoBytes", stats.queued_video_bytes as f64),
            ("queuedVideoFrames", stats.queued_video_frames as f64),
            ("activeStreams", stats.active_streams as f64),
            ("cachedAssets", stats.asset_cache_entries as f64),
        ];
        if let Some(frame) = frame {
            let checksum = frame
                .frame
                .rgba
                .iter()
                .step_by(257)
                .fold(2166136261u32, |hash, b| {
                    (hash ^ *b as u32).wrapping_mul(16777619)
                });
            fields.extend([
                ("frameIndex", frame.frame_idx as f64),
                ("durationMs", frame.total_ms.unwrap_or(0) as f64),
                ("width", frame.frame.width as f64),
                ("height", frame.frame.height as f64),
                ("checksum", checksum as f64),
                (
                    "clampedMs",
                    frame.clamped_timer_ms.map(|v| v as f64).unwrap_or(-1.0),
                ),
            ]);
        }
        for (name, number) in fields {
            js_sys::Reflect::set(&value, &JsValue::from_str(name), &JsValue::from_f64(number))?;
        }
        Ok(value.into())
    }

    pub fn stop(&mut self) -> u32 {
        self.manager.stop();
        self.manager.debug_memory_stats().video_bytes as u32
    }

    pub fn preview(&mut self, file: &str) -> Result<u32, JsValue> {
        self.manager
            .ensure_preview_frame(file)
            .map_err(|e| JsValue::from_str(&format!("{e:#}")))?;
        Ok(self.manager.debug_memory_stats().video_bytes as u32)
    }

    pub fn full_asset_error(&mut self, file: &str) -> Result<String, JsValue> {
        match self.manager.ensure_asset(file) {
            Err(error) => Ok(format!("{error:#}")),
            Ok(_) => Err(JsValue::from_str(
                "Web full movie asset API unexpectedly succeeded",
            )),
        }
    }

    pub fn idle(&mut self, frames: u32) -> u32 {
        for _ in 0..frames {
            self.manager.evict_idle_streams();
        }
        self.manager.debug_memory_stats().active_streams as u32
    }
}
