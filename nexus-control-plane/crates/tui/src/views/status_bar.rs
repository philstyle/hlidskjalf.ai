use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::app::App;

pub fn render_status_bar(app: &App, frame: &mut Frame, area: Rect) {
    let active_count = app.active_session_card_ids.len();
    let card_count = app.cards.len();
    let (cols, rows) = app.terminal_size;

    let left = format!(
        " Nexus Control Plane | {} cards | {} active",
        card_count, active_count
    );

    let flash = if let Some((ref msg, created)) = app.flash_message {
        if created.elapsed().as_secs() < 3 {
            format!(" | {}", msg)
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let right = format!("{}x{} ", cols, rows);

    let left_len = left.len() + flash.len();
    let right_len = right.len();
    let padding = (area.width as usize).saturating_sub(left_len + right_len);

    let bar = Line::from(vec![
        Span::styled(left, Style::default().fg(Color::Cyan)),
        Span::styled(flash, Style::default().fg(Color::Yellow)),
        Span::raw(" ".repeat(padding)),
        Span::styled(right, Style::default().fg(Color::DarkGray)),
    ]);

    let paragraph = Paragraph::new(bar).style(Style::default().bg(Color::Rgb(20, 20, 30)));
    frame.render_widget(paragraph, area);
}
