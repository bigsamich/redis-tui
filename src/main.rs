mod app;
mod data;
mod redis_client;
mod ui;

use anyhow::{Context, Result};
use app::{App, InputMode, Panel};
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::prelude::*;
use redis_client::{RedisClient, StreamEntry};
use std::io;
use std::sync::mpsc;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "redis-tui", about = "A Redis TUI client inspired by Redis Insight")]
struct Args {
    /// Redis host
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Redis port
    #[arg(short, long, default_value_t = 6379)]
    port: u16,

    /// Redis password
    #[arg(long)]
    password: Option<String>,

    /// Redis database number
    #[arg(short, long, default_value_t = 0)]
    db: u16,

    /// Full Redis URL (overrides host/port/password/db)
    #[arg(short, long)]
    url: Option<String>,
}

impl Args {
    fn redis_url(&self) -> String {
        if let Some(url) = &self.url {
            return url.clone();
        }
        let auth = match &self.password {
            Some(pw) => format!(":{}@", pw),
            None => String::new(),
        };
        format!(
            "redis://{}{}:{}/{}",
            auth, self.host, self.port, self.db
        )
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let url = args.redis_url();

    // Connect to Redis
    let mut client = RedisClient::connect(&url)
        .with_context(|| format!("Failed to connect to Redis at {}", url))?;

    // Set up terminal
    enable_raw_mode().context("Failed to enable raw mode")?;
    io::stdout()
        .execute(EnterAlternateScreen)
        .context("Failed to enter alternate screen")?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).context("Failed to create terminal")?;

    // Run app
    let result = run_app(&mut terminal, &mut client, &url);

    // Restore terminal
    disable_raw_mode().context("Failed to disable raw mode")?;
    io::stdout()
        .execute(LeaveAlternateScreen)
        .context("Failed to leave alternate screen")?;
    terminal.show_cursor().context("Failed to show cursor")?;

    result
}

/// State for managing the background XREAD thread
#[allow(dead_code)]
struct StreamListener {
    rx: mpsc::Receiver<Vec<StreamEntry>>,
    stop_flag: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
    /// The key this listener was started for
    watching_key: String,
}

impl StreamListener {
    fn start(url: &str, key: &str, last_id: &str, db: i64) -> Option<Self> {
        let mut client = RedisClient::connect(url).ok()?;
        if db != 0 {
            client.select_db(db).ok()?;
        }
        let (tx, rx) = mpsc::channel();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop = stop_flag.clone();
        let watching_key = key.to_string();
        let watching_id = last_id.to_string();
        let thread_key = watching_key.clone();
        let mut lid = watching_id.clone();

        let handle = std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                // Block up to 1s so we can check the stop flag periodically
                match client.xread_blocking(&thread_key, &lid, 1000) {
                    Ok(entries) if !entries.is_empty() => {
                        if let Some(last) = entries.last() {
                            lid = last.id.clone();
                        }
                        if tx.send(entries).is_err() {
                            break; // receiver dropped
                        }
                    }
                    Ok(_) => {} // timeout, no data
                    Err(_) => {
                        // Connection error, back off briefly
                        std::thread::sleep(Duration::from_millis(500));
                    }
                }
            }
        });

        Some(Self {
            rx,
            stop_flag,
            handle: Some(handle),
            watching_key,
        })
    }

    fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for StreamListener {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    client: &mut RedisClient,
    redis_url: &str,
) -> Result<()> {
    let mut app = App::new();
    app.db = client.db;

    // Initial key load
    app.refresh_keys(client);
    app.connected = client.is_connected();

    let mut stream_listener: Option<StreamListener> = None;

    loop {
        terminal.draw(|frame| ui::draw(frame, &mut app))?;

        // Poll for events with short timeout
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                // Stop stream listener on any navigation away
                let prev_key = app.selected_key_name().map(|s| s.to_string());

                match app.input_mode {
                    InputMode::Filter => handle_filter_input(&mut app, key.code),
                    InputMode::Confirm => handle_confirm_input(&mut app, client, key.code),
                    InputMode::Help => {
                        app.input_mode = InputMode::Normal;
                    }
                    InputMode::Edit => {
                        handle_edit_input(&mut app, client, key.code, key.modifiers)
                    }
                    InputMode::PlotLimit => {
                        handle_plot_limit_input(&mut app, key.code)
                    }
                    InputMode::Normal => {
                        handle_normal_input(&mut app, client, key.code, key.modifiers);

                        // Toggle stream listener with 'p'
                        if key.code == KeyCode::Char('p') && app.is_viewing_stream() {
                            if stream_listener.is_some() {
                                // Stop
                                if let Some(mut sl) = stream_listener.take() {
                                    sl.stop();
                                }
                                app.status_message = "Stream: stopped".to_string();
                            } else {
                                // Start
                                if let (Some(k), Some(lid)) = (
                                    app.selected_key_name().map(|s| s.to_string()),
                                    app.last_stream_id.clone(),
                                ) {
                                    stream_listener =
                                        StreamListener::start(redis_url, &k, &lid, app.db);
                                    if stream_listener.is_some() {
                                        app.status_message =
                                            format!("Stream: listening on '{}'", k);
                                    }
                                }
                            }
                        }

                        // Stop listener if user navigated to a different key
                        let new_key = app.selected_key_name().map(|s| s.to_string());
                        if prev_key != new_key {
                            if let Some(mut sl) = stream_listener.take() {
                                sl.stop();
                            }
                        }
                    }
                }
            }
        }

        // Drain any new stream entries from the background listener
        if let Some(ref listener) = stream_listener {
            let mut total_new = 0;
            while let Ok(entries) = listener.rx.try_recv() {
                total_new += entries.len();
                app.append_stream_entries(entries);
            }
            if total_new > 0 {
                app.status_message = format!("Stream: +{} entries (live)", total_new);
            }
        }

        if !app.running {
            drop(stream_listener);
            return Ok(());
        }
    }
}

fn handle_normal_input(
    app: &mut App,
    client: &mut RedisClient,
    code: KeyCode,
    modifiers: KeyModifiers,
) {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => {
            app.running = false;
        }
        KeyCode::Char('?') => {
            app.input_mode = InputMode::Help;
        }
        KeyCode::Tab => {
            if modifiers.contains(KeyModifiers::SHIFT) {
                app.active_panel = app.active_panel.prev();
            } else {
                app.active_panel = app.active_panel.next();
            }
        }
        KeyCode::BackTab => {
            app.active_panel = app.active_panel.prev();
        }

        // Key list navigation
        KeyCode::Up | KeyCode::Char('k') if app.active_panel == Panel::KeyList => {
            app.select_prev_key();
        }
        KeyCode::Down | KeyCode::Char('j') if app.active_panel == Panel::KeyList => {
            app.select_next_key();
        }
        KeyCode::Enter if app.active_panel == Panel::KeyList => {
            app.load_selected_value(client);
        }

        // Value view scrolling
        KeyCode::Up | KeyCode::Char('k') if app.active_panel == Panel::ValueView => {
            app.scroll_value_up();
        }
        KeyCode::Down | KeyCode::Char('j') if app.active_panel == Panel::ValueView => {
            app.scroll_value_down();
        }

        // Data plot controls
        KeyCode::Char('t') if app.active_panel == Panel::DataPlot => {
            if modifiers.contains(KeyModifiers::SHIFT) {
                app.data_type = app.data_type.prev();
            } else {
                app.data_type = app.data_type.next();
            }
            app.recompute_plot();
        }
        KeyCode::Char('T') => {
            app.data_type = app.data_type.prev();
            app.recompute_plot();
        }
        KeyCode::Char('e') => {
            app.endianness = app.endianness.toggle();
            app.recompute_plot();
        }
        KeyCode::Char('a') => {
            app.set_auto_limits();
            app.status_message = "Plot: auto limits".to_string();
        }
        KeyCode::Char('l') => {
            app.start_set_plot_limits();
        }
        KeyCode::Char('f') => {
            app.toggle_fft();
            let state = if app.fft_enabled { "ON" } else { "OFF" };
            app.status_message = format!("FFT: {}", state);
        }

        // Global data type and endianness (work from any panel)
        KeyCode::Char('t') if app.active_panel != Panel::DataPlot => {
            app.data_type = app.data_type.next();
            app.recompute_plot();
        }

        // Actions
        KeyCode::Char('/') => {
            app.input_mode = InputMode::Filter;
            app.filter_text.clear();
        }
        KeyCode::Char('r') => {
            app.refresh_keys(client);
            app.status_message = "Refreshed".to_string();
        }
        KeyCode::Char('s') => {
            if app.current_key_info.is_some() {
                app.start_edit();
            }
        }
        KeyCode::Char('n') => {
            app.start_new_key();
        }
        KeyCode::Char('x') => {
            if app.current_key_info.is_some() {
                app.start_set_ttl();
            }
        }
        KeyCode::Char('R') => {
            if app.current_key_info.is_some() {
                app.start_rename();
            }
        }
        KeyCode::Char('d') => {
            if let Some(key) = app.selected_key_name() {
                app.confirm_action = Some(format!("Delete key '{}'", key));
                app.input_mode = InputMode::Confirm;
            }
        }

        // Database selection
        KeyCode::Char(c) if c.is_ascii_digit() => {
            let db = c.to_digit(10).unwrap() as i64;
            if let Err(e) = client.select_db(db) {
                app.status_message = format!("Error: {}", e);
            } else {
                app.db = db;
                app.refresh_keys(client);
                app.status_message = format!("Switched to DB {}", db);
            }
        }

        _ => {}
    }
}

fn handle_filter_input(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Enter => {
            app.apply_filter();
            app.input_mode = InputMode::Normal;
            app.status_message = format!("Filter: {}", app.filter_pattern);
        }
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
        }
        KeyCode::Backspace => {
            app.filter_text.pop();
        }
        KeyCode::Char(c) => {
            app.filter_text.push(c);
        }
        _ => {}
    }
}

fn handle_confirm_input(app: &mut App, client: &mut RedisClient, code: KeyCode) {
    match code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            // Execute the confirmed action
            if app.confirm_action.is_some() {
                if let Some(key) = app.selected_key_name().map(|s| s.to_string()) {
                    match client.delete_key(&key) {
                        Ok(_) => {
                            app.status_message = format!("Deleted '{}'", key);
                            app.current_value = None;
                            app.current_key_info = None;
                            app.plot_data.clear();
                            app.refresh_keys(client);
                        }
                        Err(e) => {
                            app.status_message = format!("Error deleting: {}", e);
                        }
                    }
                }
            }
            app.confirm_action = None;
            app.input_mode = InputMode::Normal;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.confirm_action = None;
            app.input_mode = InputMode::Normal;
        }
        _ => {}
    }
}

fn handle_edit_input(
    app: &mut App,
    client: &mut RedisClient,
    code: KeyCode,
    _modifiers: KeyModifiers,
) {
    let is_new_key = app.edit_operation == Some(app::EditOperation::NewKey);

    match code {
        KeyCode::Esc => {
            let had_entries = app.edit_multi_count > 0;
            app.cancel_edit();
            if had_entries {
                // Refresh after multi-entry session
                app.refresh_keys(client);
                app.load_selected_value(client);
            }
        }
        KeyCode::Tab => {
            app.edit_next_field();
        }
        KeyCode::BackTab => {
            // Reverse tab
            if !app.edit_fields.is_empty() {
                if app.edit_focus == 0 {
                    app.edit_focus = app.edit_fields.len() - 1;
                } else {
                    app.edit_focus -= 1;
                }
            }
        }
        KeyCode::Enter => {
            match app.execute_edit(client) {
                Ok(_) => {
                    let op_label = app.edit_op_label().to_string();
                    let key = app.edit_key.clone();
                    if app.is_multi_entry_edit() {
                        // Stay open for next entry, clear fields
                        app.reset_edit_fields_for_next();
                        app.status_message = format!(
                            "{} on '{}' OK ({} added so far)",
                            op_label, key, app.edit_multi_count
                        );
                    } else {
                        // Single-entry operation, close popup
                        app.cancel_edit();
                        app.status_message = format!("{} on '{}' OK", op_label, key);
                        app.refresh_keys(client);
                        app.load_selected_value(client);
                    }
                }
                Err(e) => {
                    app.status_message = format!("Error: {}", e);
                }
            }
        }
        // Left/Right to change type for new key
        KeyCode::Left if is_new_key => {
            if app.new_key_type_idx == 0 {
                app.new_key_type_idx = app::KEY_TYPES.len() - 1;
            } else {
                app.new_key_type_idx -= 1;
            }
        }
        KeyCode::Right if is_new_key => {
            app.new_key_type_idx = (app.new_key_type_idx + 1) % app::KEY_TYPES.len();
        }
        KeyCode::Backspace => {
            if let Some((_label, value)) = app.edit_fields.get_mut(app.edit_focus) {
                value.pop();
            }
        }
        KeyCode::Char(c) => {
            if let Some((_label, value)) = app.edit_fields.get_mut(app.edit_focus) {
                value.push(c);
            }
        }
        _ => {}
    }
}

fn handle_plot_limit_input(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
        }
        KeyCode::Tab => {
            app.edit_next_field();
        }
        KeyCode::BackTab => {
            if !app.edit_fields.is_empty() {
                if app.edit_focus == 0 {
                    app.edit_focus = app.edit_fields.len() - 1;
                } else {
                    app.edit_focus -= 1;
                }
            }
        }
        KeyCode::Enter => {
            match app.apply_plot_limits() {
                Ok(_) => {
                    app.status_message = format!(
                        "Plot limits: {:.2} to {:.2}",
                        app.plot_y_min, app.plot_y_max
                    );
                    app.input_mode = InputMode::Normal;
                }
                Err(e) => {
                    app.status_message = format!("Error: {}", e);
                }
            }
        }
        KeyCode::Backspace => {
            if let Some((_label, value)) = app.edit_fields.get_mut(app.edit_focus) {
                value.pop();
            }
        }
        KeyCode::Char(c) => {
            if let Some((_label, value)) = app.edit_fields.get_mut(app.edit_focus) {
                value.push(c);
            }
        }
        _ => {}
    }
}
