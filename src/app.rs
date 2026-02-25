use crate::data::{DataType, Endianness, decode_blob, is_binary};
use crate::redis_client::{KeyInfo, RedisClient, RedisValue, StreamEntry};
use ratatui::widgets::ListState;

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum EditOperation {
    SetString,
    HSet,
    RPush,
    LSet,
    SAdd,
    ZAdd,
    XAdd,
    NewKey,
    SetTTL,
    RenameKey,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Panel {
    KeyList,
    ValueView,
    DataPlot,
}

impl Panel {
    pub fn next(&self) -> Panel {
        match self {
            Panel::KeyList => Panel::ValueView,
            Panel::ValueView => Panel::DataPlot,
            Panel::DataPlot => Panel::KeyList,
        }
    }

    pub fn prev(&self) -> Panel {
        match self {
            Panel::KeyList => Panel::DataPlot,
            Panel::ValueView => Panel::KeyList,
            Panel::DataPlot => Panel::ValueView,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputMode {
    Normal,
    Filter,
    Confirm,
    Help,
    Edit,
    PlotLimit,
}

pub const KEY_TYPES: &[&str] = &["string", "hash", "list", "set", "zset", "stream"];

pub struct App {
    pub running: bool,
    pub active_panel: Panel,
    pub input_mode: InputMode,

    // Key list state
    pub keys: Vec<String>,
    pub key_types: Vec<String>,
    pub key_list_state: ListState,
    pub filter_text: String,
    pub filter_pattern: String,

    // Value display
    pub current_key_info: Option<KeyInfo>,
    pub current_value: Option<RedisValue>,
    pub value_scroll: u16,

    // Stream state
    pub expanded_stream_entries: Vec<bool>,
    pub last_stream_id: Option<String>, // for XREAD tracking

    // Data plot
    pub data_type: DataType,
    pub endianness: Endianness,
    pub plot_data: Vec<f64>,
    pub plot_auto_limits: bool,
    pub plot_y_min: f64,
    pub plot_y_max: f64,
    pub fft_enabled: bool,
    pub fft_data: Vec<f64>,

    // Connection
    pub db: i64,
    pub db_size: i64,
    pub connected: bool,
    pub status_message: String,

    // Confirmation dialog
    pub confirm_action: Option<String>,

    // Edit state
    pub edit_operation: Option<EditOperation>,
    pub edit_fields: Vec<(String, String)>, // (label, value)
    pub edit_focus: usize,
    pub edit_key: String,          // the key being edited
    pub edit_multi_count: usize,   // how many entries submitted in this session
    pub new_key_type_idx: usize,   // index into KEY_TYPES for new key creation
}

impl App {
    pub fn new() -> Self {
        Self {
            running: true,
            active_panel: Panel::KeyList,
            input_mode: InputMode::Normal,

            keys: Vec::new(),
            key_types: Vec::new(),
            key_list_state: ListState::default(),
            filter_text: String::new(),
            filter_pattern: String::from("*"),

            current_key_info: None,
            current_value: None,
            value_scroll: 0,

            expanded_stream_entries: Vec::new(),
            last_stream_id: None,

            data_type: DataType::UInt8,
            endianness: Endianness::Little,
            plot_data: Vec::new(),
            plot_auto_limits: true,
            plot_y_min: 0.0,
            plot_y_max: 1.0,
            fft_enabled: false,
            fft_data: Vec::new(),

            db: 0,
            db_size: 0,
            connected: false,
            status_message: String::from("Connecting..."),

            confirm_action: None,

            edit_operation: None,
            edit_fields: Vec::new(),
            edit_focus: 0,
            edit_key: String::new(),
            edit_multi_count: 0,
            new_key_type_idx: 0,
        }
    }

    pub fn refresh_keys(&mut self, client: &mut RedisClient) {
        match client.scan_keys(&self.filter_pattern) {
            Ok(keys) => {
                // Get types for each key
                let mut types = Vec::with_capacity(keys.len());
                for key in &keys {
                    let t = client
                        .get_key_info(key)
                        .map(|info| info.key_type)
                        .unwrap_or_else(|_| "?".to_string());
                    types.push(t);
                }
                self.keys = keys;
                self.key_types = types;
                self.status_message = format!("Loaded {} keys", self.keys.len());

                // Preserve selection if possible
                if self.keys.is_empty() {
                    self.key_list_state.select(None);
                } else if self.key_list_state.selected().is_none() {
                    self.key_list_state.select(Some(0));
                } else if let Some(sel) = self.key_list_state.selected() {
                    if sel >= self.keys.len() {
                        self.key_list_state.select(Some(self.keys.len() - 1));
                    }
                }
            }
            Err(e) => {
                self.status_message = format!("Error scanning keys: {}", e);
            }
        }

        self.db_size = client.get_db_size().unwrap_or(0);
        self.connected = client.is_connected();
    }

    pub fn load_selected_value(&mut self, client: &mut RedisClient) {
        if let Some(idx) = self.key_list_state.selected() {
            if idx < self.keys.len() {
                let key = &self.keys[idx].clone();

                match client.get_key_info(key) {
                    Ok(info) => self.current_key_info = Some(info),
                    Err(e) => {
                        self.status_message = format!("Error getting key info: {}", e);
                        self.current_key_info = None;
                    }
                }

                match client.get_value(key) {
                    Ok(value) => {
                        // Track last stream ID for XREAD polling
                        if let RedisValue::Stream(ref entries) = value {
                            self.last_stream_id =
                                entries.last().map(|e| e.id.clone());
                        } else {
                            self.last_stream_id = None;
                        }
                        self.update_plot_data(&value);
                        self.current_value = Some(value);
                        self.value_scroll = 0;
                    }
                    Err(e) => {
                        self.status_message = format!("Error reading value: {}", e);
                        self.current_value = None;
                        self.plot_data.clear();
                        self.last_stream_id = None;
                    }
                }
            }
        }
    }

    /// Append new stream entries from XREAD into the current value.
    /// Returns true if new entries were added.
    pub fn append_stream_entries(&mut self, new_entries: Vec<crate::redis_client::StreamEntry>) -> bool {
        if new_entries.is_empty() {
            return false;
        }
        // Update last_stream_id
        if let Some(last) = new_entries.last() {
            self.last_stream_id = Some(last.id.clone());
        }
        // Append to existing stream value
        if let Some(RedisValue::Stream(ref mut entries)) = self.current_value {
            entries.extend(new_entries);
            // Recompute plot from updated stream
            let value = RedisValue::Stream(entries.clone());
            self.update_plot_data(&value);
            if self.fft_enabled {
                self.compute_fft();
            }
            true
        } else {
            false
        }
    }

    /// Reload the current value without resetting scroll.
    #[allow(dead_code)]
    pub fn refresh_selected_value(&mut self, client: &mut RedisClient) {
        if let Some(idx) = self.key_list_state.selected() {
            if idx < self.keys.len() {
                let key = self.keys[idx].clone();

                if let Ok(info) = client.get_key_info(&key) {
                    self.current_key_info = Some(info);
                }

                match client.get_value(&key) {
                    Ok(value) => {
                        if let RedisValue::Stream(ref entries) = value {
                            self.last_stream_id =
                                entries.last().map(|e| e.id.clone());
                        }
                        self.update_plot_data(&value);
                        if self.fft_enabled {
                            self.compute_fft();
                        }
                        self.current_value = Some(value);
                    }
                    Err(_) => {}
                }
            }
        }
    }

    pub fn is_viewing_stream(&self) -> bool {
        matches!(
            &self.current_key_info,
            Some(info) if info.key_type == "stream"
        )
    }

    fn update_plot_data(&mut self, value: &RedisValue) {
        self.plot_data = match value {
            RedisValue::String(bytes) => {
                decode_blob(bytes, self.data_type, self.endianness)
            }
            RedisValue::Stream(entries) => {
                // Extract _ fields from stream entries and decode
                self.expanded_stream_entries = vec![false; entries.len()];
                extract_stream_plot_data(entries, self.data_type, self.endianness)
            }
            RedisValue::List(items) => {
                // Try to parse list items as numbers or decode as blobs
                let mut data = Vec::new();
                for item in items {
                    if let Ok(s) = std::str::from_utf8(item) {
                        if let Ok(v) = s.parse::<f64>() {
                            data.push(v);
                            continue;
                        }
                    }
                    let decoded = decode_blob(item, self.data_type, self.endianness);
                    data.extend(decoded);
                }
                data
            }
            RedisValue::ZSet(pairs) => {
                // Plot scores
                pairs.iter().map(|(_, score)| *score).collect()
            }
            RedisValue::Hash(pairs) => {
                // Try to parse hash values as numbers
                let mut data = Vec::new();
                for (_, val) in pairs {
                    if let Ok(s) = std::str::from_utf8(val) {
                        if let Ok(v) = s.parse::<f64>() {
                            data.push(v);
                            continue;
                        }
                    }
                    let decoded = decode_blob(val, self.data_type, self.endianness);
                    data.extend(decoded);
                }
                data
            }
            _ => Vec::new(),
        };
    }

    pub fn recompute_plot(&mut self) {
        if let Some(value) = &self.current_value.clone() {
            self.update_plot_data(value);
        }
        if self.fft_enabled {
            self.compute_fft();
        }
    }

    pub fn toggle_fft(&mut self) {
        self.fft_enabled = !self.fft_enabled;
        if self.fft_enabled {
            self.compute_fft();
        } else {
            self.fft_data.clear();
        }
    }

    pub fn compute_fft(&mut self) {
        if self.plot_data.is_empty() {
            self.fft_data.clear();
            return;
        }
        self.fft_data = compute_fft_magnitude(&self.plot_data);
    }

    pub fn set_auto_limits(&mut self) {
        self.plot_auto_limits = true;
    }

    pub fn start_set_plot_limits(&mut self) {
        // Pre-fill with current auto-computed limits
        let (y_min, y_max) = self.auto_y_bounds();
        self.edit_fields = vec![
            ("Y Min".to_string(), format!("{:.2}", y_min)),
            ("Y Max".to_string(), format!("{:.2}", y_max)),
        ];
        self.edit_focus = 0;
        self.input_mode = InputMode::PlotLimit;
    }

    pub fn apply_plot_limits(&mut self) -> Result<(), String> {
        let y_min: f64 = self.edit_fields[0]
            .1
            .trim()
            .parse()
            .map_err(|_| "Invalid Y Min".to_string())?;
        let y_max: f64 = self.edit_fields[1]
            .1
            .trim()
            .parse()
            .map_err(|_| "Invalid Y Max".to_string())?;
        if y_min >= y_max {
            return Err("Y Min must be less than Y Max".to_string());
        }
        self.plot_y_min = y_min;
        self.plot_y_max = y_max;
        self.plot_auto_limits = false;
        Ok(())
    }

    /// Compute auto y-bounds from current plot data
    pub fn auto_y_bounds(&self) -> (f64, f64) {
        if self.plot_data.is_empty() {
            return (0.0, 1.0);
        }
        let y_min = self.plot_data.iter().cloned().fold(f64::INFINITY, f64::min);
        let y_max = self.plot_data.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let range = y_max - y_min;
        let pad = if range == 0.0 { 1.0 } else { range * 0.1 };
        (y_min - pad, y_max + pad)
    }

    pub fn select_next_key(&mut self) {
        if self.keys.is_empty() {
            return;
        }
        let i = match self.key_list_state.selected() {
            Some(i) => {
                if i >= self.keys.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.key_list_state.select(Some(i));
    }

    pub fn select_prev_key(&mut self) {
        if self.keys.is_empty() {
            return;
        }
        let i = match self.key_list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.keys.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.key_list_state.select(Some(i));
    }

    pub fn scroll_value_down(&mut self) {
        self.value_scroll = self.value_scroll.saturating_add(1);
    }

    pub fn scroll_value_up(&mut self) {
        self.value_scroll = self.value_scroll.saturating_sub(1);
    }

    pub fn selected_key_name(&self) -> Option<&str> {
        self.key_list_state
            .selected()
            .and_then(|i| self.keys.get(i).map(|s| s.as_str()))
    }

    pub fn apply_filter(&mut self) {
        if self.filter_text.is_empty() {
            self.filter_pattern = "*".to_string();
        } else {
            self.filter_pattern = format!("*{}*", self.filter_text);
        }
    }

    /// Format the current value for display
    pub fn format_value(&self) -> Vec<String> {
        match &self.current_value {
            None => vec!["(no value loaded)".to_string()],
            Some(RedisValue::String(bytes)) => {
                if is_binary(bytes) {
                    crate::data::format_hex(bytes)
                        .lines()
                        .map(|l| l.to_string())
                        .collect()
                } else {
                    let s = String::from_utf8_lossy(bytes).to_string();
                    // Try to pretty-print JSON
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&s) {
                        if let Ok(pretty) = serde_json::to_string_pretty(&json) {
                            return pretty.lines().map(|l| l.to_string()).collect();
                        }
                    }
                    s.lines().map(|l| l.to_string()).collect()
                }
            }
            Some(RedisValue::List(items)) => {
                items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        let s = String::from_utf8_lossy(item);
                        format!("[{}] {}", i, s)
                    })
                    .collect()
            }
            Some(RedisValue::Set(items)) => {
                items
                    .iter()
                    .map(|item| {
                        let s = String::from_utf8_lossy(item);
                        format!("- {}", s)
                    })
                    .collect()
            }
            Some(RedisValue::ZSet(pairs)) => {
                pairs
                    .iter()
                    .map(|(member, score)| {
                        let s = String::from_utf8_lossy(member);
                        format!("{:.4}  {}", score, s)
                    })
                    .collect()
            }
            Some(RedisValue::Hash(pairs)) => {
                pairs
                    .iter()
                    .map(|(field, val)| {
                        let s = String::from_utf8_lossy(val);
                        format!("{}  =>  {}", field, s)
                    })
                    .collect()
            }
            Some(RedisValue::Stream(entries)) => format_stream_entries(entries),
            Some(RedisValue::Unknown(msg)) => vec![msg.clone()],
        }
    }

    // ─── Edit operations ─────────────────────────────────────

    pub fn start_edit(&mut self) {
        let key_type = self
            .current_key_info
            .as_ref()
            .map(|i| i.key_type.as_str())
            .unwrap_or("");
        let key = match self.selected_key_name() {
            Some(k) => k.to_string(),
            None => return,
        };
        self.edit_key = key.clone();
        self.edit_focus = 0;
        self.edit_multi_count = 0;

        match key_type {
            "string" => {
                let current = match &self.current_value {
                    Some(RedisValue::String(b)) => String::from_utf8_lossy(b).to_string(),
                    _ => String::new(),
                };
                self.edit_operation = Some(EditOperation::SetString);
                self.edit_fields = vec![("Value".to_string(), current)];
            }
            "hash" => {
                self.edit_operation = Some(EditOperation::HSet);
                self.edit_fields = vec![
                    ("Field".to_string(), String::new()),
                    ("Value".to_string(), String::new()),
                ];
            }
            "list" => {
                self.edit_operation = Some(EditOperation::RPush);
                self.edit_fields = vec![("Value (appended)".to_string(), String::new())];
            }
            "set" => {
                self.edit_operation = Some(EditOperation::SAdd);
                self.edit_fields = vec![("Member".to_string(), String::new())];
            }
            "zset" => {
                self.edit_operation = Some(EditOperation::ZAdd);
                self.edit_fields = vec![
                    ("Score".to_string(), "0".to_string()),
                    ("Member".to_string(), String::new()),
                ];
            }
            "stream" => {
                self.edit_operation = Some(EditOperation::XAdd);
                self.edit_fields = vec![
                    ("Field".to_string(), String::new()),
                    ("Value".to_string(), String::new()),
                ];
            }
            _ => return,
        }
        self.input_mode = InputMode::Edit;
    }

    pub fn start_set_ttl(&mut self) {
        let key = match self.selected_key_name() {
            Some(k) => k.to_string(),
            None => return,
        };
        let current_ttl = self
            .current_key_info
            .as_ref()
            .map(|i| {
                if i.ttl < 0 {
                    String::new()
                } else {
                    i.ttl.to_string()
                }
            })
            .unwrap_or_default();
        self.edit_key = key;
        self.edit_operation = Some(EditOperation::SetTTL);
        self.edit_fields = vec![("TTL (seconds, empty=persist)".to_string(), current_ttl)];
        self.edit_focus = 0;
        self.input_mode = InputMode::Edit;
    }

    pub fn start_rename(&mut self) {
        let key = match self.selected_key_name() {
            Some(k) => k.to_string(),
            None => return,
        };
        self.edit_operation = Some(EditOperation::RenameKey);
        self.edit_fields = vec![("New name".to_string(), key.clone())];
        self.edit_key = key;
        self.edit_focus = 0;
        self.input_mode = InputMode::Edit;
    }

    pub fn start_new_key(&mut self) {
        self.edit_operation = Some(EditOperation::NewKey);
        self.new_key_type_idx = 0;
        self.edit_multi_count = 0;
        self.edit_fields = vec![
            ("Key".to_string(), String::new()),
            ("Value".to_string(), String::new()),
        ];
        self.edit_key.clear();
        self.edit_focus = 0;
        self.input_mode = InputMode::Edit;
    }

    pub fn execute_edit(&mut self, client: &mut RedisClient) -> Result<(), String> {
        let op = match &self.edit_operation {
            Some(op) => op.clone(),
            None => return Err("No operation".to_string()),
        };

        let result = match op {
            EditOperation::SetString => {
                let value = &self.edit_fields[0].1;
                client
                    .set_string(&self.edit_key, value)
                    .map_err(|e| e.to_string())
            }
            EditOperation::HSet => {
                let field = &self.edit_fields[0].1;
                let value = &self.edit_fields[1].1;
                if field.is_empty() {
                    return Err("Field name is required".to_string());
                }
                client
                    .hset(&self.edit_key, field, value)
                    .map_err(|e| e.to_string())
            }
            EditOperation::RPush => {
                let value = &self.edit_fields[0].1;
                client
                    .rpush(&self.edit_key, value)
                    .map_err(|e| e.to_string())
            }
            EditOperation::LSet => {
                let index: i64 = self.edit_fields[0]
                    .1
                    .parse()
                    .map_err(|_| "Invalid index".to_string())?;
                let value = &self.edit_fields[1].1;
                client
                    .lset(&self.edit_key, index, value)
                    .map_err(|e| e.to_string())
            }
            EditOperation::SAdd => {
                let member = &self.edit_fields[0].1;
                client
                    .sadd(&self.edit_key, member)
                    .map_err(|e| e.to_string())
            }
            EditOperation::ZAdd => {
                let score: f64 = self.edit_fields[0]
                    .1
                    .parse()
                    .map_err(|_| "Invalid score (must be a number)".to_string())?;
                let member = &self.edit_fields[1].1;
                client
                    .zadd(&self.edit_key, score, member)
                    .map_err(|e| e.to_string())
            }
            EditOperation::XAdd => {
                let field = &self.edit_fields[0].1;
                let value = &self.edit_fields[1].1;
                if field.is_empty() {
                    return Err("Field name is required".to_string());
                }
                client
                    .xadd(&self.edit_key, field, value)
                    .map_err(|e| e.to_string())
            }
            EditOperation::SetTTL => {
                let ttl_str = self.edit_fields[0].1.trim().to_string();
                let ttl = if ttl_str.is_empty() {
                    -1
                } else {
                    ttl_str
                        .parse::<i64>()
                        .map_err(|_| "Invalid TTL (must be a number)".to_string())?
                };
                client
                    .set_ttl(&self.edit_key, ttl)
                    .map_err(|e| e.to_string())
            }
            EditOperation::RenameKey => {
                let new_name = &self.edit_fields[0].1;
                if new_name.is_empty() {
                    return Err("Key name is required".to_string());
                }
                client
                    .rename_key(&self.edit_key, new_name)
                    .map_err(|e| e.to_string())
            }
            EditOperation::NewKey => {
                let key = &self.edit_fields[0].1;
                let value = &self.edit_fields[1].1;
                if key.is_empty() {
                    return Err("Key name is required".to_string());
                }
                let key_type = KEY_TYPES[self.new_key_type_idx];
                match key_type {
                    "string" => client.set_string(key, value).map_err(|e| e.to_string()),
                    "hash" => client.hset(key, "field", value).map_err(|e| e.to_string()),
                    "list" => client.rpush(key, value).map_err(|e| e.to_string()),
                    "set" => client.sadd(key, value).map_err(|e| e.to_string()),
                    "zset" => client.zadd(key, 0.0, value).map_err(|e| e.to_string()),
                    "stream" => client.xadd(key, "data", value).map_err(|e| e.to_string()),
                    _ => Err(format!("Unknown type: {}", key_type)),
                }
            }
        };

        result
    }

    pub fn cancel_edit(&mut self) {
        self.edit_operation = None;
        self.edit_fields.clear();
        self.edit_focus = 0;
        self.input_mode = InputMode::Normal;
    }

    pub fn edit_next_field(&mut self) {
        if !self.edit_fields.is_empty() {
            self.edit_focus = (self.edit_focus + 1) % self.edit_fields.len();
        }
    }

    pub fn edit_op_label(&self) -> &str {
        match &self.edit_operation {
            Some(EditOperation::SetString) => "SET",
            Some(EditOperation::HSet) => "HSET",
            Some(EditOperation::RPush) => "RPUSH",
            Some(EditOperation::LSet) => "LSET",
            Some(EditOperation::SAdd) => "SADD",
            Some(EditOperation::ZAdd) => "ZADD",
            Some(EditOperation::XAdd) => "XADD",
            Some(EditOperation::SetTTL) => "EXPIRE",
            Some(EditOperation::RenameKey) => "RENAME",
            Some(EditOperation::NewKey) => "NEW KEY",
            None => "",
        }
    }

    /// Returns true if the current edit operation supports adding multiple entries
    pub fn is_multi_entry_edit(&self) -> bool {
        matches!(
            &self.edit_operation,
            Some(EditOperation::HSet)
                | Some(EditOperation::RPush)
                | Some(EditOperation::SAdd)
                | Some(EditOperation::ZAdd)
                | Some(EditOperation::XAdd)
        )
    }

    /// Reset input fields for the next entry (keep labels, clear values)
    pub fn reset_edit_fields_for_next(&mut self) {
        for (_label, value) in &mut self.edit_fields {
            value.clear();
        }
        self.edit_focus = 0;
        self.edit_multi_count += 1;
    }
}

fn format_stream_entries(entries: &[StreamEntry]) -> Vec<String> {
    let mut lines = Vec::new();
    for entry in entries.iter().rev() {
        lines.push(format!("--- {} ---", entry.id));
        for (fname, fval) in &entry.fields {
            if fname.starts_with('_') && is_binary(fval) {
                // Binary data field - show hex summary
                let hex: String = fval
                    .iter()
                    .take(32)
                    .map(|b| format!("{:02x}", b))
                    .collect::<Vec<_>>()
                    .join(" ");
                let suffix = if fval.len() > 32 { "..." } else { "" };
                lines.push(format!(
                    "  {} [blob, {} bytes]: {}{}",
                    fname,
                    fval.len(),
                    hex,
                    suffix
                ));
            } else {
                let s = String::from_utf8_lossy(fval);
                lines.push(format!("  {}: {}", fname, s));
            }
        }
    }
    lines
}

fn extract_stream_plot_data(
    entries: &[StreamEntry],
    data_type: DataType,
    endianness: Endianness,
) -> Vec<f64> {
    let mut all_data = Vec::new();
    for entry in entries {
        for (fname, fval) in &entry.fields {
            if fname.starts_with('_') {
                let decoded = decode_blob(fval, data_type, endianness);
                all_data.extend(decoded);
            }
        }
    }
    all_data
}

/// Compute FFT magnitude spectrum using a naive DFT (no external crate needed).
/// Returns magnitudes for the first N/2 frequency bins (DC to Nyquist).
fn compute_fft_magnitude(data: &[f64]) -> Vec<f64> {
    let n = data.len();
    if n == 0 {
        return Vec::new();
    }

    // Remove DC offset (mean) for better FFT visualization
    let mean = data.iter().sum::<f64>() / n as f64;

    let half = n / 2;
    let mut magnitudes = Vec::with_capacity(half);

    for k in 0..half {
        let mut re = 0.0;
        let mut im = 0.0;
        for (i, &val) in data.iter().enumerate() {
            let angle = -2.0 * std::f64::consts::PI * (k as f64) * (i as f64) / (n as f64);
            re += (val - mean) * angle.cos();
            im += (val - mean) * angle.sin();
        }
        let mag = (re * re + im * im).sqrt() / (n as f64);
        magnitudes.push(mag);
    }

    magnitudes
}
