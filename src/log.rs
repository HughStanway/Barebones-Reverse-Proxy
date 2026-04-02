use chrono::Local;

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

    println!(
        "[{}] [{}] [{}] event={}{}",
        timestamp, level, thread_name, event, kv_str
    );
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
