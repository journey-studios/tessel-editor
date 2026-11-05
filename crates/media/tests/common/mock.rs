//! Deterministic renderer double shared by the export and flow tests: it paints a flat color
//! chosen by the sequence frame of the plan, at exactly the size it is asked for.
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tessel_editor_core::EditorState;
use tessel_editor_core::backend::{BackendError, Capabilities, FrameResult, Output};
use tessel_editor_core::fixtures::{encode_png, video_frame_color};
use tessel_editor_core::plan::RenderPlan;
use tessel_editor_core::session::Renderer;

#[derive(Clone, Default)]
pub struct Settings {
    pub delay: Duration,
    pub fail_at: Option<usize>,
    pub noise: bool,
    pub max_size: Option<u64>,
}

/// Paints `video_frame_color(plan.frame)` at the requested size and records the frame asked.
pub struct Mock {
    pub seen: Arc<Mutex<Vec<i64>>>,
    pub rendered: Arc<AtomicUsize>,
    pub settings: Settings,
}

impl Mock {
    pub fn new(settings: Settings) -> Self {
        Self {
            seen: Arc::default(),
            rendered: Arc::default(),
            settings,
        }
    }
}

impl Renderer for Mock {
    fn render_frame(
        &mut self,
        _state: &EditorState,
        plan: RenderPlan,
        output: Output,
        _probes: Vec<(u32, u32)>,
        _timeout: Duration,
    ) -> Result<FrameResult, BackendError> {
        let frame = plan.frame.expect("sequence plans carry a frame");
        let count = {
            let mut seen = self.seen.lock().unwrap();
            seen.push(frame);
            seen.len()
        };
        if self.settings.fail_at == Some(count - 1) {
            return Err(BackendError::new(
                "backend_unavailable",
                "mock renderer crashed",
            ));
        }
        std::thread::sleep(self.settings.delay);
        let (width, height) = (output.width as usize, output.height as usize);
        let rgba: Vec<u8> = if self.settings.noise {
            // Incompressible content: the encoded size is far above any estimate.
            let mut state = 0x9E37_79B9_7F4A_7C15u64 ^ frame as u64;
            (0..width * height * 4)
                .map(|index| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    if index % 4 == 3 {
                        255
                    } else {
                        (state >> 24) as u8
                    }
                })
                .collect()
        } else {
            let [r, g, b] = video_frame_color(frame);
            (0..width * height).flat_map(|_| [r, g, b, 255]).collect()
        };
        let path = output.png.expect("export asks for a PNG");
        fs::write(&path, encode_png(output.width, output.height, &rgba))
            .map_err(|error| BackendError::new("io_error", error.to_string()))?;
        self.rendered.fetch_add(1, Ordering::SeqCst);
        Ok(FrameResult {
            id: 0,
            revision: 0,
            width: output.width,
            height: output.height,
            hash: String::new(),
            mean: [0.0; 4],
            probes: Vec::new(),
            png: Some(path),
            render_ms: 0.0,
            total_ms: 0.0,
            detail: Default::default(),
        })
    }

    fn capabilities(&mut self) -> Result<Capabilities, BackendError> {
        Ok(Capabilities {
            backend: "mock".into(),
            bridge_version: 1,
            runtime_version: "test".into(),
            renderer: "mock".into(),
            layers: Vec::new(),
            formats: Value::Null,
            raw: json!({ "capabilities": { "max_size": self.settings.max_size } }),
        })
    }

    fn summary(&self) -> Value {
        json!({})
    }

    fn reset_project(&mut self) {}
}
