use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub enum AppAction {
    Quit,
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    Select,
    NewCard,
    None,
}

pub fn handle_key(key: KeyEvent) -> AppAction {
    match key.code {
        KeyCode::Char('q') => AppAction::Quit,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => AppAction::Quit,
        KeyCode::Char('h') | KeyCode::Left => AppAction::MoveLeft,
        KeyCode::Char('l') | KeyCode::Right => AppAction::MoveRight,
        KeyCode::Char('k') | KeyCode::Up => AppAction::MoveUp,
        KeyCode::Char('j') | KeyCode::Down => AppAction::MoveDown,
        KeyCode::Enter => AppAction::Select,
        KeyCode::Char('n') => AppAction::NewCard,
        _ => AppAction::None,
    }
}
