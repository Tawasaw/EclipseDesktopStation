//! Local automation reuses the GUI's Robocol session. The Unix socket is owner-only.
use crate::debug_log::SessionLog;
use crate::robocol::{RobocolClient, RobotSnapshot, DEFAULT_OP_MODE_NAME};
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

type ClientState = Arc<Mutex<Option<Arc<RobocolClient>>>>;

#[derive(Deserialize)]
struct Request {
    action: String,
    host: Option<String>,
    opmode: Option<String>,
}

pub fn socket_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is unavailable")?;
    #[cfg(target_os = "macos")]
    let base = PathBuf::from(home).join("Library/Application Support/EclipseDesktopStation");
    #[cfg(not(target_os = "macos"))]
    let base = PathBuf::from(home).join(".local/share/EclipseDesktopStation");
    Ok(base.join("automation.sock"))
}

pub fn start(client: ClientState, log: Arc<SessionLog>) -> Result<(), String> {
    let path = socket_path()?;
    let parent = path.parent().ok_or("invalid automation path")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    if path.exists() {
        if UnixStream::connect(&path).is_ok() {
            return Err("automation is already listening in another app instance".into());
        }
        if !fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_socket()
        {
            return Err("automation path exists and is not a socket".into());
        }
        fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    let listener = UnixListener::bind(&path).map_err(|e| e.to_string())?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    log.record("automation_listening", json!({"path": path}));
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let client = Arc::clone(&client);
                    let log = Arc::clone(&log);
                    thread::spawn(move || serve(stream, client, log));
                }
                Err(error) => log.record(
                    "automation_accept_error",
                    json!({"error": error.to_string()}),
                ),
            }
        }
    });
    Ok(())
}

fn validated_opmode<'a>(request: &'a Request, snapshot: &RobotSnapshot) -> Result<&'a str, String> {
    let name = request
        .opmode
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .ok_or("init/start requires a nonempty opmode name")?;
    if name == DEFAULT_OP_MODE_NAME {
        return Err("use the stop action for the default stop OpMode".into());
    }
    if !snapshot.op_modes.iter().any(|mode| mode.name == name) {
        return Err(format!(
            "unknown OpMode '{name}'; wait for it in status.op_modes"
        ));
    }
    if request.action == "start" && snapshot.active_op_mode.as_deref() != Some(name) {
        return Err(format!("OpMode '{name}' has not acknowledged INIT"));
    }
    Ok(name)
}

fn dispatch(
    request: Request,
    state: &ClientState,
    log: &Arc<SessionLog>,
) -> Result<RobotSnapshot, String> {
    if !matches!(
        request.action.as_str(),
        "status" | "connect" | "init" | "start" | "stop"
    ) {
        return Err("unknown automation action".into());
    }
    let mut guard = state.lock().map_err(|_| "client lock failed")?;
    if request.action == "connect" {
        let host = request.host.ok_or("connect requires host")?;
        let expected = format!("{}:20884", host.trim());
        if let Some(existing) = guard.as_ref() {
            if existing.snapshot().peer.as_deref() == Some(expected.as_str()) {
                existing.refresh_metadata();
                return Ok(existing.snapshot());
            }
            return Err(
                "app is already connected to a different robot; disconnect it first".into(),
            );
        }
        *guard =
            Some(RobocolClient::connect(host.trim(), Arc::clone(log)).map_err(|e| e.to_string())?);
    } else if request.action != "status" {
        let client = guard.as_ref().ok_or("not connected")?;
        if client.snapshot().peer_conflict {
            return Err("another Driver Station owns the robot connection".into());
        }
        match request.action.as_str() {
            "init" => {
                let snapshot = client.snapshot();
                client.init_op_mode(validated_opmode(&request, &snapshot)?)
            }
            "start" => {
                let snapshot = client.snapshot();
                client.run_op_mode(validated_opmode(&request, &snapshot)?)
            }
            "stop" => client.stop_op_mode(),
            _ => return Err("unknown automation action".into()),
        }
        .map_err(|e| e.to_string())?;
        log.record("automation_action", json!({"action": request.action}));
    }
    Ok(guard
        .as_ref()
        .map(|client| client.snapshot())
        .unwrap_or_default())
}

fn serve(mut stream: UnixStream, state: ClientState, log: Arc<SessionLog>) {
    let response = (|| -> Result<Value, String> {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|e| e.to_string())?;
        let read_stream = stream.try_clone().map_err(|e| e.to_string())?;
        let mut line = String::new();
        BufReader::new(read_stream)
            .take(8192)
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        if line.len() >= 8192 || !line.ends_with('\n') {
            return Err("request must be one JSON line under 8192 bytes".into());
        }
        let request: Request = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        let snapshot = dispatch(request, &state, &log)?;
        Ok(json!({"ok": true, "protocol_version": 1, "snapshot": snapshot}))
    })()
    .unwrap_or_else(|error| json!({"ok": false, "error": error}));
    let _ = writeln!(stream, "{response}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(value: Value) -> Value {
        let (mut caller, server) = UnixStream::pair().unwrap();
        let state = Arc::new(Mutex::new(None));
        let worker = thread::spawn(move || serve(server, state, Arc::new(SessionLog::new())));
        writeln!(caller, "{value}").unwrap();
        let mut line = String::new();
        BufReader::new(caller).read_line(&mut line).unwrap();
        worker.join().unwrap();
        serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn disconnected_status_is_readable() {
        let response = request(json!({"action": "status"}));
        assert_eq!(response["ok"], true);
        assert_eq!(response["protocol_version"], 1);
        assert_eq!(response["snapshot"]["connected"], false);
    }

    fn example_snapshot() -> RobotSnapshot {
        let mut snapshot = RobotSnapshot::default();
        snapshot.op_modes = vec![serde_json::from_value(json!({"name": "ExampleTeleOp"})).unwrap()];
        snapshot
    }

    #[test]
    fn accepts_registered_opmodes_and_requires_matching_init_before_start() {
        let mut snapshot = example_snapshot();
        let init: Request =
            serde_json::from_value(json!({"action": "init", "opmode": "ExampleTeleOp"})).unwrap();
        assert_eq!(validated_opmode(&init, &snapshot).unwrap(), "ExampleTeleOp");
        let start: Request =
            serde_json::from_value(json!({"action": "start", "opmode": "ExampleTeleOp"})).unwrap();
        assert!(validated_opmode(&start, &snapshot)
            .unwrap_err()
            .contains("acknowledged INIT"));
        snapshot.active_op_mode = Some("ExampleTeleOp".into());
        assert_eq!(
            validated_opmode(&start, &snapshot).unwrap(),
            "ExampleTeleOp"
        );
    }

    #[test]
    fn rejects_missing_unknown_and_default_opmode_names() {
        let snapshot = example_snapshot();
        for name in ["", " ", "UnknownOpMode", DEFAULT_OP_MODE_NAME] {
            let init: Request =
                serde_json::from_value(json!({"action": "init", "opmode": name})).unwrap();
            assert!(validated_opmode(&init, &snapshot).is_err());
        }
        assert_eq!(request(json!({"action": "invalid"}))["ok"], false);
    }

    #[test]
    fn cannot_start_without_connection() {
        assert_eq!(
            request(json!({"action": "start", "opmode": "ExampleTeleOp"}))["ok"],
            false
        );
    }
}
