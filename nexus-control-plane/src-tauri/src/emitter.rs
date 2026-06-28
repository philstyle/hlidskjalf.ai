use nexus_core::events::EventEmitter;
use tauri::{AppHandle, Emitter};

pub struct TauriEmitter(pub AppHandle);

impl EventEmitter for TauriEmitter {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        let _ = self.0.emit(event, payload);
    }
}
