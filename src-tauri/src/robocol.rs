use crate::debug_log::SessionLog;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::net::{SocketAddr, UdpSocket};
use std::str;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub const ROBOCOL_PORT: u16 = 20884;
pub const ROBOCOL_VERSION: u8 = 124;
const DEFAULT_OP_MODE_NAME: &str = "$Stop$Robot$";
const ROBOT_BATTERY_KEY: &str = "$Robot$Battery$Level$";

const CMD_REQUEST_OP_MODE_LIST: &str = "CMD_REQUEST_OP_MODE_LIST";
const CMD_NOTIFY_OP_MODE_LIST: &str = "CMD_NOTIFY_OP_MODE_LIST";
const CMD_REQUEST_ACTIVE_CONFIG: &str = "CMD_REQUEST_ACTIVE_CONFIG";
const CMD_NOTIFY_ACTIVE_CONFIGURATION: &str = "CMD_NOTIFY_ACTIVE_CONFIGURATION";
const CMD_INIT_OP_MODE: &str = "CMD_INIT_OP_MODE";
const CMD_RUN_OP_MODE: &str = "CMD_RUN_OP_MODE";
const CMD_REQUEST_CONFIGURATIONS: &str = "CMD_REQUEST_CONFIGURATIONS";
const CMD_REQUEST_CONFIGURATIONS_RESP: &str = "CMD_REQUEST_CONFIGURATIONS_RESP";
const CMD_REQUEST_PARTICULAR_CONFIGURATION: &str = "CMD_REQUEST_PARTICULAR_CONFIGURATION";
const CMD_REQUEST_PARTICULAR_CONFIGURATION_RESP: &str = "CMD_REQUEST_PARTICULAR_CONFIGURATION_RESP";
const CMD_ACTIVATE_CONFIGURATION: &str = "CMD_ACTIVATE_CONFIGURATION";
const CMD_SAVE_CONFIGURATION: &str = "CMD_SAVE_CONFIGURATION";
const CMD_DELETE_CONFIGURATION: &str = "CMD_DELETE_CONFIGURATION";
const CMD_SHOW_STACKTRACE: &str = "CMD_SHOW_STACKTRACE";
const CMD_SHOW_TOAST: &str = "CMD_SHOW_TOAST";

#[derive(Debug, Error)]
pub enum RobocolError {
    #[error("invalid robot address")]
    InvalidAddress,
    #[error("network error: {0}")]
    Network(#[from] std::io::Error),
    #[error("timed out waiting for robot response")]
    Timeout,
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RobotSnapshot {
    pub connected: bool,
    pub peer: Option<String>,
    pub peer_conflict: bool,
    pub local_port: Option<u16>,
    pub last_packet_ms: Option<u128>,
    pub last_error: Option<String>,
    pub robot_state: String,
    pub active_config: Option<RobotConfigFile>,
    pub configs: Vec<RobotConfigFile>,
    pub op_modes: Vec<OpModeMeta>,
    pub selected_op_mode: Option<String>,
    pub telemetry: Vec<TelemetryLine>,
    pub robot_battery: Option<String>,
    pub robot_error: Option<String>,
    pub robot_notice: Option<String>,
    pub log_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TelemetryLine {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpModeMeta {
    pub name: String,
    #[serde(default)]
    pub flavor: String,
    #[serde(default)]
    pub group: String,
    #[serde(default, rename = "autoTransition")]
    pub auto_transition: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RobotConfigFile {
    pub name: String,
    #[serde(default, rename = "resourceId")]
    pub resource_id: i32,
    #[serde(default)]
    pub location: String,
    #[serde(default, rename = "isDirty")]
    pub is_dirty: bool,
}

impl RobotConfigFile {
    pub fn local(name: &str) -> Self {
        Self {
            name: strip_xml_extension(name),
            resource_id: 0,
            location: "LOCAL_STORAGE".to_string(),
            is_dirty: false,
        }
    }
}

pub struct RobocolClient {
    socket: Arc<UdpSocket>,
    peer: SocketAddr,
    sequence: AtomicU16,
    running: AtomicBool,
    snapshot: Arc<RwLock<RobotSnapshot>>,
    last_config_xml: Mutex<Option<(u64, String)>>,
    log: Arc<SessionLog>,
}

impl RobocolClient {
    pub fn connect(peer_ip: &str, log: Arc<SessionLog>) -> Result<Arc<Self>, RobocolError> {
        let peer_addr = format!("{peer_ip}:{ROBOCOL_PORT}")
            .parse::<SocketAddr>()
            .map_err(|_| RobocolError::InvalidAddress)?;
        let socket = bind_socket()?;
        socket.set_nonblocking(true)?;

        let local_port = socket.local_addr().ok().map(|addr| addr.port());
        log.record(
            "connect",
            json!({
                "peer": peer_addr.to_string(),
                "local_port": local_port,
            }),
        );
        let client = Arc::new(Self {
            socket: Arc::new(socket),
            peer: peer_addr,
            sequence: AtomicU16::new(0x4000),
            running: AtomicBool::new(true),
            snapshot: Arc::new(RwLock::new(RobotSnapshot {
                connected: true,
                peer: Some(peer_addr.to_string()),
                local_port,
                log_path: Some(log.path_string()),
                robot_state: "Unknown".to_string(),
                ..RobotSnapshot::default()
            })),
            last_config_xml: Mutex::new(None),
            log,
        });

        client.spawn_receive_loop();
        client.spawn_heartbeat_loop();
        client.send_peer_discovery()?;
        client.refresh_metadata();
        Ok(client)
    }

    pub fn snapshot(&self) -> RobotSnapshot {
        let mut snapshot = self.snapshot.read().expect("snapshot lock").clone();
        snapshot.connected = self.is_fresh();
        snapshot
    }

    pub fn disconnect(&self) {
        self.log
            .record("disconnect", json!({ "peer": self.peer.to_string() }));
        self.running.store(false, Ordering::Relaxed);
        if let Ok(mut snapshot) = self.snapshot.write() {
            snapshot.connected = false;
        }
    }

    pub fn refresh_metadata(&self) {
        self.log.record("refresh_metadata", json!({}));
        let _ = self.send_command(CMD_REQUEST_ACTIVE_CONFIG, "");
        let _ = self.send_command(CMD_REQUEST_OP_MODE_LIST, "");
        let _ = self.send_command(CMD_REQUEST_CONFIGURATIONS, "");
    }

    pub fn clear_robot_messages(&self) {
        if let Ok(mut snapshot) = self.snapshot.write() {
            snapshot.robot_error = None;
            snapshot.robot_notice = None;
        }
    }

    pub fn init_op_mode(&self, name: &str) -> Result<(), RobocolError> {
        self.log.record("init_op_mode", json!({ "name": name }));
        self.send_command(CMD_INIT_OP_MODE, name)?;
        if let Ok(mut snapshot) = self.snapshot.write() {
            snapshot.selected_op_mode = Some(name.to_string());
            snapshot.robot_error = None;
            snapshot.robot_notice = None;
        }
        Ok(())
    }

    pub fn run_op_mode(&self, name: &str) -> Result<(), RobocolError> {
        self.log.record("run_op_mode", json!({ "name": name }));
        self.send_command(CMD_RUN_OP_MODE, name)
    }

    pub fn stop_op_mode(&self) -> Result<(), RobocolError> {
        self.log
            .record("stop_op_mode", json!({ "name": DEFAULT_OP_MODE_NAME }));
        self.send_command(CMD_INIT_OP_MODE, DEFAULT_OP_MODE_NAME)
    }

    pub fn download_config_xml(&self, config_name: &str) -> Result<String, RobocolError> {
        self.log
            .record("download_config_xml", json!({ "config_name": config_name }));
        let config = self
            .snapshot
            .read()
            .expect("snapshot lock")
            .configs
            .iter()
            .find(|candidate| candidate.name.eq_ignore_ascii_case(config_name))
            .cloned()
            .unwrap_or_else(|| RobotConfigFile::local(config_name));
        let config_json = serde_json::to_string(&config)?;
        let start_seq = self
            .last_config_xml
            .lock()
            .expect("config xml lock")
            .as_ref()
            .map(|(seq, _)| *seq)
            .unwrap_or(0);

        self.send_command(CMD_REQUEST_PARTICULAR_CONFIGURATION, &config_json)?;

        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if let Some((seq, xml)) = self
                .last_config_xml
                .lock()
                .expect("config xml lock")
                .clone()
            {
                if seq > start_seq {
                    return Ok(xml);
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
        Err(RobocolError::Timeout)
    }

    /// Save (upload) a config to the Robot Controller. The RC activates any
    /// config it saves (FtcEventLoopBase.handleCommandSaveConfiguration calls
    /// setActiveConfigAndUpdateUI), so a successful upload also makes this the
    /// active config. The new config appears in the config list after the
    /// metadata refresh.
    pub fn save_config_xml(&self, config_name: &str, xml: &str) -> Result<(), RobocolError> {
        self.log.record(
            "save_config_xml",
            json!({
                "config_name": config_name,
                "xml_bytes": xml.len(),
            }),
        );
        let config = RobotConfigFile::local(config_name);
        let config_json = serde_json::to_string(&config)?;
        let payload = format!("{config_json};{xml}");
        self.send_command(CMD_SAVE_CONFIGURATION, &payload)?;
        thread::sleep(Duration::from_millis(250));
        self.refresh_metadata();
        Ok(())
    }

    /// Delete a config from the Robot Controller, selected by name.
    pub fn delete_config(&self, config_name: &str) -> Result<(), RobocolError> {
        self.log
            .record("delete_config", json!({ "config_name": config_name }));
        let config = self
            .snapshot
            .read()
            .expect("snapshot lock")
            .configs
            .iter()
            .find(|candidate| candidate.name.eq_ignore_ascii_case(config_name))
            .cloned()
            .unwrap_or_else(|| RobotConfigFile::local(config_name));
        let config_json = serde_json::to_string(&config)?;
        self.send_command(CMD_DELETE_CONFIGURATION, &config_json)?;
        thread::sleep(Duration::from_millis(150));
        self.refresh_metadata();
        Ok(())
    }

    /// Activate a config that already exists on the Robot Controller, selected
    /// by name. Uses the RC's own metadata for the config when available.
    pub fn activate_config(&self, config_name: &str) -> Result<RobotSnapshot, RobocolError> {
        self.log
            .record("activate_config", json!({ "config_name": config_name }));
        let config = self
            .snapshot
            .read()
            .expect("snapshot lock")
            .configs
            .iter()
            .find(|candidate| candidate.name.eq_ignore_ascii_case(config_name))
            .cloned()
            .unwrap_or_else(|| RobotConfigFile::local(config_name));
        let config_json = serde_json::to_string(&config)?;
        self.send_command(CMD_ACTIVATE_CONFIGURATION, &config_json)?;
        self.refresh_metadata();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            let snapshot = self.snapshot();
            if snapshot
                .active_config
                .as_ref()
                .map(|active| active.name.eq_ignore_ascii_case(config_name))
                .unwrap_or(false)
            {
                return Ok(snapshot);
            }
            thread::sleep(Duration::from_millis(100));
        }
        Ok(self.snapshot())
    }

    fn is_fresh(&self) -> bool {
        let snapshot = self.snapshot.read().expect("snapshot lock");
        snapshot
            .last_packet_ms
            .map(|last| now_ms().saturating_sub(last) < 4_000)
            .unwrap_or(false)
    }

    fn spawn_receive_loop(self: &Arc<Self>) {
        let client = Arc::clone(self);
        thread::spawn(move || {
            let mut buf = vec![0_u8; 65_520];
            while client.running.load(Ordering::Relaxed) {
                match client.socket.recv_from(&mut buf) {
                    Ok((len, addr)) => {
                        if addr.ip() == client.peer.ip() {
                            client.handle_datagram(&buf[..len]);
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(err) => {
                        client.set_error(err.to_string());
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            }
        });
    }

    fn spawn_heartbeat_loop(self: &Arc<Self>) {
        let client = Arc::clone(self);
        thread::spawn(move || {
            while client.running.load(Ordering::Relaxed) {
                let _ = client.send_peer_discovery();
                let _ = client.send_heartbeat();
                let _ = client.send_keepalive();
                thread::sleep(Duration::from_millis(500));
            }
        });
    }

    fn handle_datagram(&self, data: &[u8]) {
        if data.len() < 3 {
            return;
        }
        if let Ok(mut snapshot) = self.snapshot.write() {
            snapshot.last_packet_ms = Some(now_ms());
            snapshot.connected = true;
        }

        match data[0] {
            1 => {
                self.log
                    .record("rx_heartbeat", json!({ "bytes": data.len() }));
                self.handle_heartbeat(data);
            }
            3 => self.handle_peer_discovery(data),
            4 => self.handle_command(data),
            5 => self.handle_telemetry(data),
            other => self
                .log
                .record("rx_packet", json!({ "type": other, "bytes": data.len() })),
        }
    }

    fn handle_peer_discovery(&self, data: &[u8]) {
        // PeerDiscovery wire layout (FTC SDK 11.1):
        //   [0] msg type  [1..3] payload size  [3] robocol version
        //   [4] peer type  [5..7] sequence  [7..] sdk build/version
        // peer type 3 == NOT_CONNECTED_DUE_TO_PREEXISTING_CONNECTION: the Robot
        // Controller already has another Driver Station / Driver Hub connected.
        if data.len() < 5 {
            return;
        }
        let conflict = data[4] == 3;
        let changed = match self.snapshot.write() {
            Ok(mut snapshot) => {
                let changed = snapshot.peer_conflict != conflict;
                snapshot.peer_conflict = conflict;
                changed
            }
            Err(_) => return,
        };
        if changed {
            self.log.record(
                "peer_discovery",
                json!({ "peer_type": data[4], "conflict": conflict }),
            );
        }
    }

    fn handle_heartbeat(&self, data: &[u8]) {
        if data.len() < 14 {
            return;
        }
        let robot_state = robot_state_name(data[13] as i8);
        if let Ok(mut snapshot) = self.snapshot.write() {
            snapshot.robot_state = robot_state;
        }
    }

    fn handle_command(&self, data: &[u8]) {
        let Ok(command) = decode_command(data) else {
            self.log
                .record("decode_command_error", json!({ "bytes": data.len() }));
            return;
        };
        self.log.record(
            "rx_command",
            json!({
                "name": &command.name,
                "acknowledged": command.acknowledged,
                "extra_bytes": command.extra.len(),
            }),
        );
        if !command.acknowledged {
            let _ = self.send_command_ack(&command);
        }

        match command.name.as_str() {
            CMD_NOTIFY_ACTIVE_CONFIGURATION => {
                if let Ok(config) = serde_json::from_str::<RobotConfigFile>(&command.extra) {
                    self.log.record(
                        "active_config",
                        json!({ "name": config.name, "location": config.location }),
                    );
                    if let Ok(mut snapshot) = self.snapshot.write() {
                        snapshot.active_config = Some(config);
                    }
                }
            }
            CMD_NOTIFY_OP_MODE_LIST => {
                if let Ok(op_modes) = serde_json::from_str::<Vec<OpModeMeta>>(&command.extra) {
                    self.log
                        .record("op_mode_list", json!({ "count": op_modes.len() }));
                    if let Ok(mut snapshot) = self.snapshot.write() {
                        snapshot.op_modes = op_modes
                            .into_iter()
                            .filter(|mode| mode.flavor != "SYSTEM")
                            .collect();
                    }
                }
            }
            CMD_REQUEST_CONFIGURATIONS_RESP => {
                if let Ok(configs) = serde_json::from_str::<Vec<RobotConfigFile>>(&command.extra) {
                    self.log
                        .record("config_list", json!({ "count": configs.len() }));
                    if let Ok(mut snapshot) = self.snapshot.write() {
                        snapshot.configs = configs;
                    }
                }
            }
            CMD_REQUEST_PARTICULAR_CONFIGURATION_RESP => {
                let seq = now_ms() as u64;
                self.log.record(
                    "config_xml_received",
                    json!({ "xml_bytes": command.extra.len() }),
                );
                if let Ok(mut latest) = self.last_config_xml.lock() {
                    *latest = Some((seq, command.extra));
                }
            }
            CMD_SHOW_STACKTRACE => {
                self.log
                    .record("robot_stacktrace", json!({ "text": &command.extra }));
                if let Ok(mut snapshot) = self.snapshot.write() {
                    snapshot.robot_error = Some(command.extra);
                }
            }
            CMD_SHOW_TOAST => {
                let message = parse_toast_message(&command.extra);
                self.log.record("robot_toast", json!({ "text": &message }));
                if let Ok(mut snapshot) = self.snapshot.write() {
                    snapshot.robot_notice = Some(message);
                }
            }
            _ => {}
        }
    }

    fn handle_telemetry(&self, data: &[u8]) {
        let Ok(telemetry) = decode_telemetry(data) else {
            self.log
                .record("decode_telemetry_error", json!({ "bytes": data.len() }));
            return;
        };
        self.log.record(
            "rx_telemetry",
            json!({
                "entries": telemetry.iter().map(|(key, value)| {
                    json!({
                        "key": key,
                        "value": value.chars().take(160).collect::<String>(),
                    })
                }).collect::<Vec<_>>(),
            }),
        );
        let mut robot_battery = None;
        let lines = telemetry
            .into_iter()
            .filter_map(|(key, value)| {
                if key == ROBOT_BATTERY_KEY {
                    robot_battery = Some(value);
                    None
                } else if key == "$System$None$" && value.trim().is_empty() {
                    None
                } else if key.starts_with('\0') {
                    if value.trim().is_empty() {
                        None
                    } else {
                        Some(TelemetryLine {
                            key: String::new(),
                            value,
                        })
                    }
                } else if key.trim().is_empty() && value.trim().is_empty() {
                    None
                } else {
                    Some(TelemetryLine { key, value })
                }
            })
            .collect::<Vec<_>>();

        if let Ok(mut snapshot) = self.snapshot.write() {
            if let Some(value) = robot_battery {
                snapshot.robot_battery = Some(value);
            }
            if !lines.is_empty() {
                snapshot.telemetry = lines;
            }
        }
    }

    fn send_peer_discovery(&self) -> Result<(), RobocolError> {
        let seq = self.next_seq();
        let mut packet = Vec::with_capacity(13);
        packet.push(3);
        packet.extend_from_slice(&(10_u16).to_be_bytes());
        packet.push(ROBOCOL_VERSION);
        packet.push(1);
        packet.extend_from_slice(&seq.to_be_bytes());
        packet.push(12);
        packet.extend_from_slice(&(2025_u16).to_be_bytes());
        packet.push(11);
        packet.push(1);
        packet.push(0);
        self.send_raw(&packet)
    }

    fn send_heartbeat(&self) -> Result<(), RobocolError> {
        let mut payload = Vec::with_capacity(8 + 1 + 8 * 3 + 1 + 3);
        let wall_ms = now_ms() as i64;
        payload.extend_from_slice(&monotonic_nanos().to_be_bytes());
        payload.push(0);
        payload.extend_from_slice(&wall_ms.to_be_bytes());
        payload.extend_from_slice(&0_i64.to_be_bytes());
        payload.extend_from_slice(&0_i64.to_be_bytes());
        payload.push(3);
        payload.extend_from_slice(b"UTC");
        self.send_packet(1, &payload)
    }

    fn send_keepalive(&self) -> Result<(), RobocolError> {
        self.send_packet(6, &[0])
    }

    fn send_command(&self, name: &str, extra: &str) -> Result<(), RobocolError> {
        self.log.record(
            "tx_command",
            json!({
                "name": name,
                "extra_bytes": extra.len(),
            }),
        );
        let packet = encode_command(self.next_seq(), now_nanos(), false, name, extra);
        self.send_raw(&packet)
    }

    fn send_command_ack(&self, command: &CommandPacket) -> Result<(), RobocolError> {
        let packet = encode_command(self.next_seq(), command.timestamp, true, &command.name, "");
        self.send_raw(&packet)
    }

    fn send_packet(&self, msg_type: u8, payload: &[u8]) -> Result<(), RobocolError> {
        let mut packet = Vec::with_capacity(5 + payload.len());
        packet.push(msg_type);
        packet.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        packet.extend_from_slice(&self.next_seq().to_be_bytes());
        packet.extend_from_slice(payload);
        self.send_raw(&packet)
    }

    fn send_raw(&self, packet: &[u8]) -> Result<(), RobocolError> {
        self.socket.send_to(packet, self.peer)?;
        Ok(())
    }

    fn next_seq(&self) -> u16 {
        self.sequence.fetch_add(1, Ordering::Relaxed)
    }

    fn set_error(&self, message: String) {
        self.log.record("error", json!({ "message": message }));
        if let Ok(mut snapshot) = self.snapshot.write() {
            snapshot.last_error = Some(message);
        }
    }
}

fn bind_socket() -> Result<UdpSocket, RobocolError> {
    match UdpSocket::bind(("0.0.0.0", ROBOCOL_PORT)) {
        Ok(socket) => Ok(socket),
        Err(_) => Ok(UdpSocket::bind(("0.0.0.0", 0))?),
    }
}

#[derive(Debug)]
struct CommandPacket {
    timestamp: i64,
    acknowledged: bool,
    name: String,
    extra: String,
}

fn encode_command(
    seq: u16,
    timestamp: i64,
    acknowledged: bool,
    name: &str,
    extra: &str,
) -> Vec<u8> {
    let name_bytes = name.as_bytes();
    let extra_bytes = extra.as_bytes();
    let payload_len = if acknowledged {
        8 + 1 + 2 + name_bytes.len()
    } else {
        8 + 1 + 2 + name_bytes.len() + 2 + extra_bytes.len()
    };
    let mut packet = Vec::with_capacity(5 + payload_len);
    packet.push(4);
    packet.extend_from_slice(&(payload_len as u16).to_be_bytes());
    packet.extend_from_slice(&seq.to_be_bytes());
    packet.extend_from_slice(&timestamp.to_be_bytes());
    packet.push(if acknowledged { 1 } else { 0 });
    packet.extend_from_slice(&(name_bytes.len() as u16).to_be_bytes());
    packet.extend_from_slice(name_bytes);
    if !acknowledged {
        packet.extend_from_slice(&(extra_bytes.len() as u16).to_be_bytes());
        packet.extend_from_slice(extra_bytes);
    }
    packet
}

fn decode_command(data: &[u8]) -> Result<CommandPacket, RobocolError> {
    let mut offset = 5;
    let timestamp = read_i64(data, &mut offset)?;
    let acknowledged = read_u8(data, &mut offset)? != 0;
    let name_len = read_u16(data, &mut offset)? as usize;
    let name = read_string(data, &mut offset, name_len)?;
    let extra = if acknowledged {
        String::new()
    } else {
        let extra_len = read_u16(data, &mut offset)? as usize;
        read_string(data, &mut offset, extra_len)?
    };
    Ok(CommandPacket {
        timestamp,
        acknowledged,
        name,
        extra,
    })
}

fn decode_telemetry(data: &[u8]) -> Result<Vec<(String, String)>, RobocolError> {
    let mut offset = 5;
    let _timestamp = read_i64(data, &mut offset)?;
    let _sorted = read_u8(data, &mut offset)?;
    let _robot_state = read_u8(data, &mut offset)?;
    let tag_len = read_u8(data, &mut offset)? as usize;
    let _tag = read_string(data, &mut offset, tag_len)?;

    let mut result = Vec::new();
    let string_count = read_u8(data, &mut offset)? as usize;
    for _ in 0..string_count {
        let key_len = read_u16(data, &mut offset)? as usize;
        let key = read_string(data, &mut offset, key_len)?;
        let value_len = read_u16(data, &mut offset)? as usize;
        let value = read_string(data, &mut offset, value_len)?;
        result.push((key, value));
    }

    let number_count = read_u8(data, &mut offset)? as usize;
    for _ in 0..number_count {
        let key_len = read_u16(data, &mut offset)? as usize;
        let key = read_string(data, &mut offset, key_len)?;
        let value = read_f32(data, &mut offset)?;
        result.push((key, format!("{value:.3}")));
    }
    Ok(result)
}

fn read_u8(data: &[u8], offset: &mut usize) -> Result<u8, RobocolError> {
    if *offset >= data.len() {
        return Err(RobocolError::Protocol("short packet".to_string()));
    }
    let value = data[*offset];
    *offset += 1;
    Ok(value)
}

fn read_u16(data: &[u8], offset: &mut usize) -> Result<u16, RobocolError> {
    if *offset + 2 > data.len() {
        return Err(RobocolError::Protocol("short packet".to_string()));
    }
    let value = u16::from_be_bytes([data[*offset], data[*offset + 1]]);
    *offset += 2;
    Ok(value)
}

fn read_i64(data: &[u8], offset: &mut usize) -> Result<i64, RobocolError> {
    if *offset + 8 > data.len() {
        return Err(RobocolError::Protocol("short packet".to_string()));
    }
    let value = i64::from_be_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
        data[*offset + 4],
        data[*offset + 5],
        data[*offset + 6],
        data[*offset + 7],
    ]);
    *offset += 8;
    Ok(value)
}

fn read_f32(data: &[u8], offset: &mut usize) -> Result<f32, RobocolError> {
    if *offset + 4 > data.len() {
        return Err(RobocolError::Protocol("short packet".to_string()));
    }
    let value = f32::from_be_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
    ]);
    *offset += 4;
    Ok(value)
}

fn read_string(data: &[u8], offset: &mut usize, len: usize) -> Result<String, RobocolError> {
    if *offset + len > data.len() {
        return Err(RobocolError::Protocol("short packet".to_string()));
    }
    let value = str::from_utf8(&data[*offset..*offset + len])
        .map_err(|err| RobocolError::Protocol(err.to_string()))?
        .to_string();
    *offset += len;
    Ok(value)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn now_nanos() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(i64::MAX as u128) as i64
}

fn monotonic_nanos() -> i64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(Instant::now)
        .elapsed()
        .as_nanos()
        .min(i64::MAX as u128) as i64
}

fn robot_state_name(value: i8) -> String {
    match value {
        0 => "Not started",
        1 => "Init",
        2 => "Running",
        3 => "Stopped",
        4 => "Emergency stop",
        _ => "Unknown",
    }
    .to_string()
}

fn strip_xml_extension(name: &str) -> String {
    name.trim()
        .strip_suffix(".xml")
        .or_else(|| name.trim().strip_suffix(".XML"))
        .unwrap_or_else(|| name.trim())
        .to_string()
}

/// CMD_SHOW_TOAST carries a JSON `ShowToast` payload in newer SDKs and a plain
/// string in older ones. Extract the human-readable message either way.
fn parse_toast_message(extra: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(extra) {
        if let Some(message) = value.get("message").and_then(|m| m.as_str()) {
            return message.to_string();
        }
    }
    extra.trim().to_string()
}
