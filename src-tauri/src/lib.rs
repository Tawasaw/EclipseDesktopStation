pub mod debug_log;
pub mod gamepad;
pub mod hub_check;
mod robocol;
mod xml_config;

use debug_log::SessionLog;
use gamepad::{GamepadManager, GamepadStatus};
use robocol::{RobocolClient, RobotSnapshot};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::State;
use xml_config::XmlValidation;

/// The Driver Station's send loop runs every 40 ms.
const GAMEPAD_SEND_INTERVAL: Duration = Duration::from_millis(40);

struct AppState {
    client: Arc<Mutex<Option<Arc<RobocolClient>>>>,
    log: Arc<SessionLog>,
    gamepads: Arc<GamepadManager>,
}

impl AppState {
    fn new() -> Self {
        // Logging is disabled by default; the user opts in from the UI, at
        // which point a session file is created.
        let log = Arc::new(SessionLog::new());
        let state = Self {
            client: Arc::new(Mutex::new(None)),
            gamepads: GamepadManager::start(Arc::clone(&log)),
            log,
        };
        state.spawn_gamepad_sender();
        state
    }

    /// Forward gamepad input to the connected Robot Controller.
    fn spawn_gamepad_sender(&self) {
        let client = Arc::clone(&self.client);
        let gamepads = Arc::clone(&self.gamepads);
        thread::spawn(move || loop {
            let payloads = gamepads.take_transmissions();
            if !payloads.is_empty() {
                let client = client.lock().expect("client lock").clone();
                if let Some(client) = client {
                    for payload in payloads {
                        let _ = client.send_gamepad(&payload);
                    }
                }
            }
            thread::sleep(GAMEPAD_SEND_INTERVAL);
        });
    }
}

#[derive(Debug, Serialize)]
struct DownloadResult {
    path: String,
}

#[derive(Debug, Serialize)]
struct UploadResult {
    validation: XmlValidation,
}

#[derive(Debug, Serialize)]
struct ActivateResult {
    active_config: Option<robocol::RobotConfigFile>,
}

#[tauri::command]
fn connect_robot(state: State<'_, AppState>, ip: String) -> Result<RobotSnapshot, String> {
    state
        .log
        .record("ui_connect", serde_json::json!({ "ip": ip.trim() }));
    let client =
        RobocolClient::connect(ip.trim(), Arc::clone(&state.log)).map_err(|err| err.to_string())?;
    let snapshot = client.snapshot();
    let mut guard = state.client.lock().expect("client lock");
    if let Some(previous) = guard.take() {
        previous.disconnect();
    }
    *guard = Some(client);
    Ok(snapshot)
}

#[tauri::command]
fn disconnect_robot(state: State<'_, AppState>) -> Result<(), String> {
    state.log.record("ui_disconnect", serde_json::json!({}));
    let mut guard = state.client.lock().expect("client lock");
    if let Some(client) = guard.take() {
        client.disconnect();
    }
    Ok(())
}

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> Result<RobotSnapshot, String> {
    let guard = state.client.lock().expect("client lock");
    let client = guard.as_ref().ok_or_else(|| "not connected".to_string())?;
    Ok(client.snapshot())
}

#[tauri::command]
fn refresh_robot(state: State<'_, AppState>) -> Result<(), String> {
    state.log.record("ui_refresh", serde_json::json!({}));
    with_client(&state, |client| {
        client.refresh_metadata();
        Ok(())
    })
}

#[tauri::command]
fn init_op_mode(state: State<'_, AppState>, name: String) -> Result<(), String> {
    state
        .log
        .record("ui_init_op_mode", serde_json::json!({ "name": &name }));
    with_client(&state, |client| {
        client.init_op_mode(&name).map_err(|err| err.to_string())
    })
}

#[tauri::command]
fn run_op_mode(state: State<'_, AppState>, name: String) -> Result<(), String> {
    state
        .log
        .record("ui_run_op_mode", serde_json::json!({ "name": &name }));
    with_client(&state, |client| {
        client.run_op_mode(&name).map_err(|err| err.to_string())
    })
}

#[tauri::command]
fn stop_op_mode(state: State<'_, AppState>) -> Result<(), String> {
    state.log.record("ui_stop_op_mode", serde_json::json!({}));
    with_client(&state, |client| {
        client.stop_op_mode().map_err(|err| err.to_string())
    })
}

#[tauri::command]
fn validate_config_xml(xml: String) -> Result<XmlValidation, String> {
    let result = xml_config::validate_robot_xml(&xml).map_err(|err| err.to_string())?;
    Ok(result)
}

#[tauri::command]
fn download_config_xml(
    state: State<'_, AppState>,
    config_name: String,
) -> Result<DownloadResult, String> {
    state.log.record(
        "ui_download_config_xml",
        serde_json::json!({ "config_name": &config_name }),
    );
    with_client(&state, |client| {
        let xml = client
            .download_config_xml(&config_name)
            .map_err(|err| err.to_string())?;
        let path = xml_config::save_config_to_downloads(&config_name, &xml)
            .map_err(|err| err.to_string())?;
        Ok(DownloadResult {
            path: path.display().to_string(),
        })
    })
}

#[tauri::command]
fn upload_config_xml(
    state: State<'_, AppState>,
    config_name: String,
    xml: String,
) -> Result<UploadResult, String> {
    state.log.record(
        "ui_upload_config_xml",
        serde_json::json!({ "config_name": &config_name, "xml_bytes": xml.len() }),
    );
    let validation = xml_config::validate_robot_xml(&xml).map_err(|err| err.to_string())?;
    with_client(&state, |client| {
        client
            .save_config_xml(&config_name, &xml)
            .map_err(|err| err.to_string())?;
        Ok(UploadResult { validation })
    })
}

#[tauri::command]
fn activate_config(
    state: State<'_, AppState>,
    config_name: String,
) -> Result<ActivateResult, String> {
    state.log.record(
        "ui_activate_config",
        serde_json::json!({ "config_name": &config_name }),
    );
    with_client(&state, |client| {
        let snapshot = client
            .activate_config(&config_name)
            .map_err(|err| err.to_string())?;
        Ok(ActivateResult {
            active_config: snapshot.active_config,
        })
    })
}

#[tauri::command]
fn delete_config(state: State<'_, AppState>, config_name: String) -> Result<(), String> {
    state.log.record(
        "ui_delete_config",
        serde_json::json!({ "config_name": &config_name }),
    );
    with_client(&state, |client| {
        client
            .delete_config(&config_name)
            .map_err(|err| err.to_string())
    })
}

#[tauri::command]
fn clear_robot_message(state: State<'_, AppState>) -> Result<(), String> {
    with_client(&state, |client| {
        client.clear_robot_messages();
        Ok(())
    })
}

#[tauri::command]
fn get_gamepads(state: State<'_, AppState>) -> GamepadStatus {
    state.gamepads.status()
}

#[tauri::command]
fn set_gamepad_user(
    state: State<'_, AppState>,
    id: i32,
    user: Option<u8>,
) -> Result<GamepadStatus, String> {
    state.log.record(
        "ui_set_gamepad_user",
        serde_json::json!({ "id": id, "user": user }),
    );
    state.gamepads.set_user(id, user)?;
    Ok(state.gamepads.status())
}

#[tauri::command]
fn get_log_path(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state.log.path_string())
}

#[tauri::command]
fn set_logging(state: State<'_, AppState>, enabled: bool) -> Result<String, String> {
    if enabled {
        let path = state.log.enable();
        state.log.record("logging_enabled", serde_json::json!({}));
        Ok(path)
    } else {
        // Record before disabling so the final event lands in the session file.
        state.log.record("logging_disabled", serde_json::json!({}));
        state.log.disable();
        Ok(state.log.path_string())
    }
}

#[tauri::command]
fn get_app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

fn with_client<T>(
    state: &State<'_, AppState>,
    action: impl FnOnce(&RobocolClient) -> Result<T, String>,
) -> Result<T, String> {
    let guard = state.client.lock().expect("client lock");
    let client = guard.as_ref().ok_or_else(|| "not connected".to_string())?;
    action(client)
}

pub fn run() {
    tauri::Builder::default()
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            connect_robot,
            disconnect_robot,
            get_snapshot,
            refresh_robot,
            init_op_mode,
            run_op_mode,
            stop_op_mode,
            validate_config_xml,
            download_config_xml,
            upload_config_xml,
            activate_config,
            delete_config,
            clear_robot_message,
            get_gamepads,
            set_gamepad_user,
            get_log_path,
            set_logging,
            get_app_version
        ])
        .run(tauri::generate_context!())
        .expect("error while running EclipseDesktopStation");
}
