use std::process::{Command, Stdio};
use std::io::{BufRead, BufReader};
use thiserror::Error;
use colored::*;
use clap::Parser;
use std::path::PathBuf;
mod tui;
use tui::{Tui, LogEntry, LogLevel};
use chrono::Local;
mod storage;
use storage::{LogStorage, StoredLog};

#[derive(Error, Debug)]
pub enum DevInsightError {
    #[error("ADB not found or not accessible")]
    AdbNotFound,
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
    
    #[arg(short = 'T', long, help = "Show logs from specific timestamp (format: 'YYYY-MM-DD HH:MM:SS')")]
    since: Option<String>,
    
    #[arg(short = 'b', long = "buffer", help = "Select buffer (main, system, crash)", value_parser = ["main", "system", "crash"], default_value = "main")]
    buffer: String,
    
    #[arg(short = 'v', long = "format", help = "Log format (brief, process, tag, thread, raw)", value_parser = ["brief", "process", "tag", "thread", "raw"], default_value = "brief")]
    format: String,
    
    #[arg(short = 'i', long = "interactive", help = "Use interactive TUI mode")]
    interactive: bool,
    
    #[arg(long = "save", help = "Save logs to file")]
    save: bool,
    
    #[arg(long = "save-path", help = "Directory to save logs", default_value = "logs")]
    save_path: PathBuf,
    
    #[arg(long = "max-size", help = "Maximum log file size in MB before rotation", default_value = "100")]
    max_size: u64,
    
    #[arg(long = "load", help = "Load and analyze logs from file")]
    load: Option<PathBuf>,
}

struct LogProcessor {
    filter_level: Option<String>,
    filter_tag: Option<String>,
}

impl LogProcessor {
    fn new(filter_level: Option<String>, filter_tag: Option<String>) -> Self {
        Self {
            filter_level,
            filter_tag,
        }
    }

    fn should_process_log(&self, log: &str) -> bool {
        if let Some(level) = &self.filter_level {
            let level_pattern = format!(" {}/", level); // Brief format
            let alt_pattern = format!("/{} ", level);   // Tag format
            if !log.contains(&level_pattern) && !log.contains(&alt_pattern) {
                return false;
            }
        }

        if let Some(tag) = &self.filter_tag {
            if !log.contains(tag) {
                return false;
            }
        }

        true
    }

    fn format_log(&self, log: &str) -> String {
        // Remove debug prints
        let formatted = if log.contains("E/") || log.contains(" E ") || log.contains("Error:") {
            format!("{}  {}", "🔴".red().bold(), log.bright_red().bold())
        } else if log.contains("W/") || log.contains(" W ") || log.contains("Warning:") {
            format!("{}  {}", "⚠️".yellow().bold(), log.bright_yellow().bold())
        } else if log.contains("I/") || log.contains(" I ") {
            format!("{}  {}", "ℹ️".green(), log.bright_green())
        } else if log.contains("D/") || log.contains(" D ") {
            format!("{}  {}", "🔧".blue(), log.bright_blue())
        } else if log.contains("V/") || log.contains(" V ") {
            format!("{}  {}", "📝".white(), log.bright_white())
        } else {
            format!("{}  {}", "❓".normal(), log)
        };

        // Keep color override
        colored::control::set_override(true);
        formatted
    }
}

fn main() -> Result<(), DevInsightError> {
    let cli = Cli::parse();
    
    if cli.interactive {
        run_interactive_mode(&cli)?;
    } else {
        run_standard_mode(cli)?;
    }
    
    Ok(())
}

fn run_interactive_mode(cli: &Cli) -> Result<(), DevInsightError> {
    // Create channels for logs and storage updates
    let (log_tx, log_rx) = std::sync::mpsc::channel();
    let (storage_tx, storage_rx) = std::sync::mpsc::channel();
    
    // Create TUI with receivers
    let mut tui = Tui::new(log_rx, storage_rx).map_err(|e| DevInsightError::IoError(e))?;
    
    // Initialize storage if needed
    let storage = if cli.save {
        Some(LogStorage::new(
            cli.save_path.clone(),
            cli.max_size,
            Some(storage_tx)
        ).map_err(|e| DevInsightError::StorageError(e.to_string()))?)
    } else {
        None
    };

    // Set up ADB command with optimized buffer settings
    let process = Command::new("adb")
        .args(["logcat", 
              "-v", "threadtime",     // Use threadtime format
              "-T", "50",            // Get last 50 logs
              "-b", "all"])          // All buffers
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| DevInsightError::AdbNotFound)?;

    let stdout = process.stdout
        .ok_or(DevInsightError::LogcatCaptureFailed("Failed to capture stdout".to_string()))?;
    let reader = BufReader::new(stdout);

    // Process logs in a separate thread
    let tx_clone = log_tx.clone();
    let mut storage = storage;  // Move storage into the thread
    std::thread::spawn(move || {
        for line in reader.lines() {
            match line {
                Ok(log) => {
                    let entry = parse_log_entry(&log);
                    
                    // Store log if storage is enabled
                    if let Some(storage) = &mut storage {
                        let stored_log = StoredLog {
                            timestamp: Local::now(),
                            level: entry.level.as_str().to_string(),
                            tag: entry.tag.clone(),
                            message: entry.message.clone(),
                            device_id: None,
                        };
                        storage.store_log(stored_log).ok();
                    }
                    
                    tx_clone.send(entry).ok();
                }
                Err(e) => {
                    eprintln!("Error reading log: {}", e);  // Use eprintln for errors
                }
            }
        }
    });

    // Run the TUI
    tui.run().map_err(|e| DevInsightError::IoError(e))?;
    
    Ok(())
}

fn parse_log_entry(log: &str) -> LogEntry {
    // Example threadtime format: "03-21 10:23:45.678  1234  5678 D Tag: Message"
    let parts: Vec<&str> = log.splitn(2, ':').collect();
    let message = parts.get(1)
        .map(|s| s.trim())
        .unwrap_or(log)
        .to_string();
    
    let header_parts: Vec<&str> = parts.get(0)
        .unwrap_or(&"")
        .split_whitespace()
        .collect();
    
    let timestamp = if header_parts.len() >= 2 {
        format!("{} {}", header_parts[0], header_parts[1])
    } else {
        chrono::Local::now().format("%m-%d %H:%M:%S").to_string()
    };

    let tag = header_parts
        .iter()
        .rev()
        .take(2)
        .last()
        .unwrap_or(&"UNKNOWN")
        .to_string();

    let level = if log.contains(" E ") || log.contains("Error") {
        LogLevel::Error
    } else if log.contains(" W ") || log.contains("Warning") {
        LogLevel::Warning
    } else if log.contains(" I ") || log.contains("Info") {
        LogLevel::Info
    } else if log.contains(" D ") || log.contains("Debug") {
        LogLevel::Debug
    } else if log.contains(" V ") || log.contains("Verbose") {
        LogLevel::Verbose
    } else {
        LogLevel::Unknown
    };

    LogEntry {
        level,
        timestamp,
        tag,
        message,
    }
}

// Rename existing main logic
fn run_standard_mode(cli: Cli) -> Result<(), DevInsightError> {
    // Force color output
    colored::control::set_override(true);
    
    println!("{}", "DevInsight: Android Log Analyzer".cyan().bold());
    println!("{}", "=".repeat(50).cyan());

    let processor = LogProcessor::new(cli.filter.clone(), cli.tag.clone());

    println!("{}", "Starting DevInsight: Real-time Android Log Analyzer...".cyan().bold());

    // Clear logs if requested
    if cli.clear {
        // Clear logs using separate command
        Command::new("adb")
            .args(["logcat", "-c"])
            .output()
            .map_err(|_| DevInsightError::AdbNotFound)?;
        println!("{}", "Logs cleared.".green().bold());
    }

    // Build the adb command for monitoring
    let mut adb_command = Command::new("adb");
    adb_command.arg("logcat");

    // Add buffer selection - capture all buffers by default
    adb_command.args(&["-b", "all"]);

    // Add format selection
    adb_command.arg("-v").arg(&cli.format);

    // Print the command we're running (for debugging)
    println!("{}", "Running command:".cyan().bold());
    println!("{:?}", adb_command);

    // First check if adb is available
    if Command::new("adb").arg("devices").output().is_err() {
        return Err(DevInsightError::AdbNotFound);
    }

    let process = adb_command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())  // Capture stderr too
        .spawn()
        .map_err(|_| DevInsightError::AdbNotFound)?;

    let stdout = process.stdout
        .ok_or(DevInsightError::LogcatCaptureFailed("Failed to capture stdout".to_string()))?;
    let reader = BufReader::new(stdout);

    // Print command info
    println!("{}", "Log Settings:".yellow().bold());
    println!("Buffer: All buffers");  // Changed from cli.buffer since we're using all
    println!("Format: {}", cli.format.blue());
    if let Some(f) = &cli.filter {
        println!("Filter Level: {}", f.blue());
    }
    if let Some(t) = &cli.tag {
        println!("Tag Filter: {}", t.blue());
    }
    println!("{}", "=".repeat(50).yellow());

    // Add a startup message to verify logging is working
    Command::new("adb")
        .args(["shell", "log", "-p", "i", "-t", "DevInsight", "Log monitoring started"])
        .output()
        .ok();

    // Initialize storage if needed
    let mut storage = if cli.save {
        Some(LogStorage::new(
            cli.save_path.clone(),
            cli.max_size,
            None // No storage updates needed in standard mode
        ).map_err(|e| DevInsightError::StorageError(e.to_string()))?)
    } else {
        None
    };

    for line in reader.lines() {
        match line {
            Ok(log) => {
                if processor.should_process_log(&log) {
                    // Store log if storage is enabled
                    if let Some(storage) = &mut storage {
                        let entry = parse_log_entry(&log);
                        let stored_log = StoredLog {
                            timestamp: Local::now(),
                            level: entry.level.as_str().to_string(),
                            tag: entry.tag.clone(),
                            message: entry.message.clone(),
                            device_id: None,
                        };
                        storage.store_log(stored_log).ok();
                    }
                    println!("{}", processor.format_log(&log));
                }
            }
            Err(e) => {
                println!("{}", format!("Error reading log: {}", e).red().bold());
                break;
            }
        }
    }

    Ok(())
}
