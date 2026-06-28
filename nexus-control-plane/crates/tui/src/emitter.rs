use nexus_core::events::EventEmitter;
use tokio::sync::mpsc;

pub struct TuiEmitter {
    refresh_tx: mpsc::UnboundedSender<()>,
}

impl TuiEmitter {
    pub fn new(refresh_tx: mpsc::UnboundedSender<()>) -> Self {
        Self { refresh_tx }
    }
}

impl EventEmitter for TuiEmitter {
    fn emit(&self, event: &str, _payload: serde_json::Value) {
        match event {
            "session:exit" | "session:started" | "card:created" => {
                let _ = self.refresh_tx.send(());
            }
            _ => {}
        }
    }
}
