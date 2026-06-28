use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;
use ratatui::prelude::*;
use tokio::sync::mpsc;

use nexus_core::db::DbState;
use nexus_core::pty::PtyManager;
use nexus_core::types::{Card, Lane};

use crate::input::{handle_key, AppAction};
use crate::views;

pub struct App {
    pub db: DbState,
    pub pty: Arc<PtyManager>,

    pub lanes: Vec<Lane>,
    pub cards: Vec<Card>,
    pub active_session_card_ids: HashSet<String>,

    pub selected_lane: usize,
    pub selected_card: usize,

    pub should_quit: bool,
    pub flash_message: Option<(String, Instant)>,
    pub terminal_size: (u16, u16),
}

impl App {
    pub fn new(db: DbState, pty: Arc<PtyManager>) -> Self {
        let terminal_size = crossterm::terminal::size().unwrap_or((80, 24));
        Self {
            db,
            pty,
            lanes: Vec::new(),
            cards: Vec::new(),
            active_session_card_ids: HashSet::new(),
            selected_lane: 0,
            selected_card: 0,
            should_quit: false,
            flash_message: None,
            terminal_size,
        }
    }

    pub fn load_board_data(&mut self) -> Result<(), String> {
        self.lanes = nexus_core::services::lanes::list_lanes(&self.db)?;
        self.cards = nexus_core::services::cards::list_cards(&self.db)?;

        let active = self.pty.list_active_sessions();
        self.active_session_card_ids = active.iter().map(|(cid, _, _)| cid.clone()).collect();

        // Clamp navigation indices
        if !self.lanes.is_empty() && self.selected_lane >= self.lanes.len() {
            self.selected_lane = self.lanes.len() - 1;
        }
        let lane_card_count = self.cards_for_lane(self.selected_lane).len();
        if lane_card_count > 0 && self.selected_card >= lane_card_count {
            self.selected_card = lane_card_count - 1;
        }

        Ok(())
    }

    pub fn cards_for_lane(&self, lane_index: usize) -> Vec<&Card> {
        if lane_index >= self.lanes.len() {
            return vec![];
        }
        let lane_id = &self.lanes[lane_index].id;
        self.cards.iter().filter(|c| c.lane_id == *lane_id).collect()
    }

    pub fn handle_action(&mut self, action: AppAction) {
        match action {
            AppAction::Quit => {
                self.should_quit = true;
            }
            AppAction::MoveLeft => {
                if self.selected_lane > 0 {
                    self.selected_lane -= 1;
                    self.clamp_card_index();
                }
            }
            AppAction::MoveRight => {
                if self.selected_lane + 1 < self.lanes.len() {
                    self.selected_lane += 1;
                    self.clamp_card_index();
                }
            }
            AppAction::MoveUp => {
                if self.selected_card > 0 {
                    self.selected_card -= 1;
                }
            }
            AppAction::MoveDown => {
                let count = self.cards_for_lane(self.selected_lane).len();
                if count > 0 && self.selected_card + 1 < count {
                    self.selected_card += 1;
                }
            }
            AppAction::Select => {
                self.flash_message = Some((
                    "Terminal view coming in Phase 4".to_string(),
                    Instant::now(),
                ));
            }
            AppAction::NewCard => {
                self.flash_message = Some((
                    "Card creation coming in Phase 5".to_string(),
                    Instant::now(),
                ));
            }
            AppAction::None => {}
        }
    }

    fn clamp_card_index(&mut self) {
        let count = self.cards_for_lane(self.selected_lane).len();
        if count == 0 {
            self.selected_card = 0;
        } else if self.selected_card >= count {
            self.selected_card = count - 1;
        }
    }

    fn render(&self, frame: &mut Frame) {
        let size = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(size);

        views::board::render_board(self, frame, chunks[0]);
        views::status_bar::render_status_bar(self, frame, chunks[1]);
    }

    pub async fn run(
        &mut self,
        terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
        mut refresh_rx: mpsc::UnboundedReceiver<()>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut event_stream = EventStream::new();
        let tick_rate = Duration::from_millis(250);
        let mut tick_interval = tokio::time::interval(tick_rate);

        self.terminal_size = crossterm::terminal::size()?;
        terminal.draw(|frame| self.render(frame))?;

        loop {
            tokio::select! {
                maybe_event = event_stream.next() => {
                    if let Some(Ok(event)) = maybe_event {
                        match event {
                            Event::Key(key) => {
                                let action = handle_key(key);
                                self.handle_action(action);
                                if self.should_quit {
                                    return Ok(());
                                }
                            }
                            Event::Resize(cols, rows) => {
                                self.terminal_size = (cols, rows);
                            }
                            _ => {}
                        }
                        terminal.draw(|frame| self.render(frame))?;
                    }
                }

                _ = refresh_rx.recv() => {
                    while refresh_rx.try_recv().is_ok() {}
                    let _ = self.load_board_data();
                    terminal.draw(|frame| self.render(frame))?;
                }

                _ = tick_interval.tick() => {
                    let active = self.pty.list_active_sessions();
                    self.active_session_card_ids = active.iter()
                        .map(|(cid, _, _)| cid.clone())
                        .collect();
                    self.terminal_size = crossterm::terminal::size()
                        .unwrap_or(self.terminal_size);
                    terminal.draw(|frame| self.render(frame))?;
                }
            }
        }
    }
}
