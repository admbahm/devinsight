# DevInsight

DevInsight is a Rust-powered Android `adb logcat` viewer for developers who want fast local log triage from a terminal. It supports a standard streaming CLI mode and an interactive TUI mode with filtering, search, stats, clipboard copy, and optional JSONL log storage.

## Prerequisites

- Rust and Cargo
- Android Debug Bridge (`adb`)
- A connected Android device or running emulator

## Installation

```bash
cargo install --path .
```

## Quick Start

```bash
# Standard streaming mode
devinsight

# Interactive TUI mode
devinsight -i

# Filter by log level and tag
devinsight --filter E --tag MyApp
devinsight -i --filter E --tag MyApp

# Select an Android logcat buffer
devinsight --buffer main
devinsight --buffer system
devinsight --buffer crash

# Show logs since a timestamp or count accepted by adb logcat -T
devinsight --since "2024-03-20 10:00:00"
devinsight -i --since 50
```

## Storage

DevInsight can save streamed logs as JSONL files and load them later.

```bash
# Save logs to ./logs/logcat_YYYYMMDD_HHMMSS.jsonl
devinsight --save

# Save logs from TUI mode
devinsight -i --save

# Use a custom save directory and rotation size in MB
devinsight --save --save-path /tmp/devinsight-logs --max-size 200

# Load a saved JSONL file instead of spawning adb
devinsight --load /tmp/devinsight-logs/logcat_20240321_143022.jsonl
devinsight -i --load /tmp/devinsight-logs/logcat_20240321_143022.jsonl
```

Stored JSONL fields are kept stable:

- `timestamp`
- `level`
- `tag`
- `message`
- `device_id`

## TUI Controls

| Key | Action |
| --- | --- |
| `1` / `2` / `3` | Logs, stats, storage views |
| `Space` | Pause or resume intake |
| `t` | Toggle tail mode |
| `/` | Search visible logs |
| `e` / `w` / `i` / `d` / `v` | Toggle level filters |
| `Up` / `Down` | Scroll |
| `PageUp` / `PageDown` | Scroll faster |
| `Home` / `g` | Jump to first log |
| `End` / `G` | Jump to latest log |
| `y` or `c` | Copy selected log |
| `n` | Toggle error notifications when built with the `macos` feature |
| `q` | Quit |

## Command Line Options

| Option | Short | Description |
| --- | --- | --- |
| `--filter` | `-f` | Filter logs by level (`E`, `W`, `I`, `D`, `V`) |
| `--tag` | `-t` | Filter logs by tag substring |
| `--clear` | `-c` | Clear logcat before streaming |
| `--since` | `-T` | Pass a timestamp or count to `adb logcat -T` |
| `--buffer` | `-b` | Select `main`, `system`, or `crash` buffer |
| `--format` | `-v` | Standard-mode logcat format |
| `--interactive` | `-i` | Use the interactive TUI |
| `--save` | | Save streamed logs to JSONL |
| `--save-path` | | Directory for saved logs |
| `--max-size` | | Rotation size in MB |
| `--load` | | Load a saved JSONL file instead of spawning `adb` |

## Development

```bash
cargo test
cargo run -- -i --filter E --tag TestApp --buffer main
```

This project currently focuses on Android logcat only. Broader sources such as iOS syslogs, Docker logs, and cloud logging are intentionally out of scope for the current reliability pass.
