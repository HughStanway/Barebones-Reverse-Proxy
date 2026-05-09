use chrono::Local;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::RwLock;

static LOG_FILE: RwLock<Option<File>> = RwLock::new(None);

pub fn update_log_file(path: Option<&str>) {
    if let Some(p) = path {
        if let Some(parent) = std::path::Path::new(p).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .expect("Failed to open log file");
        *LOG_FILE.write().expect("Failed to acquire log file lock") = Some(file);
    } else {
        *LOG_FILE.write().expect("Failed to acquire log file lock") = None;
    }
}

pub enum LogLevel {
    INFO,
    ERROR,
    DEBUG,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::INFO => write!(f, "INFO"),
            LogLevel::ERROR => write!(f, "ERROR"),
            LogLevel::DEBUG => write!(f, "DEBUG"),
        }
    }
}

pub fn log_event(level: LogLevel, event: &str, kv: &[(&str, String)]) {
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
    let thread_name = std::thread::current()
        .name()
        .unwrap_or("unknown-thread")
        .to_string();

    let mut kv_str = String::new();
    for (k, v) in kv {
        kv_str.push_str(&format!(" {}={}", k, v));
    }

    let log_line = format!(
        "[{}] [{}] [{}] event={}{}",
        timestamp, level, thread_name, event, kv_str
    );

    println!("{}", log_line);

    if let Ok(mut guard) = LOG_FILE.write() {
        if let Some(file) = guard.as_mut() {
            let _ = writeln!(file, "{}", log_line);
        }
    }
}

#[macro_export]
macro_rules! log_info {
    ($event:expr $(, $k:expr => $v:expr)*) => {
        $crate::log::log_event($crate::log::LogLevel::INFO, $event, &[$(($k, $v.to_string())),*]);
    };
}

#[macro_export]
macro_rules! log_error {
    ($event:expr $(, $k:expr => $v:expr)*) => {
        $crate::log::log_event($crate::log::LogLevel::ERROR, $event, &[$(($k, $v.to_string())),*]);
    };
}
