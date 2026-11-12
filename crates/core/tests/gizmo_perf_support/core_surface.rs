//! The core as a surface of the journey: the real JSON executor on a real `EditorState`, no window.
#![allow(dead_code)]

use serde_json::{Value, json};
use tessel_editor_core::scene_plan::probe;
use tessel_editor_core::{EditorState, headless};

use super::{Reply, Surface};

pub struct CoreSurface {
    pub state: EditorState,
}

impl CoreSurface {
    pub fn new() -> Self {
        Self {
            state: EditorState::demo(),
        }
    }
}

impl Surface for CoreSurface {
    fn name(&self) -> &'static str {
        "core"
    }

    fn call(&mut self, request: &Value) -> Reply {
        match headless::execute(&mut self.state, request) {
            Ok(value) => Reply { ok: true, value },
            Err(error) => Reply {
                ok: false,
                value: json!({"code": error.code, "message": error.message, "details": error.details}),
            },
        }
    }

    fn layouts(&self) -> Option<u64> {
        Some(probe::layouts_built())
    }
}
