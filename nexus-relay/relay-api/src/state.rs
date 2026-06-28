use relay_db::DbPool;
use tokio::sync::mpsc::Sender;

#[derive(Clone)]
pub struct AppState {
    pub db: DbPool,
    pub notify_tx: Option<Sender<relay_notify::types::NotifyEvent>>,
    pub blob_repo: Option<relay_archive::git::GitRepo>,
}
