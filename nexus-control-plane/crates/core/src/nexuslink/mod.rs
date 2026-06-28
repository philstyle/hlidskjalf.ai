mod server;
pub mod bootstrap;
pub mod pwa;
pub mod wake;

pub use server::ApiEvent;
pub use wake::AgentWake;

use crate::db::DbState;
use crate::events::EventEmitter;
use crate::github::GithubService;
use crate::pty::PtyManager;
use crate::tailscale::TailscaleService;
use std::net::SocketAddr;
use std::sync::Arc;

/// Shared state between Tauri commands and the axum server.
/// Both access the same Arcs — no duplication.
#[derive(Clone)]
pub struct NexusLinkState {
    pub db: DbState,
    pub pty: Arc<PtyManager>,
    #[allow(dead_code)]
    pub tailscale: Arc<TailscaleService>,
    pub bind_addr: SocketAddr,
    pub emitter: Arc<dyn EventEmitter>,
    pub github: Arc<GithubService>,
    /// Broadcast channel for SSE API events (board updates, session changes).
    pub api_events: tokio::sync::broadcast::Sender<server::ApiEvent>,
    /// Shared bootstrap process state.
    pub bootstrap_state: bootstrap::SharedBootstrapState,
    /// Runtime settings cache backed by the settings table.
    pub settings: crate::settings::SharedSettings,
    /// Agent Wake — polling NexusRelay and draining relay messages into idle sessions.
    pub wake: Arc<wake::AgentWake>,
    /// NexusRelay config — None if relay env vars not set.
    pub relay_config: Option<Arc<crate::relay::RelayConfig>>,
    /// Context wind-down shared state — enabled flag + configurable thresholds.
    pub winddown: crate::winddown::SharedWinddownState,
}

/// Start the NexusLink HTTP server. Logs and returns on bind failure (no crash).
pub async fn start_server(state: NexusLinkState) {
    server::run(state).await;
}
