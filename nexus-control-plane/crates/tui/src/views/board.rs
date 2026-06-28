use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;

fn hex_to_color(hex: &str) -> Color {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return Color::White;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(255);
    Color::Rgb(r, g, b)
}

fn visible_lane_count(terminal_width: u16, total_lanes: usize) -> usize {
    let min_lane_width: u16 = 20;
    let max_visible = (terminal_width / min_lane_width) as usize;
    max_visible.min(total_lanes).max(1)
}

fn compute_lane_offset(selected: usize, visible: usize, total: usize) -> usize {
    if selected >= visible {
        (selected - visible + 1).min(total.saturating_sub(visible))
    } else {
        0
    }
}

pub fn render_board(app: &App, frame: &mut Frame, area: Rect) {
    if app.lanes.is_empty() {
        let msg = Paragraph::new("No lanes configured")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(msg, area);
        return;
    }

    let visible = visible_lane_count(area.width, app.lanes.len());
    let lane_offset = compute_lane_offset(app.selected_lane, visible, app.lanes.len());

    let constraints: Vec<Constraint> = (0..visible)
        .map(|_| Constraint::Ratio(1, visible as u32))
        .collect();

    let lane_areas = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (i, lane_area) in lane_areas.iter().enumerate() {
        let lane_index = lane_offset + i;
        if lane_index >= app.lanes.len() {
            break;
        }
        let lane = &app.lanes[lane_index];
        let is_selected_lane = lane_index == app.selected_lane;
        let lane_color = hex_to_color(&lane.color);
        let lane_cards = app.cards_for_lane(lane_index);

        let title = format!(" {} {} ({}) ", lane.emoji, lane.name, lane_cards.len());
        let border_style = if is_selected_lane {
            Style::default()
                .fg(lane_color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let lane_block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(border_style);

        let inner = lane_block.inner(*lane_area);
        frame.render_widget(lane_block, *lane_area);

        render_lane_cards(app, frame, inner, &lane_cards, is_selected_lane);
    }
}

fn render_lane_cards(
    app: &App,
    frame: &mut Frame,
    area: Rect,
    cards: &[&nexus_core::types::Card],
    is_selected_lane: bool,
) {
    if cards.is_empty() {
        let empty = Paragraph::new("(empty)")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, area);
        return;
    }

    let card_height: u16 = 3;
    let max_visible = (area.height / card_height).max(1) as usize;

    let scroll_offset = if is_selected_lane && app.selected_card >= max_visible {
        app.selected_card - max_visible + 1
    } else {
        0
    };

    for (i, card) in cards.iter().enumerate().skip(scroll_offset) {
        let y = (i - scroll_offset) as u16 * card_height;
        if y + card_height > area.height {
            break;
        }

        let card_area = Rect::new(area.x, area.y + y, area.width, card_height);
        let is_selected = is_selected_lane && i == app.selected_card;
        let has_session = app.active_session_card_ids.contains(&card.id);

        render_card(frame, card_area, card, is_selected, has_session);
    }
}

fn render_card(
    frame: &mut Frame,
    area: Rect,
    card: &nexus_core::types::Card,
    is_selected: bool,
    has_session: bool,
) {
    let border_style = if is_selected {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let dot_prefix = if has_session { 2 } else { 0 }; // "● " = 2 chars
    let max_name_len = (inner.width as usize).saturating_sub(dot_prefix);
    let name = if card.name.len() > max_name_len {
        let trunc = max_name_len.saturating_sub(3);
        format!("{}...", &card.name[..trunc.min(card.name.len())])
    } else {
        card.name.clone()
    };

    let mut spans = vec![];
    if has_session {
        spans.push(Span::styled(
            "● ",
            Style::default().fg(Color::Green),
        ));
    }
    spans.push(Span::raw(name));

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, inner);
}
