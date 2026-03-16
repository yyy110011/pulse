use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::dashboard::Dashboard;
use crate::session::SessionState;

/// Draw the dashboard view.
pub fn draw(frame: &mut Frame, dashboard: &Dashboard, rt: &tokio::runtime::Handle) {
    match dashboard.focused {
        Some(idx) => draw_focused(frame, dashboard, idx, rt),
        None => draw_grid(frame, dashboard, rt),
    }
}

fn draw_grid(frame: &mut Frame, dashboard: &Dashboard, rt: &tokio::runtime::Handle) {
    let area = frame.area();

    // Layout: grid + status bar
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let grid_area = main_chunks[0];
    let status_area = main_chunks[1];

    let rows = dashboard.rows();
    let cols = dashboard.cols;

    // Create row constraints
    let row_constraints: Vec<Constraint> = (0..rows)
        .map(|_| Constraint::Ratio(1, rows as u32))
        .collect();

    let row_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(row_constraints)
        .split(grid_area);

    for row in 0..rows {
        let items_in_row = if (row + 1) * cols <= dashboard.total() {
            cols
        } else {
            dashboard.total() - row * cols
        };

        let col_constraints: Vec<Constraint> = (0..items_in_row)
            .map(|_| Constraint::Ratio(1, cols as u32))
            .collect();

        let col_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(col_constraints)
            .split(row_chunks[row]);

        for col in 0..items_in_row {
            let idx = row * cols + col;
            let is_selected = idx == dashboard.selected;
            draw_tile(frame, dashboard, idx, is_selected, col_chunks[col], rt);
        }
    }

    // Status bar
    let status = Line::from(vec![
        Span::styled(
            " ↑↓←→ Navigate  Enter Focus  q Quit",
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(Paragraph::new(status), status_area);
}

fn draw_tile(
    frame: &mut Frame,
    dashboard: &Dashboard,
    idx: usize,
    is_selected: bool,
    area: Rect,
    rt: &tokio::runtime::Handle,
) {
    let session = &dashboard.sessions[idx];

    // Block on the async lock briefly - this is fine for UI rendering
    let data = rt.block_on(session.lock());

    let (state_icon, state_color) = match &data.state {
        SessionState::Idle => ("◌", Color::DarkGray),
        SessionState::Connecting => ("◌", Color::Yellow),
        SessionState::NeedPassword => ("⚷", Color::Yellow),
        SessionState::Authenticating => ("◌", Color::Yellow),
        SessionState::Connected => ("●", Color::Green),
        SessionState::Disconnected(_) => ("●", Color::Red),
    };

    let border_color = if is_selected {
        Color::Cyan
    } else {
        match &data.state {
            SessionState::Connected => Color::Green,
            SessionState::Disconnected(_) => Color::Red,
            _ => Color::DarkGray,
        }
    };

    let border_style = if is_selected {
        Style::default().fg(border_color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(border_color)
    };

    let title = format!(
        " {state_icon} {} ",
        data.host.name,
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled(
            title,
            Style::default().fg(state_color).add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Content inside the tile
    let content_lines = match &data.state {
        SessionState::Connected => {
            let preview = data.screen_lines(inner.height as usize);
            preview
                .into_iter()
                .map(|l| Line::from(Span::styled(l, Style::default().fg(Color::White))))
                .collect::<Vec<_>>()
        }
        SessionState::NeedPassword => {
            vec![
                Line::from(Span::styled(
                    data.host.display_connection(),
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "⚷ Password required",
                    Style::default().fg(Color::Yellow),
                )),
                Line::from(Span::styled(
                    "Press Enter to focus & type password",
                    Style::default().fg(Color::DarkGray),
                )),
            ]
        }
        SessionState::Disconnected(msg) => {
            vec![
                Line::from(Span::styled(
                    data.host.display_connection(),
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    format!("✗ {msg}"),
                    Style::default().fg(Color::Red),
                )),
            ]
        }
        _ => {
            vec![
                Line::from(Span::styled(
                    data.host.display_connection(),
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    data.state.label(),
                    Style::default().fg(Color::Yellow),
                )),
            ]
        }
    };

    let paragraph = Paragraph::new(content_lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

fn draw_focused(
    frame: &mut Frame,
    dashboard: &Dashboard,
    idx: usize,
    rt: &tokio::runtime::Handle,
) {
    let area = frame.area();
    let session = &dashboard.sessions[idx];
    let data = rt.block_on(session.lock());

    // Layout: terminal + status bar
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let (state_icon, state_color) = match &data.state {
        SessionState::Connected => ("●", Color::Green),
        SessionState::NeedPassword => ("⚷", Color::Yellow),
        SessionState::Disconnected(_) => ("●", Color::Red),
        _ => ("◌", Color::Yellow),
    };

    let title = format!(
        " {state_icon} {} — {} ",
        data.host.name,
        data.host.display_connection(),
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(state_color))
        .title(Span::styled(
            title,
            Style::default().fg(state_color).add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(chunks[0]);
    frame.render_widget(block, chunks[0]);

    match &data.state {
        SessionState::Connected => {
            // Render full terminal screen from vt100
            let screen = data.screen.screen();
            let rows = inner.height as usize;
            let total_rows = screen.size().0 as usize;
            let start = if total_rows > rows { total_rows - rows } else { 0 };

            let lines: Vec<Line> = (start..total_rows)
                .map(|r| {
                    let r16 = r as u16;
                    let content = screen.contents_between(r16, 0, r16 + 1, 0);
                    Line::from(Span::styled(content, Style::default().fg(Color::White)))
                })
                .collect();

            frame.render_widget(Paragraph::new(lines), inner);
        }
        SessionState::NeedPassword => {
            let password_display = if dashboard.entering_password {
                format!("Password: {}", "*".repeat(dashboard.password_input.len()))
            } else {
                "Press any key to enter password...".to_string()
            };

            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "Authentication required",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    password_display,
                    Style::default().fg(Color::White),
                )),
            ];
            frame.render_widget(Paragraph::new(lines), inner);
        }
        SessionState::Disconnected(msg) => {
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    format!("✗ Disconnected: {msg}"),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
            ];
            frame.render_widget(Paragraph::new(lines), inner);
        }
        _ => {
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    data.state.label(),
                    Style::default().fg(Color::Yellow),
                )),
            ];
            frame.render_widget(Paragraph::new(lines), inner);
        }
    }

    // Status bar
    let status = Line::from(vec![
        Span::styled(" Esc Back  ", Style::default().fg(Color::DarkGray)),
        if data.state.is_connected() {
            Span::styled("(typing sends to remote)", Style::default().fg(Color::DarkGray))
        } else {
            Span::raw("")
        },
    ]);
    frame.render_widget(Paragraph::new(status), chunks[1]);
}
