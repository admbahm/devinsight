use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::fs::{create_dir_all, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

#[derive(Serialize, Deserialize)]
pub struct StoredLog {
    pub timestamp: DateTime<Local>,
    pub level: String,
    pub tag: String,
    pub message: String,
    pub device_id: Option<String>,
}

pub struct StorageUpdate {
    pub current_file: String,
    pub total_size: u64,
    pub file_count: usize,
}

pub struct LogStorage {
    current_file: File,
    current_path: PathBuf,
    base_path: PathBuf,
    max_size: u64,
    current_size: u64,
    storage_tx: Option<Sender<StorageUpdate>>,
}

impl LogStorage {
    pub fn new(
        base_path: PathBuf,
        max_size: u64,
        tx: Option<Sender<StorageUpdate>>,
    ) -> io::Result<Self> {
        create_dir_all(&base_path)?;
        let file_path = Self::generate_filename(&base_path);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)?;
        let current_size = file.metadata()?.len();

        // Send initial storage info
        if let Some(tx) = &tx {
            let update = StorageUpdate {
                current_file: file_path.to_string_lossy().to_string(),
                total_size: Self::get_directory_size(&base_path)?,
                file_count: Self::count_log_files(&base_path)?,
            };
            tx.send(update).ok();
        }

        Ok(Self {
            current_file: file,
            current_path: file_path,
            base_path,
            max_size,
            current_size,
            storage_tx: tx,
        })
    }

    fn generate_filename(base_path: &PathBuf) -> PathBuf {
        let timestamp = Local::now().format("%Y%m%d_%H%M%S");
        let base_name = format!("logcat_{}", timestamp);
        let first_candidate = base_path.join(format!("{}.jsonl", base_name));
        if !first_candidate.exists() {
            return first_candidate;
        }

        for suffix in 1.. {
            let candidate = base_path.join(format!("{}_{}.jsonl", base_name, suffix));
            if !candidate.exists() {
                return candidate;
            }
        }

        unreachable!("unbounded suffix search should always return a filename")
    }

    fn get_directory_size(path: &PathBuf) -> io::Result<u64> {
        let mut total_size = 0;
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                total_size += entry.metadata()?.len();
            }
        }
        Ok(total_size)
    }

    fn count_log_files(path: &PathBuf) -> io::Result<usize> {
        let count = std::fs::read_dir(path)?
            .filter(|entry| {
                entry
                    .as_ref()
                    .map(|e| e.path().extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
                    .unwrap_or(false)
            })
            .count();
        Ok(count)
    }

    fn send_storage_update(&self) -> io::Result<()> {
        if let Some(tx) = &self.storage_tx {
            let update = StorageUpdate {
                current_file: self.current_path.to_string_lossy().to_string(),
                total_size: Self::get_directory_size(&self.base_path)?,
                file_count: Self::count_log_files(&self.base_path)?,
            };
            tx.send(update).ok();
        }
        Ok(())
    }

    pub fn store_log(&mut self, log: StoredLog) -> io::Result<()> {
        let log_json = serde_json::to_string(&log)?;
        self.current_file.write_all(log_json.as_bytes())?;
        self.current_file.write_all(b"\n")?;

        self.current_size += log_json.len() as u64 + 1;
        if self.current_size >= self.max_size * 1024 * 1024 {
            self.rotate_log()?;
        }

        self.send_storage_update()?;
        Ok(())
    }

    fn rotate_log(&mut self) -> io::Result<()> {
        let new_file_path = Self::generate_filename(&self.base_path);
        let new_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&new_file_path)?;

        self.current_file = new_file;
        self.current_path = new_file_path;
        self.current_size = 0;
        self.send_storage_update()?;
        Ok(())
    }

    pub fn load_logs_from_file(path: &Path) -> io::Result<Vec<StoredLog>> {
        let reader = BufReader::new(File::open(path)?);
        let mut logs = Vec::new();

        for line in reader.lines() {
            let log_str = line?;
            let log = serde_json::from_str::<StoredLog>(&log_str)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            logs.push(log);
        }

        Ok(logs)
    }

    #[allow(dead_code)]
    pub fn query_logs(
        &self,
        start_time: DateTime<Local>,
        end_time: DateTime<Local>,
    ) -> io::Result<Vec<StoredLog>> {
        let mut logs = Vec::new();
        for entry in std::fs::read_dir(&self.base_path)? {
            let entry = entry?;
            let reader = BufReader::new(File::open(entry.path())?);
            for line in reader.lines() {
                if let Ok(log_str) = line {
                    if let Ok(log) = serde_json::from_str::<StoredLog>(&log_str) {
                        if log.timestamp >= start_time && log.timestamp <= end_time {
                            logs.push(log);
                        }
                    }
                }
            }
        }
        Ok(logs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_log_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("devinsight_{}_{}", name, nanos))
    }

    fn stored_log(message: &str) -> StoredLog {
        StoredLog {
            timestamp: Local::now(),
            level: "INFO".to_string(),
            tag: "TestApp".to_string(),
            message: message.to_string(),
            device_id: None,
        }
    }

    #[test]
    fn stores_and_loads_jsonl_logs() {
        let dir = temp_log_dir("round_trip");
        let mut storage = LogStorage::new(dir.clone(), 100, None).unwrap();

        storage.store_log(stored_log("hello")).unwrap();
        let current_path = storage.current_path.clone();
        drop(storage);

        let loaded = LogStorage::load_logs_from_file(&current_path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].message, "hello");

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rotates_when_size_limit_is_reached() {
        let dir = temp_log_dir("rotation");
        let mut storage = LogStorage::new(dir.clone(), 1, None).unwrap();
        let initial_path = storage.current_path.clone();

        for i in 0..200 {
            storage
                .store_log(stored_log(&"x".repeat(10_000 + i)))
                .unwrap();
            if storage.current_path != initial_path {
                break;
            }
        }

        assert_ne!(storage.current_path, initial_path);
        assert!(LogStorage::count_log_files(&dir).unwrap() >= 2);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn storage_update_reports_active_file() {
        let dir = temp_log_dir("updates");
        let (tx, rx) = std::sync::mpsc::channel();
        let mut storage = LogStorage::new(dir.clone(), 100, Some(tx)).unwrap();
        let initial_update = rx.recv().unwrap();
        assert_eq!(
            initial_update.current_file,
            storage.current_path.to_string_lossy()
        );

        storage.store_log(stored_log("hello")).unwrap();
        let update = rx.recv().unwrap();
        assert_eq!(update.current_file, storage.current_path.to_string_lossy());

        std::fs::remove_dir_all(dir).unwrap();
    }
}
