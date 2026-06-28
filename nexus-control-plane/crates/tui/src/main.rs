mod app;
mod emitter;
mod input;
mod views;

use std::sync::Arc;

use nexus_core::db;
use nexus_core::pty::PtyManager;

use app::App;
use emitter::TuiEmitter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Init database
    let db = db::init_db().map_err(|e| format!("DB init failed: {}", e))?;

    // 2. Create TuiEmitter with refresh channel
    let (refresh_tx, refresh_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let emitter: Arc<dyn nexus_core::events::EventEmitter> =
        Arc::new(TuiEmitter::new(refresh_tx));

    // 3. Get tokio runtime handle
    let runtime_handle = tokio::runtime::Handle::current();

    // 4. Create PtyManager
    let pty = Arc::new(PtyManager::new(emitter, runtime_handle));

    // 5. Setup terminal
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
    )?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;

    // 6. Build App, load data, run
    let mut app = App::new(db, pty.clone());
    app.load_board_data().map_err(|e| format!("Failed to load board: {}", e))?;

    let result = app.run(&mut terminal, refresh_rx).await;

    // 7. Teardown: kill PTYs, restore terminal
    pty.kill_all();
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::LeaveAlternateScreen,
    )?;
    terminal.show_cursor()?;

    result
}
