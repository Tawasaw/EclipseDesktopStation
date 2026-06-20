use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Per-session JSONL log. Disabled by default — no file is created until the
/// user opts in via the UI. Logging is best-effort: if the log directory or
/// file cannot be created the app still runs normally, with logging disabled.
pub struct SessionLog {
    /// `Some` while logging is enabled, holding the open session file and its
    /// path. `None` when logging is disabled (the default).
    sink: Mutex<Option<LogSink>>,
}

struct LogSink {
    path: PathBuf,
    file: File,
}

impl SessionLog {
    /// Create a log in the disabled state. Call [`SessionLog::enable`] to start
    /// writing a session file.
    pub fn new() -> Self {
        Self {
            sink: Mutex::new(None),
        }
    }

    /// Start a fresh session file if logging is not already enabled. Returns the
    /// resulting path string (or a disabled/unavailable marker).
    pub fn enable(&self) -> String {
        let mut guard = self.sink.lock().expect("log lock");
        if guard.is_none() {
            if let Some((path, file)) = open_session_file() {
                *guard = Some(LogSink { path, file });
            }
        }
        match guard.as_ref() {
            Some(sink) => sink.path.display().to_string(),
            None => "(logging unavailable)".to_string(),
        }
    }

    /// Stop logging and close the current session file.
    pub fn disable(&self) {
        let mut guard = self.sink.lock().expect("log lock");
        *guard = None;
    }

    pub fn path_string(&self) -> String {
        let guard = self.sink.lock().expect("log lock");
        match guard.as_ref() {
            Some(sink) => sink.path.display().to_string(),
            None => "(logging disabled)".to_string(),
        }
    }

    pub fn record(&self, event: &str, detail: Value) {
        if let Ok(mut guard) = self.sink.lock() {
            if let Some(sink) = guard.as_mut() {
                let entry = json!({
                    "ts_ms": now_ms(),
                    "event": event,
                    "detail": detail,
                });
                let _ = writeln!(sink.file, "{entry}");
            }
        }
    }
}

fn open_session_file() -> Option<(PathBuf, File)> {
    let log_dir = log_dir()?;
    fs::create_dir_all(&log_dir).ok()?;
    let path = log_dir.join(format!("session-{}.jsonl", now_ms()));
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()?;
    Some((path, file))
}

/// Per-OS directory for session logs, outside the app bundle so logging works
/// in a packaged build.
fn log_dir() -> Option<PathBuf> {
    let app = "EclipseDesktopStation";

    #[cfg(target_os = "macos")]
    {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Logs").join(app));
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            return Some(PathBuf::from(local).join(app).join("logs"));
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(xdg) = std::env::var_os("XDG_STATE_HOME") {
            return Some(PathBuf::from(xdg).join(app).join("logs"));
        }
        if let Some(home) = std::env::var_os("HOME") {
            return Some(PathBuf::from(home).join(".local/state").join(app).join("logs"));
        }
    }

    #[allow(unreachable_code)]
    Some(std::env::temp_dir().join(app).join("logs"))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
