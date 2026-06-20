use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Per-session JSONL log. Logging is best-effort: if the log directory or file
/// cannot be created the app still runs normally, with logging disabled.
pub struct SessionLog {
    path: Option<PathBuf>,
    file: Mutex<Option<File>>,
}

impl SessionLog {
    pub fn new() -> Self {
        match Self::open() {
            Some((path, file)) => Self {
                path: Some(path),
                file: Mutex::new(Some(file)),
            },
            None => Self {
                path: None,
                file: Mutex::new(None),
            },
        }
    }

    fn open() -> Option<(PathBuf, File)> {
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

    pub fn path_string(&self) -> String {
        match &self.path {
            Some(path) => path.display().to_string(),
            None => "(logging disabled)".to_string(),
        }
    }

    pub fn record(&self, event: &str, detail: Value) {
        let entry = json!({
            "ts_ms": now_ms(),
            "event": event,
            "detail": detail,
        });
        if let Ok(mut guard) = self.file.lock() {
            if let Some(file) = guard.as_mut() {
                let _ = writeln!(file, "{entry}");
            }
        }
    }
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
