use crate::storage::StorageUpdate;
use copypasta::{ClipboardContext, ClipboardProvider};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, MouseEvent, MouseEventKind,
    },
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs},
    Frame, Terminal,
};
use std::collections::VecDeque;
use std::io;
use std::process::Command;
use std::time::{Duration, Instant};

#[cfg(feature = "macos")]
use mac_notification_sys::{get_bundle_identifier_or_default, send_notification, Notification};

pub struct LogEntry {
    pub level: LogLevel,
    pub timestamp: String,
    pub pid: Option<u32>,
    pub tid: Option<u32>,
    pub tag: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warning,
    Info,
    Debug,
    Verbose,
    Unknown,
}

impl LogLevel {
    fn color(&self) -> Color {
        match self {
            LogLevel::Error => Color::Red,
            LogLevel::Warning => Color::Yellow,
            LogLevel::Info => Color::Green,
            LogLevel::Debug => Color::Blue,
            LogLevel::Verbose => Color::White,
            LogLevel::Unknown => Color::Gray,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Error => "ERROR",
            LogLevel::Warning => "WARN",
            LogLevel::Info => "INFO",
            LogLevel::Debug => "DEBUG",
            LogLevel::Verbose => "VERBOSE",
            LogLevel::Unknown => "UNKNOWN",
        }
    }

    pub fn from_logcat_char(level: char) -> Self {
        match level {
            'E' => LogLevel::Error,
            'W' => LogLevel::Warning,
            'I' => LogLevel::Info,
            'D' => LogLevel::Debug,
            'V' => LogLevel::Verbose,
            _ => LogLevel::Unknown,
        }
    }

    pub fn from_storage_str(level: &str) -> Self {
        match level.to_ascii_uppercase().as_str() {
            "E" | "ERROR" => LogLevel::Error,
            "W" | "WARN" | "WARNING" => LogLevel::Warning,
            "I" | "INFO" => LogLevel::Info,
            "D" | "DEBUG" => LogLevel::Debug,
            "V" | "VERBOSE" => LogLevel::Verbose,
            _ => LogLevel::Unknown,
        }
    }
}

// Update View enum
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum View {
    Logs,
    Stats,
    Storage,
}

// Add Display implementation for View
impl std::fmt::Display for View {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            View::Logs => write!(f, "Logs"),
            View::Stats => write!(f, "Stats"),
            View::Storage => write!(f, "Storage"),
        }
    }
}

// Add application state
pub struct AppState {
    pub current_view: View,
    pub logs: VecDeque<LogEntry>,
    pub filtered_logs: Vec<usize>, // Indices into logs
    pub scroll: usize,
    pub paused: bool,
    pub search_query: String,
    pub search_mode: bool,
    pub storage_info: Option<StorageInfo>,
    pub stats: LogStats,
    pub level_filters: Vec<LogLevel>, // Enabled log levels
    pub tail_mode: bool,              // Add this field
    pub status_message: Option<(String, Instant)>, // (message, timestamp)
    pub connection_status: ConnectionStatus,
    pub notify_on_error: bool,
    #[allow(dead_code)]
    pub last_notification: Option<Instant>,
    pub wrap_mode: bool,
    pub inspector_active: bool,
}

pub struct StorageInfo {
    pub current_file: String,
    pub total_size: u64,
    pub file_count: usize,
}

pub struct LogStats {
    pub error_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
    pub debug_count: usize,
    pub verbose_count: usize,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ConnectionStatus {
    Connected,
    Disconnected,
    Error,
}

impl AppState {
    pub fn new(initial_level_filters: Option<Vec<LogLevel>>) -> Self {
        Self {
            current_view: View::Logs,
            logs: VecDeque::with_capacity(10000), // Limit memory usage
            filtered_logs: Vec::new(),
            scroll: 0,
            paused: false,
            search_query: String::new(),
            search_mode: false,
            storage_info: None,
            stats: LogStats {
                error_count: 0,
                warning_count: 0,
                info_count: 0,
                debug_count: 0,
                verbose_count: 0,
            },
            level_filters: initial_level_filters.unwrap_or_else(|| {
                vec![
                    LogLevel::Error,
                    LogLevel::Warning,
                    LogLevel::Info,
                    LogLevel::Debug,
                    LogLevel::Verbose,
                ]
            }),
            tail_mode: true, // Start with tail mode enabled
            status_message: None,
            connection_status: ConnectionStatus::Connected,
            notify_on_error: true,
            last_notification: None,
            wrap_mode: false,
            inspector_active: false,
        }
    }

    pub fn add_log(&mut self, entry: LogEntry) {
        self.add_logs(vec![entry]);
    }

    pub fn add_logs(&mut self, entries: Vec<LogEntry>) {
        if !self.paused {
            let mut needs_full_filter_update = false;

            for entry in entries {
                // Send macOS notification for errors
                #[cfg(feature = "macos")]
                if self.notify_on_error && entry.level == LogLevel::Error {
                    // Limit notifications to once every 5 seconds
                    if self
                        .last_notification
                        .map_or(true, |t| t.elapsed() > Duration::from_secs(5))
                    {
                        let bundle = get_bundle_identifier_or_default("com.devinsight.app");
                        let mut notification = Notification::new();
                        notification
                            .title("DevInsight Error")
                            .subtitle(&entry.tag)
                            .message(&entry.message)
                            .sound("Basso");

                        send_notification(
                            &bundle,
                            Some(&entry.tag),
                            "DevInsight Error",
                            Some(&notification),
                        )
                        .ok();
                        self.last_notification = Some(Instant::now());
                    }
                }

                // Batch process logs for better performance
                if self.logs.len() >= 10000 {
                    // Remove oldest 1000 logs when we hit the limit
                    for _ in 0..1000 {
                        self.logs.pop_front();
                    }
                    needs_full_filter_update = true;
                }

                // Update statistics
                match entry.level {
                    LogLevel::Error => self.stats.error_count += 1,
                    LogLevel::Warning => self.stats.warning_count += 1,
                    LogLevel::Info => self.stats.info_count += 1,
                    LogLevel::Debug => self.stats.debug_count += 1,
                    LogLevel::Verbose => self.stats.verbose_count += 1,
                    LogLevel::Unknown => (),
                }

                let log_index = self.logs.len();
                if !needs_full_filter_update && self.matches_filters(&entry) {
                    self.filtered_logs.push(log_index);
                }
                self.logs.push_back(entry);
            }

            if needs_full_filter_update {
                self.update_filtered_logs();
            } else if self.tail_mode {
                self.scroll = self.filtered_logs.len().saturating_sub(1);
            }
        }
    }

    pub fn toggle_level(&mut self, level: LogLevel) {
        if let Some(pos) = self.level_filters.iter().position(|&l| l == level) {
            self.level_filters.remove(pos);
        } else {
            self.level_filters.push(level);
        }
        self.update_filtered_logs();
    }

    fn matches_filters(&self, log: &LogEntry) -> bool {
        let level_match = self.level_filters.contains(&log.level);
        let search_match = if self.search_query.is_empty() {
            true
        } else {
            let search_term = self.search_query.to_lowercase();
            log.message.to_lowercase().contains(&search_term)
                || log.tag.to_lowercase().contains(&search_term)
                || log.level.as_str().to_lowercase().contains(&search_term)
        };

        level_match && search_match
    }

    fn update_filtered_logs(&mut self) {
        self.filtered_logs = self
            .logs
            .iter()
            .enumerate()
            .filter(|(_, log)| self.matches_filters(log))
            .map(|(i, _)| i)
            .collect();

        // Update scroll position if in tail mode
        if self.tail_mode {
            self.scroll = self.filtered_logs.len().saturating_sub(1);
        }
    }
}

pub struct Tui {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    state: AppState,
    log_rx: std::sync::mpsc::Receiver<LogEntry>,
    storage_rx: std::sync::mpsc::Receiver<StorageUpdate>,
    clipboard: Option<ClipboardContext>,
}

impl Tui {
    pub fn new(
        log_rx: std::sync::mpsc::Receiver<LogEntry>,
        storage_rx: std::sync::mpsc::Receiver<StorageUpdate>,
        initial_level_filters: Option<Vec<LogLevel>>,
    ) -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        stdout.execute(EnterAlternateScreen)?;
        stdout.execute(EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        let clipboard = ClipboardContext::new().ok();

        Ok(Self {
            terminal,
            state: AppState::new(initial_level_filters),
            log_rx,
            storage_rx,
            clipboard,
        })
    }

    pub fn run(&mut self) -> io::Result<()> {
        const SPINNERS: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let mut spinner_idx = 0;
        let mut collected = 0;
        let mut no_logs_count = 0;
        const INITIAL_BATCH_SIZE: usize = 50;
        const MAX_WAIT_CYCLES: usize = 20; // About 1 second max wait

        let mut last_check = Instant::now();
        const CHECK_INTERVAL: Duration = Duration::from_secs(5);

        while collected < INITIAL_BATCH_SIZE && no_logs_count < MAX_WAIT_CYCLES {
            self.terminal.draw(|f| {
                let area = f.size();
                let loading_area = Rect::new(
                    area.width.saturating_sub(40) / 2,
                    area.height.saturating_sub(3) / 2,
                    40.min(area.width),
                    3.min(area.height),
                );

                let status = if collected == 0 {
                    format!("{} Waiting for logs...", SPINNERS[spinner_idx])
                } else {
                    format!(
                        "{} Collecting logs {}/{}",
                        SPINNERS[spinner_idx], collected, INITIAL_BATCH_SIZE
                    )
                };

                let loading = Paragraph::new(status)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_type(ratatui::widgets::BorderType::Rounded),
                    )
                    .style(Style::default().fg(Color::Cyan))
                    .alignment(ratatui::layout::Alignment::Center);

                f.render_widget(loading, loading_area);
            })?;

            spinner_idx = (spinner_idx + 1) % SPINNERS.len();

            if let Ok(log) = self.log_rx.try_recv() {
                self.state.add_log(log);
                collected += 1;
                no_logs_count = 0;
            } else {
                no_logs_count += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }

            if last_check.elapsed() >= CHECK_INTERVAL {
                // Check ADB connection
                match Command::new("adb").args(["get-state"]).output() {
                    Ok(output) if output.status.success() => {
                        self.state.connection_status = ConnectionStatus::Connected;
                    }
                    Ok(_) => {
                        self.state.connection_status = ConnectionStatus::Disconnected;
                    }
                    Err(_e) => {
                        self.state.connection_status = ConnectionStatus::Error;
                    }
                }
                last_check = Instant::now();
            }
        }

        // Force initial update and scroll position
        self.state.update_filtered_logs();
        self.state.scroll = self.state.filtered_logs.len().saturating_sub(1);

        // Main event loop
        loop {
            // Process any new logs
            let mut batch = Vec::new();
            while let Ok(log) = self.log_rx.try_recv() {
                batch.push(log);
            }
            if !batch.is_empty() {
                self.state.add_logs(batch);
            }

            // Process storage updates
            while let Ok(update) = self.storage_rx.try_recv() {
                self.state.storage_info = Some(StorageInfo {
                    current_file: update.current_file,
                    total_size: update.total_size,
                    file_count: update.file_count,
                });
            }

            self.draw()?;

            if event::poll(std::time::Duration::from_millis(50))? {
                match event::read()? {
                    Event::Key(key) => {
                        if self.state.search_mode {
                            match key.code {
                                KeyCode::Esc => {
                                    self.state.search_mode = false;
                                    self.state.search_query.clear();
                                    self.state.update_filtered_logs();
                                }
                                KeyCode::Enter => {
                                    self.state.search_mode = false;
                                }
                                KeyCode::Char(c) => {
                                    self.state.search_query.push(c);
                                    self.state.update_filtered_logs();
                                }
                                KeyCode::Backspace => {
                                    if !self.state.search_query.is_empty() {
                                        self.state.search_query.pop();
                                        self.state.update_filtered_logs();
                                    }
                                }
                                _ => {}
                            }
                        } else {
                            match key.code {
                                KeyCode::Char('q') => break,
                                KeyCode::Char('1') => self.state.current_view = View::Logs,
                                KeyCode::Char('2') => self.state.current_view = View::Stats,
                                KeyCode::Char('3') => self.state.current_view = View::Storage,
                                KeyCode::Char('/') => self.state.search_mode = true,
                                KeyCode::Char(' ') => self.state.paused = !self.state.paused,
                                KeyCode::Char('t') => self.state.tail_mode = !self.state.tail_mode,
                                KeyCode::Up | KeyCode::Char('k') => {
                                    if self.state.scroll > 0 {
                                        self.state.tail_mode = false; // Disable tail mode when scrolling up
                                        self.state.scroll = self.state.scroll.saturating_sub(1);
                                    }
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    let max_scroll =
                                        self.state.filtered_logs.len().saturating_sub(1);
                                    if self.state.scroll < max_scroll {
                                        self.state.scroll += 1;
                                        // Only re-enable tail mode if we're at the very bottom
                                        if self.state.scroll == max_scroll {
                                            self.state.tail_mode = true;
                                        }
                                    }
                                }
                                KeyCode::Enter => {
                                    self.state.inspector_active = !self.state.inspector_active;
                                }
                                KeyCode::Char('o') | KeyCode::Char('W') => {
                                    self.state.wrap_mode = !self.state.wrap_mode;
                                    self.state.status_message = Some((
                                        format!(
                                            "Wrap mode {}",
                                            if self.state.wrap_mode {
                                                "enabled"
                                            } else {
                                                "disabled"
                                            }
                                        ),
                                        Instant::now(),
                                    ));
                                }
                                KeyCode::End | KeyCode::Char('G') => {
                                    let max_scroll =
                                        self.state.filtered_logs.len().saturating_sub(1);
                                    self.state.scroll = max_scroll;
                                }
                                KeyCode::Home | KeyCode::Char('g') => {
                                    self.state.scroll = 0;
                                }
                                KeyCode::Char('e') => self.state.toggle_level(LogLevel::Error),
                                KeyCode::Char('w') => self.state.toggle_level(LogLevel::Warning),
                                KeyCode::Char('i') => self.state.toggle_level(LogLevel::Info),
                                KeyCode::Char('d') => self.state.toggle_level(LogLevel::Debug),
                                KeyCode::Char('v') => self.state.toggle_level(LogLevel::Verbose),
                                KeyCode::PageUp => {
                                    self.state.tail_mode = false;
                                    self.state.scroll = self.state.scroll.saturating_sub(10);
                                }
                                KeyCode::PageDown => {
                                    let max_scroll =
                                        self.state.filtered_logs.len().saturating_sub(1);
                                    self.state.scroll = (self.state.scroll + 10).min(max_scroll);
                                    if self.state.scroll == max_scroll {
                                        self.state.tail_mode = true;
                                    }
                                }
                                KeyCode::Char('y') => {
                                    if let Some(clipboard) = &mut self.clipboard {
                                        if let Some(&index) =
                                            self.state.filtered_logs.get(self.state.scroll)
                                        {
                                            if let Some(log) = self.state.logs.get(index) {
                                                let log_text = format_log_for_clipboard(log);
                                                if clipboard.set_contents(log_text).is_ok() {
                                                    // Show copy confirmation in status
                                                    self.state.status_message = Some((
                                                        "Log copied to clipboard".to_string(),
                                                        Instant::now(),
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }
                                KeyCode::Char('c') => {
                                    // Command+C: Copy current log
                                    if let Some(clipboard) = &mut self.clipboard {
                                        if let Some(&index) =
                                            self.state.filtered_logs.get(self.state.scroll)
                                        {
                                            if let Some(log) = self.state.logs.get(index) {
                                                let log_text = format_log_for_clipboard(log);
                                                if clipboard.set_contents(log_text).is_ok() {
                                                    self.state.status_message = Some((
                                                        "Log copied to clipboard".to_string(),
                                                        Instant::now(),
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }
                                KeyCode::Char('n') => {
                                    // Command+N: Toggle notifications
                                    self.state.notify_on_error = !self.state.notify_on_error;
                                    self.state.status_message = Some((
                                        format!(
                                            "Notifications {}",
                                            if self.state.notify_on_error {
                                                "enabled"
                                            } else {
                                                "disabled"
                                            }
                                        ),
                                        Instant::now(),
                                    ));
                                }
                                _ => {}
                            }
                        }
                    }
                    Event::Mouse(MouseEvent { kind, .. }) => match kind {
                        MouseEventKind::ScrollUp => {
                            if self.state.scroll > 0 {
                                self.state.tail_mode = false;
                                self.state.scroll = self.state.scroll.saturating_sub(3);
                            }
                        }
                        MouseEventKind::ScrollDown => {
                            let max_scroll = self.state.filtered_logs.len().saturating_sub(1);
                            if self.state.scroll < max_scroll {
                                self.state.scroll = (self.state.scroll + 3).min(max_scroll);
                                if self.state.scroll == max_scroll {
                                    self.state.tail_mode = true;
                                }
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn draw(&mut self) -> io::Result<()> {
        self.terminal.draw(|f| {
            let size = f.size();
            let main_block = Block::default()
                .borders(Borders::NONE)
                .style(Style::default());

            let main_layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Length(3),
                        Constraint::Min(5),
                        Constraint::Length(1),
                        Constraint::Length(3),
                    ]
                    .as_ref(),
                )
                .horizontal_margin(1)
                .vertical_margin(0)
                .split(size);

            f.render_widget(main_block, size);
            Self::draw_tabs(f, main_layout[0], self.state.current_view);

            match self.state.current_view {
                View::Logs => Self::draw_logs(f, main_layout[1], &self.state),
                View::Stats => Self::draw_stats(f, main_layout[1], &self.state),
                View::Storage => Self::draw_storage(f, main_layout[1], &self.state),
            }

            FooterStatus::new(&self.state).render(f, main_layout[2], main_layout[3]);
        })?;
        Ok(())
    }

    fn draw_tabs(f: &mut Frame, area: Rect, current_view: View) {
        let titles = vec!["Logs", "Stats", "Storage"];
        let tabs = Tabs::new(titles)
            .block(Block::default().borders(Borders::ALL).title("Views"))
            .select(match current_view {
                View::Logs => 0,
                View::Stats => 1,
                View::Storage => 2,
            })
            .style(Style::default().fg(Color::White))
            .highlight_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );
        f.render_widget(tabs, area);
    }

    fn draw_logs(f: &mut Frame, area: Rect, state: &AppState) {
        // Split layout if inspector is active
        let (logs_area, inspector_area) = if state.inspector_active {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(60), Constraint::Percentage(40)].as_ref())
                .split(area);
            (chunks[0], Some(chunks[1]))
        } else {
            (area, None)
        };

        // Calculate actual display area accounting for borders and padding
        let inner_width = logs_area.width.saturating_sub(2); // Subtract 2 for borders
        let max_display = logs_area.height.saturating_sub(2); // Subtract 2 for borders
        let total_logs = state.filtered_logs.len();

        // Calculate the start index for displaying logs
        let start_index = if state.tail_mode {
            total_logs.saturating_sub(max_display as usize)
        } else {
            state.scroll
        };

        let visible_entries: Vec<&LogEntry> = state
            .filtered_logs
            .iter()
            .skip(start_index)
            .take(max_display as usize)
            .filter_map(|&index| state.logs.get(index))
            .collect();

        // Calculate dynamic tag column width based on visible logs
        let max_tag_len = visible_entries
            .iter()
            .map(|log| log.tag.len())
            .max()
            .unwrap_or(8);
        let tag_width = max_tag_len.clamp(8, 20);

        let visible_logs: Vec<ListItem> = visible_entries
            .iter()
            .enumerate()
            .map(|(i, log)| {
                let current_index = start_index + i;
                let is_selected = current_index == state.scroll;

                // Fixed widths for specific components
                const TIMESTAMP_WIDTH: usize = 19;
                const LEVEL_WIDTH: usize = 5;
                const PADDING: usize = 7; // For brackets, spaces, and colon

                // Calculate remaining width for message
                let message_width = (inner_width as usize)
                    .saturating_sub(TIMESTAMP_WIDTH)
                    .saturating_sub(tag_width)
                    .saturating_sub(LEVEL_WIDTH)
                    .saturating_sub(PADDING)
                    .saturating_sub(2); // Account for icon and space

                // Get the icon for the log level
                let icon = match log.level {
                    LogLevel::Error => "🔴",
                    LogLevel::Warning => "⚠️",
                    LogLevel::Info => "ℹ️",
                    LogLevel::Debug => "🔧",
                    LogLevel::Verbose => "📝",
                    LogLevel::Unknown => "❓",
                };

                let formatted_tag = format_tag(&log.tag, tag_width);
                let tag_color = get_tag_color(&log.tag);
                let level_color = log.level.color();
                let message_color = match log.level {
                    LogLevel::Error => Color::Red,
                    LogLevel::Warning => Color::Yellow,
                    LogLevel::Info => Color::Reset,
                    _ => Color::Gray,
                };

                let mut lines = Vec::new();

                if state.wrap_mode {
                    let wrapped = wrap_message(&log.message, message_width);
                    for (idx, msg_line) in wrapped.iter().enumerate() {
                        if idx == 0 {
                            let spans = vec![
                                Span::styled(format!("{} ", icon), Style::default().fg(level_color)),
                                Span::styled(format!("{:<TIMESTAMP_WIDTH$} ", log.timestamp), Style::default().fg(Color::DarkGray)),
                                Span::styled(format!("[{:<tag_width$}] ", formatted_tag), Style::default().fg(tag_color).add_modifier(Modifier::BOLD)),
                                Span::styled(format!("{:<LEVEL_WIDTH$}: ", log.level.as_str()), Style::default().fg(level_color)),
                                Span::styled(msg_line.clone(), Style::default().fg(message_color)),
                            ];
                            lines.push(Line::from(spans));
                        } else {
                            let padding_len = TIMESTAMP_WIDTH + tag_width + LEVEL_WIDTH + PADDING + 2;
                            let padding_spaces = " ".repeat(padding_len);
                            let spans = vec![
                                Span::styled(padding_spaces, Style::default().fg(Color::Reset)),
                                Span::styled(msg_line.clone(), Style::default().fg(message_color)),
                            ];
                            lines.push(Line::from(spans));
                        }
                    }
                } else {
                    let truncated_message = if log.message.len() > message_width {
                        format!(
                            "{}...",
                            log.message.chars().take(message_width.saturating_sub(3)).collect::<String>()
                        )
                    } else {
                        log.message.clone()
                    };

                    let spans = vec![
                        Span::styled(format!("{} ", icon), Style::default().fg(level_color)),
                        Span::styled(format!("{:<TIMESTAMP_WIDTH$} ", log.timestamp), Style::default().fg(Color::DarkGray)),
                        Span::styled(format!("[{:<tag_width$}] ", formatted_tag), Style::default().fg(tag_color).add_modifier(Modifier::BOLD)),
                        Span::styled(format!("{:<LEVEL_WIDTH$}: ", log.level.as_str()), Style::default().fg(level_color)),
                        Span::styled(truncated_message, Style::default().fg(message_color)),
                    ];
                    lines.push(Line::from(spans));
                }

                let mut item = ListItem::new(lines);
                if is_selected {
                    item = item.style(Style::default().bg(Color::Rgb(40, 44, 52)));
                }
                item
            })
            .collect();

        let title = if state.search_mode {
            format!(
                " Log Output (Searching: '{}', {} matches) ",
                state.search_query,
                state.filtered_logs.len()
            )
        } else {
            format!(" Log Output ({} logs) ", state.filtered_logs.len())
        };

        let logs = List::new(visible_logs)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_type(ratatui::widgets::BorderType::Rounded),
            )
            .highlight_style(Style::default().bg(Color::DarkGray));

        f.render_widget(logs, logs_area);

        // Render inspector if active
        if let Some(inspector_area) = inspector_area {
            let mut details = Vec::new();
            if let Some(&selected_index) = state.filtered_logs.get(state.scroll) {
                if let Some(log) = state.logs.get(selected_index) {
                    details.push(Line::from(vec![
                        Span::styled("Timestamp: ", Style::default().fg(Color::DarkGray)),
                        Span::raw(&log.timestamp),
                    ]));

                    let pid_str = log.pid.map_or("N/A".to_string(), |p| p.to_string());
                    let tid_str = log.tid.map_or("N/A".to_string(), |t| t.to_string());
                    details.push(Line::from(vec![
                        Span::styled("PID/TID:   ", Style::default().fg(Color::DarkGray)),
                        Span::raw(format!("{} / {}", pid_str, tid_str)),
                    ]));

                    details.push(Line::from(vec![
                        Span::styled("Level:     ", Style::default().fg(Color::DarkGray)),
                        Span::styled(log.level.as_str(), Style::default().fg(log.level.color()).add_modifier(Modifier::BOLD)),
                    ]));

                    details.push(Line::from(vec![
                        Span::styled("Tag:       ", Style::default().fg(Color::DarkGray)),
                        Span::styled(&log.tag, Style::default().fg(get_tag_color(&log.tag)).add_modifier(Modifier::BOLD)),
                    ]));

                    details.push(Line::from(""));
                    details.push(Line::from(Span::styled("Message:", Style::default().fg(Color::DarkGray))));

                    let message_color = match log.level {
                        LogLevel::Error => Color::Red,
                        LogLevel::Warning => Color::Yellow,
                        LogLevel::Info => Color::Reset,
                        _ => Color::Gray,
                    };

                    let inspector_inner_width = inspector_area.width.saturating_sub(2) as usize;
                    for line in wrap_message(&log.message, inspector_inner_width) {
                        details.push(Line::from(Span::styled(line, Style::default().fg(message_color))));
                    }
                } else {
                    details.push(Line::from("No log details available"));
                }
            } else {
                details.push(Line::from("No log selected"));
            }

            let paragraph = Paragraph::new(details)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Log Inspector ")
                        .border_type(ratatui::widgets::BorderType::Rounded)
                        .border_style(Style::default().fg(Color::Cyan)),
                );
            f.render_widget(paragraph, inspector_area);
        }
    }

    fn draw_stats(f: &mut Frame, area: Rect, state: &AppState) {
        let stats = format!(
            "\nLog Statistics:\n\
            \n\
            🔴 Errors:   {}\n\
            ⚠️  Warnings: {}\n\
            ℹ️  Info:     {}\n\
            🔧 Debug:    {}\n\
            📝 Verbose:  {}\n\
            \n\
            Total Logs: {}\n\
            Memory Usage: {} entries",
            state.stats.error_count,
            state.stats.warning_count,
            state.stats.info_count,
            state.stats.debug_count,
            state.stats.verbose_count,
            state.logs.len(),
            state.logs.capacity(),
        );

        let stats_widget = Paragraph::new(stats)
            .block(Block::default().borders(Borders::ALL).title("Statistics"))
            .style(Style::default().fg(Color::White));
        f.render_widget(stats_widget, area);
    }

    fn draw_storage(f: &mut Frame, area: Rect, state: &AppState) {
        let storage_info = if let Some(info) = &state.storage_info {
            format!(
                "\nStorage Information:\n\
                \n\
                Current File: {}\n\
                Total Size: {} MB\n\
                File Count: {}\n",
                info.current_file,
                info.total_size / (1024 * 1024),
                info.file_count,
            )
        } else {
            "\nStorage not enabled\n\nUse --save to enable log storage".to_string()
        };

        let storage_widget = Paragraph::new(storage_info)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Storage Status"),
            )
            .style(Style::default().fg(Color::White));
        f.render_widget(storage_widget, area);
    }
}

struct FooterStatus<'a> {
    state: &'a AppState,
    now: Instant,
}

impl<'a> FooterStatus<'a> {
    const HELP_TEXT: &'static str = "1-3: Views | Enter: Inspect | o: Wrap | Space: Pause | t: Tail | /: Search | y: Copy | e/w/i/d/v: Filters | ↑/↓: Scroll | q: Quit";
    const STATUS_MESSAGE_TTL: Duration = Duration::from_secs(2);

    fn new(state: &'a AppState) -> Self {
        Self {
            state,
            now: Instant::now(),
        }
    }

    #[cfg(test)]
    fn with_now(state: &'a AppState, now: Instant) -> Self {
        Self { state, now }
    }

    fn render(&self, f: &mut Frame, status_area: Rect, help_area: Rect) {
        let status = Paragraph::new(self.status_line()).style(Style::default().fg(Color::White));
        f.render_widget(status, status_area);

        let help = Paragraph::new(Self::HELP_TEXT)
            .block(Block::default().borders(Borders::ALL))
            .style(Style::default().fg(Color::Gray));
        f.render_widget(help, help_area);
    }

    fn status_line(&self) -> Line<'static> {
        if self.state.search_mode {
            return Line::from(format!(
                "Search: {} | Press Enter to confirm or Esc to cancel",
                self.state.search_query
            ));
        }

        if let Some((msg, time)) = &self.state.status_message {
            if self.now.duration_since(*time) <= Self::STATUS_MESSAGE_TTL {
                return Line::from(msg.clone());
            }
        }

        self.normal_status_line()
    }

    fn normal_status_line(&self) -> Line<'static> {
        let state = self.state;
        let mut spans = Vec::new();

        match state.connection_status {
            ConnectionStatus::Connected => {
                spans.push(Span::styled(
                    "🟢 Connected",
                    Style::default().fg(Color::Green),
                ));
            }
            ConnectionStatus::Disconnected => {
                spans.push(Span::styled(
                    "🔴 Disconnected",
                    Style::default().fg(Color::Red),
                ));
            }
            ConnectionStatus::Error => {
                spans.push(Span::styled(
                    "⚠️  Error",
                    Style::default().fg(Color::Yellow),
                ));
            }
        }

        spans.push(Span::raw(format!(
            " | {:>3} logs | Filters [",
            state.filtered_logs.len()
        )));
        self.push_level_filter(&mut spans, LogLevel::Error, "E", Color::Red);
        spans.push(Span::raw(" "));
        self.push_level_filter(&mut spans, LogLevel::Warning, "W", Color::Yellow);
        spans.push(Span::raw(" "));
        self.push_level_filter(&mut spans, LogLevel::Info, "I", Color::Green);
        spans.push(Span::raw(" "));
        self.push_level_filter(&mut spans, LogLevel::Debug, "D", Color::Blue);
        spans.push(Span::raw(" "));
        self.push_level_filter(&mut spans, LogLevel::Verbose, "V", Color::White);
        spans.push(Span::raw(format!(
            "] | {:>3}/{:<3} | ",
            state.scroll + 1,
            state.filtered_logs.len()
        )));

        if state.paused {
            spans.push(Span::styled("PAUSED", Style::default().fg(Color::Red)));
        } else {
            spans.push(Span::styled("RUNNING", Style::default().fg(Color::Green)));
        }
        spans.push(Span::raw(" | "));

        if state.tail_mode {
            spans.push(Span::styled("TAIL", Style::default().fg(Color::Cyan)));
        } else {
            spans.push(Span::styled("SCROLL", Style::default().fg(Color::Yellow)));
        }
        spans.push(Span::raw(format!(" | {}", state.current_view)));

        Line::from(spans)
    }

    fn push_level_filter(
        &self,
        spans: &mut Vec<Span<'static>>,
        level: LogLevel,
        label: &'static str,
        color: Color,
    ) {
        if self.state.level_filters.contains(&level) {
            spans.push(Span::styled(label, Style::default().fg(color)));
        } else {
            spans.push(Span::styled("-", Style::default().fg(Color::DarkGray)));
        }
    }
}

fn format_log_for_clipboard(log: &LogEntry) -> String {
    let thread_info = match (log.pid, log.tid) {
        (Some(pid), Some(tid)) => format!(" pid={} tid={}", pid, tid),
        (Some(pid), None) => format!(" pid={}", pid),
        (None, Some(tid)) => format!(" tid={}", tid),
        (None, None) => String::new(),
    };

    format!(
        "{}{} [{}] {}: {}",
        log.timestamp,
        thread_info,
        log.tag,
        log.level.as_str(),
        log.message
    )
}

fn get_tag_color(tag: &str) -> Color {
    let mut hash: u32 = 5381;
    for c in tag.bytes() {
        hash = ((hash << 5).wrapping_add(hash)).wrapping_add(c as u32);
    }
    let colors = [
        Color::Cyan,
        Color::Magenta,
        Color::Blue,
        Color::Green,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
    ];
    colors[(hash as usize) % colors.len()]
}

fn format_tag(tag: &str, width: usize) -> String {
    if tag.len() <= width {
        format!("{:<width$}", tag, width = width)
    } else {
        let mut truncated = tag.chars().take(width.saturating_sub(2)).collect::<String>();
        truncated.push_str("..");
        truncated
    }
}

fn wrap_message(message: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![message.to_string()];
    }
    let mut lines = Vec::new();
    for sub_msg in message.lines() {
        let mut current_line = String::new();
        for word in sub_msg.split_whitespace() {
            if current_line.is_empty() {
                current_line.push_str(word);
            } else if current_line.len() + 1 + word.len() <= width {
                current_line.push(' ');
                current_line.push_str(word);
            } else {
                lines.push(current_line);
                current_line = word.to_string();
                while current_line.len() > width {
                    let (part1, part2) = current_line.split_at(width);
                    lines.push(part1.to_string());
                    current_line = part2.to_string();
                }
            }
        }
        if !current_line.is_empty() {
            lines.push(current_line);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

impl Drop for Tui {
    fn drop(&mut self) {
        disable_raw_mode().unwrap();
        self.terminal
            .backend_mut()
            .execute(LeaveAlternateScreen)
            .unwrap();
        self.terminal
            .backend_mut()
            .execute(DisableMouseCapture)
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, buffer::Buffer};

    fn render_footer_snapshot(state: &AppState, now: Instant, width: u16) -> Vec<String> {
        let backend = TestBackend::new(width, 4);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                FooterStatus::with_now(state, now).render(
                    f,
                    Rect::new(0, 0, width, 1),
                    Rect::new(0, 1, width, 3),
                );
            })
            .unwrap();

        buffer_lines(terminal.backend().buffer())
    }

    fn buffer_lines(buffer: &Buffer) -> Vec<String> {
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer.get(x, y).symbol())
                    .collect::<String>()
            })
            .collect()
    }

    fn app_state_with_logs() -> AppState {
        let mut state = AppState::new(None);
        state.filtered_logs = vec![0, 1, 2];
        state.scroll = 2;
        state
    }

    #[test]
    fn footer_normal_state_matches_golden_snapshot() {
        let state = app_state_with_logs();
        let now = Instant::now();

        let lines = render_footer_snapshot(&state, now, 96);

        assert_eq!(
            lines,
            vec![
                "🟢  Connected |   3 logs | Filters [E W I D V] |   3/3   | RUNNING | TAIL | Logs                 ",
                "┌──────────────────────────────────────────────────────────────────────────────────────────────┐",
                "│1-3: Views | Enter: Inspect | o: Wrap | Space: Pause | t: Tail | /: Search | y: Copy | e/w/i/d│",
                "└──────────────────────────────────────────────────────────────────────────────────────────────┘",
            ]
        );
    }

    #[test]
    fn footer_search_mode_matches_golden_snapshot() {
        let mut state = app_state_with_logs();
        state.search_mode = true;
        state.search_query = "fatal".to_string();
        let now = Instant::now();

        let lines = render_footer_snapshot(&state, now, 80);

        assert_eq!(
            lines,
            vec![
                "Search: fatal | Press Enter to confirm or Esc to cancel                         ",
                "┌──────────────────────────────────────────────────────────────────────────────┐",
                "│1-3: Views | Enter: Inspect | o: Wrap | Space: Pause | t: Tail | /: Search | y│",
                "└──────────────────────────────────────────────────────────────────────────────┘",
            ]
        );
    }

    #[test]
    fn footer_recent_status_message_matches_golden_snapshot() {
        let mut state = app_state_with_logs();
        let now = Instant::now();
        state.status_message = Some(("Log copied to clipboard".to_string(), now));

        let lines = render_footer_snapshot(&state, now, 72);

        assert_eq!(
            lines,
            vec![
                "Log copied to clipboard                                                 ",
                "┌──────────────────────────────────────────────────────────────────────┐",
                "│1-3: Views | Enter: Inspect | o: Wrap | Space: Pause | t: Tail | /: Se│",
                "└──────────────────────────────────────────────────────────────────────┘",
            ]
        );
    }

    #[test]
    fn footer_expired_status_message_falls_back_to_normal_snapshot() {
        let mut state = app_state_with_logs();
        let now = Instant::now();
        state.status_message = Some((
            "Log copied to clipboard".to_string(),
            now - FooterStatus::STATUS_MESSAGE_TTL - Duration::from_secs(1),
        ));

        let lines = render_footer_snapshot(&state, now, 96);

        assert_eq!(
            lines[0],
            "🟢  Connected |   3 logs | Filters [E W I D V] |   3/3   | RUNNING | TAIL | Logs                 "
        );
    }
}
