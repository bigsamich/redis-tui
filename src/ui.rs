use crate::app::{App, EditOperation, InputMode, Panel, KEY_TYPES};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        Axis, Block, Borders, Chart, Clear, Dataset, GraphType, List, ListItem, Paragraph, Wrap,
    },
    Frame,
};

const HIGHLIGHT_COLOR: Color = Color::Cyan;
const BORDER_ACTIVE: Color = Color::Cyan;
const BORDER_INACTIVE: Color = Color::DarkGray;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let size = frame.area();

    // Main layout: title bar, body, status bar
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title bar
            Constraint::Min(5),   // body
            Constraint::Length(1), // status bar
        ])
        .split(size);

    draw_title_bar(frame, app, outer[0]);
    draw_body(frame, app, outer[1]);
    draw_status_bar(frame, app, outer[2]);

    // Draw overlays
    match app.input_mode {
        InputMode::Filter => draw_filter_popup(frame, app, size),
        InputMode::Confirm => draw_confirm_popup(frame, app, size),
        InputMode::Help => draw_help_popup(frame, size),
        InputMode::Edit => draw_edit_popup(frame, app, size),
        InputMode::PlotLimit => draw_plot_limit_popup(frame, app, size),
        InputMode::Normal => {}
    }
}

fn draw_title_bar(frame: &mut Frame, app: &App, area: Rect) {
    let url_text = app.url_display();
    let title = Line::from(vec![
        Span::styled(" Redis TUI ", Style::default().fg(Color::White).bg(Color::Blue).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(url_text, Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled("[?]Help [q]Quit", Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(title), area);
}

fn draw_body(frame: &mut Frame, app: &mut App, area: Rect) {
    // Horizontal split: key list | right panels
    let h_split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(75),
        ])
        .split(area);

    draw_key_list(frame, app, h_split[0]);

    // Right side: vertical split for value view and data plot
    let v_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(55),
            Constraint::Percentage(45),
        ])
        .split(h_split[1]);

    draw_value_view(frame, app, v_split[0]);
    draw_data_plot(frame, app, v_split[1]);
}

fn draw_key_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let border_color = if app.active_panel == Panel::KeyList {
        BORDER_ACTIVE
    } else {
        BORDER_INACTIVE
    };

    let title = format!(
        " Keys ({}) [/]Filter [r]Refresh ",
        app.keys.len()
    );

    let items: Vec<ListItem> = app
        .keys
        .iter()
        .enumerate()
        .map(|(i, key)| {
            let type_badge = if i < app.key_types.len() {
                type_badge(&app.key_types[i])
            } else {
                ("???", Color::DarkGray)
            };

            let line = Line::from(vec![
                Span::styled(
                    format!("{:<6}", type_badge.0),
                    Style::default().fg(type_badge.1),
                ),
                Span::raw(key),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .title(title),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, area, &mut app.key_list_state);
}

fn draw_value_view(frame: &mut Frame, app: &App, area: Rect) {
    let border_color = if app.active_panel == Panel::ValueView {
        BORDER_ACTIVE
    } else {
        BORDER_INACTIVE
    };

    let mut lines: Vec<Line> = Vec::new();

    // Key metadata header
    if let Some(info) = &app.current_key_info {
        lines.push(Line::from(vec![
            Span::styled("Key: ", Style::default().fg(Color::Yellow)),
            Span::styled(&info.name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Type: ", Style::default().fg(Color::Yellow)),
            Span::styled(&info.key_type, Style::default().fg(Color::Green)),
            Span::raw("  "),
            Span::styled("TTL: ", Style::default().fg(Color::Yellow)),
            Span::styled(
                if info.ttl == -1 {
                    "none".to_string()
                } else if info.ttl == -2 {
                    "expired".to_string()
                } else {
                    format!("{}s", info.ttl)
                },
                Style::default().fg(Color::White),
            ),
            Span::raw("  "),
            Span::styled("Size: ", Style::default().fg(Color::Yellow)),
            Span::styled(format_size(info.size), Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled("Enc: ", Style::default().fg(Color::Yellow)),
            Span::styled(&info.encoding, Style::default().fg(Color::White)),
        ]));
        lines.push(Line::from(Span::styled(
            "─".repeat(area.width.saturating_sub(2) as usize),
            Style::default().fg(Color::DarkGray),
        )));
    }

    // Value content
    let value_lines = app.format_value();
    for line in &value_lines {
        lines.push(Line::from(Span::raw(line)));
    }

    let title = if app.active_panel == Panel::ValueView {
        " Value [j/k]Scroll "
    } else {
        " Value "
    };

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .title(title),
        )
        .wrap(Wrap { trim: false })
        .scroll((app.value_scroll, 0));

    frame.render_widget(paragraph, area);
}

fn draw_data_plot(frame: &mut Frame, app: &App, area: Rect) {
    let border_color = if app.active_panel == Panel::DataPlot {
        BORDER_ACTIVE
    } else {
        BORDER_INACTIVE
    };

    let limits_label = if app.plot_auto_limits { "auto" } else { "manual" };
    let fft_label = if app.fft_enabled { "ON" } else { "OFF" };
    let title = format!(
        " Plot [t]{} [e]{} [a/l]{} [f]FFT:{} ",
        app.data_type, app.endianness, limits_label, fft_label
    );

    if app.plot_data.is_empty() {
        let msg = Paragraph::new("No plottable data. Select a key with binary data.")
            .style(Style::default().fg(Color::DarkGray))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_color))
                    .title(title),
            );
        frame.render_widget(msg, area);
        return;
    }

    if app.fft_enabled && !app.fft_data.is_empty() {
        // Split area: top for signal, bottom for FFT
        let plot_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        draw_signal_chart(frame, app, plot_split[0], &title, border_color);
        draw_fft_chart(frame, app, plot_split[1], border_color);
    } else {
        draw_signal_chart(frame, app, area, &title, border_color);
    }
}

fn draw_signal_chart(frame: &mut Frame, app: &App, area: Rect, title: &str, border_color: Color) {
    let data_points: Vec<(f64, f64)> = app
        .plot_data
        .iter()
        .enumerate()
        .map(|(i, v)| (i as f64, *v))
        .collect();

    let x_max = data_points.len() as f64;

    let (y_lo, y_hi) = if app.plot_auto_limits {
        app.auto_y_bounds()
    } else {
        (app.plot_y_min, app.plot_y_max)
    };

    let datasets = vec![Dataset::default()
        .name(format!("{} values", app.plot_data.len()))
        .marker(symbols::Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(Color::Cyan))
        .data(&data_points)];

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .title(title.to_string()),
        )
        .x_axis(
            Axis::default()
                .title("Index")
                .bounds([0.0, x_max])
                .labels(vec![
                    Line::from("0"),
                    Line::from(format!("{}", x_max as i64 / 2)),
                    Line::from(format!("{}", x_max as i64)),
                ]),
        )
        .y_axis(
            Axis::default()
                .title("Value")
                .bounds([y_lo, y_hi])
                .labels(vec![
                    Line::from(format!("{:.2}", y_lo)),
                    Line::from(format!("{:.2}", (y_lo + y_hi) / 2.0)),
                    Line::from(format!("{:.2}", y_hi)),
                ]),
        );

    frame.render_widget(chart, area);
}

fn draw_fft_chart(frame: &mut Frame, app: &App, area: Rect, border_color: Color) {
    let fft_points: Vec<(f64, f64)> = app
        .fft_data
        .iter()
        .enumerate()
        .map(|(i, v)| (i as f64, *v))
        .collect();

    let x_max = fft_points.len() as f64;
    let y_max = app
        .fft_data
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);
    let y_hi = if y_max <= 0.0 { 1.0 } else { y_max * 1.1 };

    let datasets = vec![Dataset::default()
        .name(format!("{} bins", app.fft_data.len()))
        .marker(symbols::Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(Color::Yellow))
        .data(&fft_points)];

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .title(" FFT Magnitude "),
        )
        .x_axis(
            Axis::default()
                .title("Bin")
                .bounds([0.0, x_max])
                .labels(vec![
                    Line::from("0"),
                    Line::from(format!("{}", x_max as i64 / 2)),
                    Line::from(format!("{}", x_max as i64)),
                ]),
        )
        .y_axis(
            Axis::default()
                .title("Magnitude")
                .bounds([0.0, y_hi])
                .labels(vec![
                    Line::from("0"),
                    Line::from(format!("{:.2}", y_hi / 2.0)),
                    Line::from(format!("{:.2}", y_hi)),
                ]),
        );

    frame.render_widget(chart, area);
}

fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let status_color = if app.connected {
        Color::Green
    } else {
        Color::Red
    };
    let status_text = if app.connected {
        "Connected"
    } else {
        "Disconnected"
    };

    let line = Line::from(vec![
        Span::raw(" "),
        Span::styled(status_text, Style::default().fg(status_color)),
        Span::raw(" | "),
        Span::raw(format!("DB: {} ", app.db)),
        Span::raw("| "),
        Span::raw(format!("Keys: {} ", app.db_size)),
        Span::raw("| "),
        Span::styled(&app.status_message, Style::default().fg(Color::DarkGray)),
    ]);

    let bar = Paragraph::new(line).style(Style::default().bg(Color::Black));
    frame.render_widget(bar, area);
}

fn draw_filter_popup(frame: &mut Frame, app: &App, area: Rect) {
    let popup_area = centered_rect(50, 3, area);
    frame.render_widget(Clear, popup_area);

    let text = format!("Filter: {}_", app.filter_text);
    let popup = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(HIGHLIGHT_COLOR))
            .title(" Filter Keys (Enter to apply, Esc to cancel) "),
    );
    frame.render_widget(popup, popup_area);
}

fn draw_confirm_popup(frame: &mut Frame, app: &App, area: Rect) {
    let popup_area = centered_rect(50, 5, area);
    frame.render_widget(Clear, popup_area);

    let msg = if let Some(action) = &app.confirm_action {
        format!("{}?\n\n[y] Yes  [n/Esc] No", action)
    } else {
        "Confirm? [y/n]".to_string()
    };

    let popup = Paragraph::new(msg).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red))
            .title(" Confirm "),
    );
    frame.render_widget(popup, popup_area);
}

fn draw_help_popup(frame: &mut Frame, area: Rect) {
    let popup_area = centered_rect(60, 20, area);
    frame.render_widget(Clear, popup_area);

    let help_text = vec![
        Line::from(Span::styled("Keyboard Shortcuts", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![Span::styled("Navigation", Style::default().fg(Color::Cyan))]),
        Line::from("  Up/Down, j/k    Navigate key list"),
        Line::from("  Enter           Select key / load value"),
        Line::from("  Tab / Shift+Tab Cycle panels"),
        Line::from(""),
        Line::from(vec![Span::styled("Actions", Style::default().fg(Color::Cyan))]),
        Line::from("  /               Filter keys"),
        Line::from("  r               Refresh keys"),
        Line::from("  s               Set/edit value"),
        Line::from("  n               New key"),
        Line::from("  d               Delete selected key"),
        Line::from("  p               Toggle stream listen"),
        Line::from("  x               Set TTL"),
        Line::from("  R               Rename key"),
        Line::from("  0-9             Select database"),
        Line::from(""),
        Line::from(vec![Span::styled("Data Plot", Style::default().fg(Color::Cyan))]),
        Line::from("  t / T           Cycle data type forward/back"),
        Line::from("  e               Toggle endianness (LE/BE)"),
        Line::from("  a               Auto-fit Y limits"),
        Line::from("  l               Set manual Y limits"),
        Line::from("  f               Toggle FFT analysis"),
        Line::from(""),
        Line::from(vec![Span::styled("General", Style::default().fg(Color::Cyan))]),
        Line::from("  ?               Toggle this help"),
        Line::from("  q / Esc         Quit (or close popup)"),
    ];

    let popup = Paragraph::new(help_text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(HIGHLIGHT_COLOR))
            .title(" Help "),
    );
    frame.render_widget(popup, popup_area);
}

fn draw_plot_limit_popup(frame: &mut Frame, app: &App, area: Rect) {
    let popup_area = centered_rect(50, 8, area);
    frame.render_widget(Clear, popup_area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        "Set Y-Axis Limits",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    for (i, (label, value)) in app.edit_fields.iter().enumerate() {
        let is_focused = i == app.edit_focus;
        let cursor = if is_focused { "_" } else { "" };
        let indicator = if is_focused { "> " } else { "  " };
        let label_style = if is_focused {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Yellow)
        };
        let input_style = if is_focused {
            Style::default().fg(Color::White).bg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(vec![
            Span::styled(indicator, Style::default().fg(Color::Cyan)),
            Span::styled(format!("{}: ", label), label_style),
            Span::styled(format!("{}{}", value, cursor), input_style),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("[Enter]", Style::default().fg(Color::Green)),
        Span::raw(" Apply  "),
        Span::styled("[Esc]", Style::default().fg(Color::Red)),
        Span::raw(" Cancel  "),
        Span::styled("[Tab]", Style::default().fg(Color::Yellow)),
        Span::raw(" Next line"),
    ]));

    let popup = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(HIGHLIGHT_COLOR))
            .title(" Plot Limits "),
    );
    frame.render_widget(popup, popup_area);
}

fn draw_edit_popup(frame: &mut Frame, app: &App, area: Rect) {
    let field_count = app.edit_fields.len();
    let is_new_key = app.edit_operation == Some(EditOperation::NewKey);
    let is_multi = app.is_multi_entry_edit();
    let extra_type = if is_new_key { 2 } else { 0 };
    let extra_count = if is_multi && app.edit_multi_count > 0 { 1 } else { 0 };
    let height = (5 + field_count * 2 + extra_type + extra_count).min(22) as u16;
    let popup_area = centered_rect(60, height, area);
    frame.render_widget(Clear, popup_area);

    let title = if is_new_key {
        " New Key ".to_string()
    } else {
        format!(" Edit: {} ", app.edit_key)
    };

    let mut lines: Vec<Line> = Vec::new();

    // Operation label + multi-entry count
    let mut op_line = vec![
        Span::styled("Operation: ", Style::default().fg(Color::Yellow)),
        Span::styled(
            app.edit_op_label(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if is_multi && app.edit_multi_count > 0 {
        op_line.push(Span::styled(
            format!("  ({} added)", app.edit_multi_count),
            Style::default().fg(Color::Cyan),
        ));
    }
    lines.push(Line::from(op_line));

    // Type selector for new key
    if is_new_key {
        let type_name = KEY_TYPES[app.new_key_type_idx];
        lines.push(Line::from(vec![
            Span::styled("Type: ", Style::default().fg(Color::Yellow)),
            Span::styled("< ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                type_name,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" >", Style::default().fg(Color::DarkGray)),
            Span::raw("  (Left/Right to change)"),
        ]));
    }

    lines.push(Line::from(""));

    // Fields
    for (i, (label, value)) in app.edit_fields.iter().enumerate() {
        let is_focused = i == app.edit_focus;
        let label_style = if is_focused {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Yellow)
        };

        let cursor = if is_focused { "_" } else { "" };
        let input_style = if is_focused {
            Style::default().fg(Color::White).bg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        };
        let indicator = if is_focused { "> " } else { "  " };
        let display_val = format!("{}{}", value, cursor);
        lines.push(Line::from(vec![
            Span::styled(indicator, Style::default().fg(Color::Cyan)),
            Span::styled(format!("{}: ", label), label_style),
            Span::styled(display_val, input_style),
        ]));
    }

    lines.push(Line::from(""));

    // Footer with context-aware instructions
    if is_multi {
        lines.push(Line::from(vec![
            Span::styled("[Enter]", Style::default().fg(Color::Green)),
            Span::raw(" Add entry  "),
            Span::styled("[Esc]", Style::default().fg(Color::Red)),
            Span::raw(" Done  "),
            Span::styled("[Tab]", Style::default().fg(Color::Yellow)),
            Span::raw(" Next line"),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("[Enter]", Style::default().fg(Color::Green)),
            Span::raw(" Submit  "),
            Span::styled("[Esc]", Style::default().fg(Color::Red)),
            Span::raw(" Cancel  "),
            Span::styled("[Tab]", Style::default().fg(Color::Yellow)),
            Span::raw(" Next line"),
        ]));
    }

    let popup = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(HIGHLIGHT_COLOR))
            .title(title),
    );
    frame.render_widget(popup, popup_area);
}

/// Create a centered rect with given percentage width and fixed height
fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((area.height.saturating_sub(height)) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn type_badge(key_type: &str) -> (&str, Color) {
    match key_type {
        "string" => ("STR", Color::Green),
        "list" => ("LIST", Color::Blue),
        "set" => ("SET", Color::Magenta),
        "zset" => ("ZSET", Color::Yellow),
        "hash" => ("HASH", Color::Red),
        "stream" => ("STRM", Color::Cyan),
        _ => ("???", Color::DarkGray),
    }
}

fn format_size(bytes: i64) -> String {
    if bytes < 0 {
        return "?".to_string();
    }
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

// App helper method for URL display
impl App {
    pub fn url_display(&self) -> String {
        format!("db:{}", self.db)
    }
}
