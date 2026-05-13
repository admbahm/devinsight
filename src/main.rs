use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread::JoinHandle;

use chrono::Local;
use clap::Parser;
use colored::*;
use thiserror::Error;

mod storage;
mod tui;

use storage::{LogStorage, StoredLog};
use tui::{LogEntry, LogLevel, Tui};

#[derive(Error, Debug)]
pub enum DevInsightError {
    #[error("ADB not found or not accessible")]
    AdbNotFound,
    #[error("No connected Android device or running emulator found")]
    AdbDeviceNotConnected,
    #[error("Failed to capture logcat output: {0}")]
    LogcatCaptureFailed(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Invalid timestamp format: {0}")]
    TimestampError(String),
    #[error("Storage error: {0}")]
    StorageError(String),
    #[error("JSON serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}

#[derive(Parser, Debug)]
#[command(name = "DevInsight")]
#[command(author = "Adam Deane")]
#[command(version = "0.1.0")]
#[command(about = "Real-time Android Log Analyzer")]
struct Cli {
    #[arg(short, long, help = "Filter logs by error level (E, W, D, etc.)")]
    filter: Option<String>,

    #[arg(short, long, help = "Filter logs by specific tag")]
    tag: Option<String>,

    #[arg(short = 'c', long, help = "Clear logs before starting")]
    clear: bool,

    #[arg(
        short = 'T',
        long,
        help = "Show logs from specific timestamp or count accepted by adb logcat -T"
    )]
    since: Option<String>,

    #[arg(short = 'b', long = "buffer", help = "Select buffer (main, system, crash)", value_parser = ["main", "system", "crash"], default_value = "main")]
    buffer: String,

    #[arg(short = 'v', long = "format", help = "Log format (brief, process, tag, thread, raw)", value_parser = ["brief", "process", "tag", "thread", "raw"], default_value = "brief")]
    format: String,

    #[arg(short = 'i', long = "interactive", help = "Use interactive TUI mode")]
    interactive: bool,

    #[arg(long = "save", help = "Save logs to file")]
    save: bool,

    #[arg(
        long = "save-path",
        help = "Directory to save logs",
        default_value = "logs"
    )]
    save_path: PathBuf,

    #[arg(
        long = "max-size",
        help = "Maximum log file size in MB before rotation",
        default_value = "100"
    )]
    max_size: u64,

    #[arg(long = "load", help = "Load and analyze logs from file")]
    load: Option<PathBuf>,
}

#[derive(Clone, Debug, Default)]
struct LogFilter {
    level: Option<LogLevel>,
    tag: Option<String>,
}

impl LogFilter {
    fn from_cli(cli: &Cli) -> Result<Self, DevInsightError> {
        let level = cli.filter.as_deref().map(parse_filter_level).transpose()?;

        Ok(Self {
            level,
            tag: cli.tag.clone(),
        })
    }

    fn should_process_entry(&self, entry: &LogEntry) -> bool {
        if let Some(level) = self.level {
            if entry.level != level {
                return false;
            }
        }

        if let Some(tag) = &self.tag {
            if !entry.tag.contains(tag) {
                return false;
            }
        }

        true
    }

    fn initial_tui_levels(&self) -> Option<Vec<LogLevel>> {
        self.level.map(|level| vec![level])
    }
}

fn main() -> Result<(), DevInsightError> {
    let cli = Cli::parse();

    if cli.interactive {
        run_interactive_mode(&cli)?;
    } else {
        run_standard_mode(&cli)?;
    }

    Ok(())
}

fn run_interactive_mode(cli: &Cli) -> Result<(), DevInsightError> {
    let filter = LogFilter::from_cli(cli)?;
    let (log_tx, log_rx) = std::sync::mpsc::channel();
    let (storage_tx, storage_rx) = std::sync::mpsc::channel();
    let mut tui = Tui::new(log_rx, storage_rx, filter.initial_tui_levels())?;

    if let Some(load_path) = &cli.load {
        for entry in load_entries(load_path)? {
            if filter.should_process_entry(&entry) {
                log_tx.send(entry).ok();
            }
        }
        drop(log_tx);
        return tui.run().map_err(DevInsightError::IoError);
    }

    if cli.clear {
        clear_logs()?;
    }

    let storage = if cli.save {
        Some(LogStorage::new(
            cli.save_path.clone(),
            cli.max_size,
            Some(storage_tx),
        )?)
    } else {
        None
    };

    ensure_adb_available()?;
    let mut child = spawn_logcat(cli, "threadtime", Some("50"))?;
    let stdout = child.stdout.take().ok_or_else(|| {
        DevInsightError::LogcatCaptureFailed("Failed to capture stdout".to_string())
    })?;
    let reader = BufReader::new(stdout);
    let reader_handle = spawn_reader_thread(reader, log_tx, filter, storage);

    let run_result = tui.run().map_err(DevInsightError::IoError);
    stop_logcat_child(&mut child);
    let _ = reader_handle.join();

    run_result
}

fn run_standard_mode(cli: &Cli) -> Result<(), DevInsightError> {
    colored::control::set_override(true);

    println!("{}", "DevInsight: Android Log Analyzer".cyan().bold());
    println!("{}", "=".repeat(50).cyan());

    let filter = LogFilter::from_cli(cli)?;

    if let Some(load_path) = &cli.load {
        println!("{}", "Loading stored logs...".cyan().bold());
        for entry in load_entries(load_path)? {
            if filter.should_process_entry(&entry) {
                println!("{}", format_entry_for_cli(&entry));
            }
        }
        return Ok(());
    }

    if cli.clear {
        clear_logs()?;
        println!("{}", "Logs cleared.".green().bold());
    }

    ensure_adb_available()?;
    let mut child = spawn_logcat(cli, &cli.format, None)?;
    let stdout = child.stdout.take().ok_or_else(|| {
        DevInsightError::LogcatCaptureFailed("Failed to capture stdout".to_string())
    })?;
    let reader = BufReader::new(stdout);

    println!("{}", "Log Settings:".yellow().bold());
    println!("Buffer: {}", cli.buffer.blue());
    println!("Format: {}", cli.format.blue());
    if let Some(since) = &cli.since {
        println!("Since: {}", since.blue());
    }
    if let Some(level) = &cli.filter {
        println!("Filter Level: {}", level.blue());
    }
    if let Some(tag) = &cli.tag {
        println!("Tag Filter: {}", tag.blue());
    }
    println!("{}", "=".repeat(50).yellow());

    let mut storage = if cli.save {
        Some(LogStorage::new(cli.save_path.clone(), cli.max_size, None)?)
    } else {
        None
    };

    for line in reader.lines() {
        match line {
            Ok(raw_log) => {
                let entry = parse_log_entry(&raw_log);
                if filter.should_process_entry(&entry) {
                    if let Some(storage) = &mut storage {
                        storage.store_log(stored_log_from_entry(&entry)).ok();
                    }
                    println!("{}", colorize_raw_or_entry(&raw_log, &entry));
                }
            }
            Err(err) => {
                stop_logcat_child(&mut child);
                return Err(DevInsightError::IoError(err));
            }
        }
    }

    stop_logcat_child(&mut child);
    Ok(())
}

fn spawn_reader_thread<R: BufRead + Send + 'static>(
    reader: R,
    log_tx: std::sync::mpsc::Sender<LogEntry>,
    filter: LogFilter,
    mut storage: Option<LogStorage>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        for line in reader.lines() {
            let Ok(raw_log) = line else {
                break;
            };
            let entry = parse_log_entry(&raw_log);
            if !filter.should_process_entry(&entry) {
                continue;
            }

            if let Some(storage) = &mut storage {
                storage.store_log(stored_log_from_entry(&entry)).ok();
            }

            if log_tx.send(entry).is_err() {
                break;
            }
        }
    })
}

fn spawn_logcat(
    cli: &Cli,
    format: &str,
    default_tail_count: Option<&str>,
) -> Result<Child, DevInsightError> {
    let args = build_logcat_args(
        &cli.buffer,
        format,
        cli.since.as_deref(),
        default_tail_count,
    );
    Command::new("adb")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| DevInsightError::AdbNotFound)
}

fn build_logcat_args(
    buffer: &str,
    format: &str,
    since: Option<&str>,
    default_tail_count: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "logcat".to_string(),
        "-b".to_string(),
        buffer.to_string(),
        "-v".to_string(),
        format.to_string(),
    ];

    if let Some(since) = since {
        args.push("-T".to_string());
        args.push(since.to_string());
    } else if let Some(count) = default_tail_count {
        args.push("-T".to_string());
        args.push(count.to_string());
    }

    args
}

fn ensure_adb_available() -> Result<(), DevInsightError> {
    let output = Command::new("adb")
        .arg("devices")
        .output()
        .map_err(|_| DevInsightError::AdbNotFound)?;

    if !output.status.success() {
        return Err(DevInsightError::AdbNotFound);
    }

    let state = Command::new("adb")
        .arg("get-state")
        .output()
        .map_err(|_| DevInsightError::AdbNotFound)?;

    if state.status.success() {
        Ok(())
    } else {
        Err(DevInsightError::AdbDeviceNotConnected)
    }
}

fn clear_logs() -> Result<(), DevInsightError> {
    Command::new("adb")
        .args(["logcat", "-c"])
        .output()
        .map_err(|_| DevInsightError::AdbNotFound)?;
    Ok(())
}

fn stop_logcat_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        child.kill().ok();
    }
    child.wait().ok();
}

fn parse_filter_level(level: &str) -> Result<LogLevel, DevInsightError> {
    let parsed = if level.len() == 1 {
        LogLevel::from_logcat_char(level.chars().next().unwrap().to_ascii_uppercase())
    } else {
        LogLevel::from_storage_str(level)
    };

    if parsed == LogLevel::Unknown {
        Err(DevInsightError::LogcatCaptureFailed(format!(
            "Invalid log level filter '{}'. Use E, W, I, D, or V.",
            level
        )))
    } else {
        Ok(parsed)
    }
}

fn parse_log_entry(log: &str) -> LogEntry {
    parse_threadtime_log(log).unwrap_or_else(|| parse_fallback_log(log))
}

fn parse_threadtime_log(log: &str) -> Option<LogEntry> {
    let mut parts = log.split_whitespace();
    let date = parts.next()?;
    let time = parts.next()?;
    let pid = parts.next()?.parse::<u32>().ok()?;
    let tid = parts.next()?.parse::<u32>().ok()?;
    let level_raw = parts.next()?;
    let level_char = level_raw.chars().next()?;
    let tag_with_colon = parts.next()?;
    let tag = tag_with_colon.strip_suffix(':')?;
    let message = parts.collect::<Vec<_>>().join(" ");

    Some(LogEntry {
        level: LogLevel::from_logcat_char(level_char),
        timestamp: format!("{} {}", date, time),
        pid: Some(pid),
        tid: Some(tid),
        tag: tag.to_string(),
        message,
    })
}

fn parse_fallback_log(log: &str) -> LogEntry {
    let mut level = LogLevel::Unknown;
    let mut tag = "UNKNOWN".to_string();
    let mut message = log.to_string();

    for marker in ["E/", "W/", "I/", "D/", "V/"] {
        if let Some(start) = log.find(marker) {
            level = LogLevel::from_logcat_char(marker.chars().next().unwrap());
            let rest = &log[start + marker.len()..];
            if let Some((parsed_tag, parsed_message)) = rest.split_once(':') {
                tag = parsed_tag
                    .split_once('(')
                    .map(|(tag, _)| tag)
                    .unwrap_or(parsed_tag)
                    .trim()
                    .to_string();
                message = parsed_message.trim().to_string();
            }
            break;
        }
    }

    LogEntry {
        level,
        timestamp: Local::now().format("%m-%d %H:%M:%S").to_string(),
        pid: None,
        tid: None,
        tag,
        message,
    }
}

fn stored_log_from_entry(entry: &LogEntry) -> StoredLog {
    StoredLog {
        timestamp: Local::now(),
        level: entry.level.as_str().to_string(),
        tag: entry.tag.clone(),
        message: entry.message.clone(),
        device_id: None,
    }
}

fn entry_from_stored_log(log: StoredLog) -> LogEntry {
    LogEntry {
        level: LogLevel::from_storage_str(&log.level),
        timestamp: log.timestamp.format("%m-%d %H:%M:%S%.3f").to_string(),
        pid: None,
        tid: None,
        tag: log.tag,
        message: log.message,
    }
}

fn load_entries(path: &PathBuf) -> Result<Vec<LogEntry>, DevInsightError> {
    let logs = LogStorage::load_logs_from_file(path)?;
    Ok(logs.into_iter().map(entry_from_stored_log).collect())
}

fn colorize_raw_or_entry(raw_log: &str, entry: &LogEntry) -> String {
    match entry.level {
        LogLevel::Error => format!("{}  {}", "🔴".red().bold(), raw_log.bright_red().bold()),
        LogLevel::Warning => format!(
            "{}  {}",
            "⚠️".yellow().bold(),
            raw_log.bright_yellow().bold()
        ),
        LogLevel::Info => format!("{}  {}", "ℹ️".green(), raw_log.bright_green()),
        LogLevel::Debug => format!("{}  {}", "🔧".blue(), raw_log.bright_blue()),
        LogLevel::Verbose => format!("{}  {}", "📝".white(), raw_log.bright_white()),
        LogLevel::Unknown => format!("{}  {}", "❓".normal(), raw_log),
    }
}

fn format_entry_for_cli(entry: &LogEntry) -> String {
    let line = format!(
        "{} [{}] {}: {}",
        entry.timestamp,
        entry.tag,
        entry.level.as_str(),
        entry.message
    );
    colorize_raw_or_entry(&line, entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cli(buffer: &str, since: Option<&str>) -> Cli {
        Cli {
            filter: None,
            tag: None,
            clear: false,
            since: since.map(ToString::to_string),
            buffer: buffer.to_string(),
            format: "brief".to_string(),
            interactive: false,
            save: false,
            save_path: PathBuf::from("logs"),
            max_size: 100,
            load: None,
        }
    }

    #[test]
    fn parses_threadtime_line() {
        let entry = parse_log_entry("03-21 10:23:45.678  1234  5678 D TestApp: Message text");
        assert_eq!(entry.timestamp, "03-21 10:23:45.678");
        assert_eq!(entry.pid, Some(1234));
        assert_eq!(entry.tid, Some(5678));
        assert_eq!(entry.level, LogLevel::Debug);
        assert_eq!(entry.tag, "TestApp");
        assert_eq!(entry.message, "Message text");
    }

    #[test]
    fn parses_message_with_colons() {
        let entry =
            parse_log_entry("03-21 10:23:45.678  1234  5678 E TestApp: failed: reason: detail");
        assert_eq!(entry.level, LogLevel::Error);
        assert_eq!(entry.tag, "TestApp");
        assert_eq!(entry.message, "failed: reason: detail");
    }

    #[test]
    fn parses_long_tag_name() {
        let entry =
            parse_log_entry("03-21 10:23:45.678  1234  5678 I VeryLongApplicationTag: Started");
        assert_eq!(entry.level, LogLevel::Info);
        assert_eq!(entry.tag, "VeryLongApplicationTag");
    }

    #[test]
    fn keeps_malformed_lines_as_unknown() {
        let entry = parse_log_entry("not a logcat line");
        assert_eq!(entry.level, LogLevel::Unknown);
        assert_eq!(entry.tag, "UNKNOWN");
        assert_eq!(entry.message, "not a logcat line");
    }

    #[test]
    fn parses_brief_fallback_line() {
        let entry = parse_log_entry("E/TestApp ( 1234): failed: reason");
        assert_eq!(entry.level, LogLevel::Error);
        assert_eq!(entry.tag, "TestApp");
        assert_eq!(entry.message, "failed: reason");
    }

    #[test]
    fn parses_all_log_levels() {
        for (raw, expected) in [
            ('E', LogLevel::Error),
            ('W', LogLevel::Warning),
            ('I', LogLevel::Info),
            ('D', LogLevel::Debug),
            ('V', LogLevel::Verbose),
        ] {
            let line = format!("03-21 10:23:45.678  1234  5678 {} TestApp: Message", raw);
            assert_eq!(parse_log_entry(&line).level, expected);
        }
    }

    #[test]
    fn filters_by_level() {
        let filter = LogFilter {
            level: Some(LogLevel::Error),
            tag: None,
        };
        assert!(filter.should_process_entry(&parse_log_entry(
            "03-21 10:23:45.678  1234  5678 E TestApp: Message"
        )));
        assert!(!filter.should_process_entry(&parse_log_entry(
            "03-21 10:23:45.678  1234  5678 W TestApp: Message"
        )));
    }

    #[test]
    fn filters_by_tag() {
        let filter = LogFilter {
            level: None,
            tag: Some("TestApp".to_string()),
        };
        assert!(filter.should_process_entry(&parse_log_entry(
            "03-21 10:23:45.678  1234  5678 I TestApp: Message"
        )));
        assert!(!filter.should_process_entry(&parse_log_entry(
            "03-21 10:23:45.678  1234  5678 I OtherApp: Message"
        )));
    }

    #[test]
    fn filters_by_level_and_tag() {
        let filter = LogFilter {
            level: Some(LogLevel::Error),
            tag: Some("TestApp".to_string()),
        };
        assert!(filter.should_process_entry(&parse_log_entry(
            "03-21 10:23:45.678  1234  5678 E TestApp: Message"
        )));
        assert!(!filter.should_process_entry(&parse_log_entry(
            "03-21 10:23:45.678  1234  5678 E OtherApp: Message"
        )));
        assert!(!filter.should_process_entry(&parse_log_entry(
            "03-21 10:23:45.678  1234  5678 W TestApp: Message"
        )));
    }

    #[test]
    fn builds_logcat_args_for_buffers() {
        for buffer in ["main", "system", "crash"] {
            let cli = test_cli(buffer, None);
            let args = build_logcat_args(&cli.buffer, "brief", None, None);
            assert_eq!(args, vec!["logcat", "-b", buffer, "-v", "brief"]);
        }
    }

    #[test]
    fn builds_logcat_args_with_since() {
        let cli = test_cli("main", Some("2024-03-20 10:00:00"));
        let args = build_logcat_args(&cli.buffer, "brief", cli.since.as_deref(), None);
        assert_eq!(
            args,
            vec![
                "logcat",
                "-b",
                "main",
                "-v",
                "brief",
                "-T",
                "2024-03-20 10:00:00"
            ]
        );
    }

    #[test]
    fn tui_and_standard_share_buffer_since_args() {
        let cli = test_cli("system", Some("50"));
        let standard = build_logcat_args(&cli.buffer, &cli.format, cli.since.as_deref(), None);
        let tui = build_logcat_args(&cli.buffer, "threadtime", cli.since.as_deref(), Some("50"));

        assert_eq!(&standard[0..3], &tui[0..3]);
        assert_eq!(&standard[5..], &tui[5..]);
        assert_eq!(tui[4], "threadtime");
    }
}
