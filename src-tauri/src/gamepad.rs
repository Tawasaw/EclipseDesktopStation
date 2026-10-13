use crate::debug_log::SessionLog;
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Gamepad wire format, mirroring `Gamepad.toByteArray()` in FTC SDK 12.0
/// (ROBOCOL_GAMEPAD_VERSION 5, 60-byte payload).
const GAMEPAD_VERSION: u8 = 5;
const GAMEPAD_PAYLOAD_SIZE: usize = 60;
/// `Gamepad.Type` / `Gamepad.LegacyType` ordinals. The SDK 12.0 Driver Station
/// reports PS4-compatible gamepads as SONY_PS4 and everything else (F310
/// included) as XBOX_360.
const GAMEPAD_TYPE_XBOX_360: u8 = 2;
const GAMEPAD_TYPE_SONY_PS4: u8 = 3;
/// The DS stops resending a gamepad that has been at rest this long
/// (SendOnceRunnable.GAMEPAD_UPDATE_THRESHOLD).
const AT_REST_RESEND_MS: i64 = 1000;
/// How many zeroed packets to send for a user slot that loses its gamepad, so
/// the robot doesn't keep acting on the last stick position.
const NEUTRAL_REPEATS: u8 = 3;

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct GamepadState {
    pub left_stick_x: f32,
    pub left_stick_y: f32,
    pub right_stick_x: f32,
    pub right_stick_y: f32,
    pub left_trigger: f32,
    pub right_trigger: f32,
    pub dpad_up: bool,
    pub dpad_down: bool,
    pub dpad_left: bool,
    pub dpad_right: bool,
    pub a: bool,
    pub b: bool,
    pub x: bool,
    pub y: bool,
    pub guide: bool,
    pub start: bool,
    pub back: bool,
    pub left_bumper: bool,
    pub right_bumper: bool,
    pub left_stick_button: bool,
    pub right_stick_button: bool,
}

impl GamepadState {
    /// Same axes-only check as `Gamepad.atRest()`.
    fn at_rest(&self) -> bool {
        self.left_stick_x == 0.0
            && self.left_stick_y == 0.0
            && self.right_stick_x == 0.0
            && self.right_stick_y == 0.0
            && self.left_trigger == 0.0
            && self.right_trigger == 0.0
    }

    /// Button bitfield in `toByteArray()` order: right_bumper is bit 0, the
    /// (unused here) touchpad bits sit above left_stick_button.
    fn button_bits(&self) -> i32 {
        [
            self.left_stick_button,
            self.right_stick_button,
            self.dpad_up,
            self.dpad_down,
            self.dpad_left,
            self.dpad_right,
            self.a,
            self.b,
            self.x,
            self.y,
            self.guide,
            self.start,
            self.back,
            self.left_bumper,
            self.right_bumper,
        ]
        .iter()
        .fold(0, |bits, &pressed| (bits << 1) | pressed as i32)
    }
}

/// Encode a Gamepad payload (without the 5-byte Robocol header).
fn encode_gamepad(
    id: i32,
    timestamp_ms: i64,
    user: u8,
    gamepad_type: u8,
    state: &GamepadState,
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(GAMEPAD_PAYLOAD_SIZE);
    payload.push(GAMEPAD_VERSION);
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&timestamp_ms.to_be_bytes());
    for value in [
        state.left_stick_x,
        state.left_stick_y,
        state.right_stick_x,
        state.right_stick_y,
        state.left_trigger,
        state.right_trigger,
    ] {
        payload.extend_from_slice(&value.to_be_bytes());
    }
    payload.extend_from_slice(&state.button_bits().to_be_bytes());
    payload.push(user);
    // LegacyType and Type share ordinals for these two types.
    payload.push(gamepad_type);
    payload.push(gamepad_type);
    // Touchpad finger 1/2 x/y: not reported.
    payload.extend_from_slice(&[0; 16]);
    debug_assert_eq!(payload.len(), GAMEPAD_PAYLOAD_SIZE);
    payload
}

/// `MathUtil.cleanMotionValues`: zero inside the deadzone, clamp to [-1, 1],
/// and rescale so the output still spans the full range.
fn clean_motion_value(value: f32, deadzone: f32) -> f32 {
    if value < deadzone && value > -deadzone {
        0.0
    } else if value > 1.0 {
        1.0
    } else if value < -1.0 {
        -1.0
    } else if value > 0.0 {
        (value - deadzone) / (1.0 - deadzone)
    } else {
        (value + deadzone) / (1.0 - deadzone)
    }
}

/// Parse an XInput input report (20 bytes, type 0x00). Matches the DS's
/// XboxControlSurfacePacket: Y axes are inverted so pushing up is negative.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_xinput_report(report: &[u8], deadzone: f32) -> Option<GamepadState> {
    if report.len() < 14 || report[0] != 0x00 {
        return None;
    }
    let buttons = u16::from_be_bytes([report[2], report[3]]);
    let pressed = |mask: u16| buttons & mask != 0;
    let axis = |offset: usize| i16::from_le_bytes([report[offset], report[offset + 1]]) as f32;
    Some(GamepadState {
        dpad_up: pressed(0x0100),
        dpad_down: pressed(0x0200),
        dpad_left: pressed(0x0400),
        dpad_right: pressed(0x0800),
        start: pressed(0x1000),
        back: pressed(0x2000),
        left_stick_button: pressed(0x4000),
        right_stick_button: pressed(0x8000),
        left_bumper: pressed(0x0001),
        right_bumper: pressed(0x0002),
        guide: pressed(0x0004),
        a: pressed(0x0010),
        b: pressed(0x0020),
        x: pressed(0x0040),
        y: pressed(0x0080),
        left_trigger: report[4] as f32 / 255.0,
        right_trigger: report[5] as f32 / 255.0,
        left_stick_x: clean_motion_value(axis(6) / 32767.0, deadzone),
        left_stick_y: clean_motion_value(axis(8) / -32767.0, deadzone),
        right_stick_x: clean_motion_value(axis(10) / 32767.0, deadzone),
        right_stick_y: clean_motion_value(axis(12) / -32767.0, deadzone),
    })
}

#[derive(Debug, Clone, Copy)]
struct Model {
    name: &'static str,
    /// Stick deadzone from the DS's KnownGamepads table.
    deadzone: f32,
    ps4: bool,
}

/// The DS's deadzone for gamepads it doesn't recognize.
const UNKNOWN_DEADZONE: f32 = 0.2;

/// Gamepads the SDK 12.0 Driver Station knows (KnownGamepads.Model).
fn known_model(vendor: u16, product: u16) -> Option<Model> {
    let (name, deadzone, ps4) = match (vendor, product) {
        (0x054c, 0x05c4) | (0x054c, 0x09cc) => ("Sony DualShock 4", 0.08, true),
        (0x054c, 0x0ce6) => ("Sony DualSense", 0.05, true),
        (0x046d, 0xc21d) => ("Logitech F310", 0.06, false),
        (0x046d, 0xc216) => ("Logitech F310 (D mode)", 0.06, false),
        (0x7545, 0x0104) => ("Etpark gamepad", 0.05, true),
        (0x0c12, 0x2e18) => ("REV USB PS4-compatible gamepad", 0.0, true),
        (0x045e, 0x028e) => ("Xbox 360 Controller", 0.15, false),
        _ => return None,
    };
    Some(Model {
        name,
        deadzone,
        ps4,
    })
}

struct Pad {
    id: i32,
    name: String,
    user: Option<u8>,
    gamepad_type: u8,
    state: GamepadState,
    /// Monotonic ms of the last state change; sent as the Gamepad timestamp.
    changed_ms: i64,
}

struct NeutralPacket {
    id: i32,
    user: u8,
    gamepad_type: u8,
    remaining: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct GamepadInfo {
    pub id: i32,
    pub name: String,
    /// 1 = gamepad1, 2 = gamepad2, None = not sending to the robot.
    pub user: Option<u8>,
    pub state: GamepadState,
}

#[derive(Debug, Clone, Serialize)]
pub struct GamepadStatus {
    pub notice: Option<String>,
    pub pads: Vec<GamepadInfo>,
}

pub struct GamepadManager {
    pads: Mutex<Vec<Pad>>,
    neutral: Mutex<Vec<NeutralPacket>>,
    /// User-facing problems, keyed by the backend that reported them.
    notices: Mutex<BTreeMap<&'static str, String>>,
    next_id: AtomicI32,
    log: Arc<SessionLog>,
}

impl GamepadManager {
    pub fn start(log: Arc<SessionLog>) -> Arc<Self> {
        let manager = Arc::new(Self {
            pads: Mutex::new(Vec::new()),
            neutral: Mutex::new(Vec::new()),
            notices: Mutex::new(BTreeMap::new()),
            next_id: AtomicI32::new(1),
            log,
        });
        system::spawn(Arc::clone(&manager));
        #[cfg(target_os = "macos")]
        xinput_usb::spawn(Arc::clone(&manager));
        manager
    }

    pub fn status(&self) -> GamepadStatus {
        let notices = self.notices.lock().expect("notices lock");
        let pads = self.pads.lock().expect("pads lock");
        GamepadStatus {
            notice: (!notices.is_empty())
                .then(|| notices.values().cloned().collect::<Vec<_>>().join(" ")),
            pads: pads
                .iter()
                .map(|pad| GamepadInfo {
                    id: pad.id,
                    name: pad.name.clone(),
                    user: pad.user,
                    state: pad.state,
                })
                .collect(),
        }
    }

    /// Assign a gamepad to gamepad1/gamepad2 (or unassign with `None`). Any
    /// other gamepad holding that slot is unassigned, like the DS's Start+A /
    /// Start+B binding.
    pub fn set_user(&self, id: i32, user: Option<u8>) -> Result<(), String> {
        if matches!(user, Some(u) if u != 1 && u != 2) {
            return Err("user must be 1 or 2".to_string());
        }
        let mut pads = self.pads.lock().expect("pads lock");
        if !pads.iter().any(|pad| pad.id == id) {
            return Err("gamepad not found".to_string());
        }
        self.assign(&mut pads, id, user);
        Ok(())
    }

    /// Gamepad payloads to send this tick, following the DS rule: send every
    /// tick unless the gamepad has been at rest for over a second.
    pub fn take_transmissions(&self) -> Vec<Vec<u8>> {
        let now = monotonic_ms();
        let mut payloads = Vec::new();
        {
            let mut neutral = self.neutral.lock().expect("neutral lock");
            for packet in neutral.iter_mut() {
                payloads.push(encode_gamepad(
                    packet.id,
                    now,
                    packet.user,
                    packet.gamepad_type,
                    &GamepadState::default(),
                ));
                packet.remaining -= 1;
            }
            neutral.retain(|packet| packet.remaining > 0);
        }
        let pads = self.pads.lock().expect("pads lock");
        for pad in pads.iter() {
            let Some(user) = pad.user else { continue };
            if now - pad.changed_ms > AT_REST_RESEND_MS && pad.state.at_rest() {
                continue;
            }
            payloads.push(encode_gamepad(
                pad.id,
                pad.changed_ms,
                user,
                pad.gamepad_type,
                &pad.state,
            ));
        }
        payloads
    }

    /// Register a newly attached gamepad and return its id.
    fn add_pad(&self, name: &str, ps4: bool) -> i32 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut pads = self.pads.lock().expect("pads lock");
        pads.push(Pad {
            id,
            name: name.to_string(),
            user: None,
            gamepad_type: if ps4 {
                GAMEPAD_TYPE_SONY_PS4
            } else {
                GAMEPAD_TYPE_XBOX_360
            },
            state: GamepadState::default(),
            changed_ms: monotonic_ms(),
        });
        self.log
            .record("gamepad_added", json!({ "id": id, "name": name }));
        // Convenience over the official DS: take the first free slot so a
        // single plugged-in controller drives gamepad1 without Start+A.
        let free = [1, 2]
            .into_iter()
            .find(|slot| !pads.iter().any(|pad| pad.user == Some(*slot)));
        if let Some(slot) = free {
            self.assign(&mut pads, id, Some(slot));
        }
        id
    }

    fn remove_pad(&self, id: i32) {
        let mut pads = self.pads.lock().expect("pads lock");
        if let Some(index) = pads.iter().position(|pad| pad.id == id) {
            let pad = pads.remove(index);
            self.log.record("gamepad_removed", json!({ "id": id }));
            if let Some(user) = pad.user {
                self.queue_neutral(id, user, pad.gamepad_type);
            }
        }
    }

    fn update_pad(&self, id: i32, state: GamepadState) {
        let mut pads = self.pads.lock().expect("pads lock");
        let Some(pad) = pads.iter_mut().find(|pad| pad.id == id) else {
            return;
        };
        if pad.state == state {
            return;
        }
        let previous = pad.state;
        pad.state = state;
        pad.changed_ms = monotonic_ms();
        // DS binding gestures: Start+A -> gamepad1, Start+B -> gamepad2.
        if state.start && state.a && !previous.a {
            self.assign(&mut pads, id, Some(1));
        } else if state.start && state.b && !previous.b {
            self.assign(&mut pads, id, Some(2));
        }
    }

    fn set_notice(&self, source: &'static str, notice: Option<String>) {
        let mut notices = self.notices.lock().expect("notices lock");
        match notice {
            Some(message) => notices.insert(source, message),
            None => notices.remove(source),
        };
    }

    fn assign(&self, pads: &mut [Pad], id: i32, user: Option<u8>) {
        if let Some(user) = user {
            for other in pads.iter_mut().filter(|pad| pad.id != id) {
                if other.user == Some(user) {
                    other.user = None;
                }
            }
        }
        let Some(pad) = pads.iter_mut().find(|pad| pad.id == id) else {
            return;
        };
        if pad.user == user {
            return;
        }
        if let Some(previous) = pad.user {
            self.queue_neutral(id, previous, pad.gamepad_type);
        }
        pad.user = user;
        pad.changed_ms = monotonic_ms();
        self.log
            .record("gamepad_assigned", json!({ "id": id, "user": user }));
    }

    fn queue_neutral(&self, id: i32, user: u8, gamepad_type: u8) {
        let mut neutral = self.neutral.lock().expect("neutral lock");
        neutral.retain(|packet| packet.user != user);
        neutral.push(NeutralPacket {
            id,
            user,
            gamepad_type,
            remaining: NEUTRAL_REPEATS,
        });
    }
}

fn monotonic_ms() -> i64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    // Offset so timestamps are never zero, which the SDK treats as "unset".
    START.get_or_init(Instant::now).elapsed().as_millis() as i64 + 1
}

/// Gamepads the OS already recognizes, via gilrs: XInput/Windows.Gaming.Input
/// on Windows, evdev on Linux, HID on macOS (PS4/PS5, F310 in D mode, ...).
mod system {
    use super::{clean_motion_value, known_model, GamepadManager, GamepadState, UNKNOWN_DEADZONE};
    use gilrs::ev::filter::{axis_dpad_to_button, Filter};
    use gilrs::{Axis, Button, EventType, Gamepad, GamepadId, GilrsBuilder};
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    const NOTICE: &str = "system";
    const POLL_TIMEOUT: Duration = Duration::from_millis(50);

    struct Tracked {
        id: i32,
        deadzone: f32,
    }

    pub fn spawn(manager: Arc<GamepadManager>) {
        thread::spawn(move || {
            // Default filters include gilrs's own deadzone; apply the DS's
            // per-model deadzone instead, keeping only the hat-to-dpad mapping.
            let builder = GilrsBuilder::new()
                .with_default_filters(false)
                .set_update_state(false);
            let mut gilrs = match builder.build() {
                Ok(gilrs) => gilrs,
                Err(err) => {
                    manager.set_notice(NOTICE, Some(format!("Gamepad input unavailable: {err}")));
                    return;
                }
            };
            let mut tracked: HashMap<GamepadId, Tracked> = HashMap::new();
            let initial = gilrs.gamepads().map(|(gid, _)| gid).collect::<Vec<_>>();
            for gid in initial {
                connect(&manager, &mut tracked, gid, &gilrs.gamepad(gid));
            }
            loop {
                let mut changed = HashSet::new();
                let mut next = gilrs.next_event_blocking(Some(POLL_TIMEOUT));
                while let Some(event) = next.filter_ev(&axis_dpad_to_button, &mut gilrs) {
                    if !event.is_dropped() {
                        gilrs.update(&event);
                        match event.event {
                            EventType::Connected => {
                                connect(&manager, &mut tracked, event.id, &gilrs.gamepad(event.id));
                            }
                            EventType::Disconnected => {
                                if let Some(pad) = tracked.remove(&event.id) {
                                    manager.remove_pad(pad.id);
                                }
                            }
                            _ => {
                                changed.insert(event.id);
                            }
                        }
                    }
                    next = gilrs.next_event();
                }
                for gid in changed {
                    if let Some(pad) = tracked.get(&gid) {
                        manager.update_pad(pad.id, read_state(&gilrs.gamepad(gid), pad.deadzone));
                    }
                }
            }
        });
    }

    fn connect(
        manager: &GamepadManager,
        tracked: &mut HashMap<GamepadId, Tracked>,
        gid: GamepadId,
        gamepad: &Gamepad<'_>,
    ) {
        if tracked.contains_key(&gid) {
            return;
        }
        let model = gamepad
            .vendor_id()
            .zip(gamepad.product_id())
            .and_then(|(vendor, product)| known_model(vendor, product));
        let (name, deadzone, ps4) = match model {
            Some(model) => (model.name.to_string(), model.deadzone, model.ps4),
            None => (gamepad.name().to_string(), UNKNOWN_DEADZONE, false),
        };
        let id = manager.add_pad(&name, ps4);
        tracked.insert(gid, Tracked { id, deadzone });
    }

    fn read_state(gamepad: &Gamepad<'_>, deadzone: f32) -> GamepadState {
        let stick = |axis| clean_motion_value(gamepad.value(axis), deadzone);
        let trigger = |button| gamepad.button_data(button).map_or(0.0, |data| data.value());
        let pressed = |button| gamepad.is_pressed(button);
        GamepadState {
            // gilrs reports up as positive; FTC wants up negative.
            left_stick_x: stick(Axis::LeftStickX),
            left_stick_y: -stick(Axis::LeftStickY),
            right_stick_x: stick(Axis::RightStickX),
            right_stick_y: -stick(Axis::RightStickY),
            left_trigger: trigger(Button::LeftTrigger2),
            right_trigger: trigger(Button::RightTrigger2),
            dpad_up: pressed(Button::DPadUp),
            dpad_down: pressed(Button::DPadDown),
            dpad_left: pressed(Button::DPadLeft),
            dpad_right: pressed(Button::DPadRight),
            a: pressed(Button::South),
            b: pressed(Button::East),
            x: pressed(Button::West),
            y: pressed(Button::North),
            guide: pressed(Button::Mode),
            start: pressed(Button::Start),
            back: pressed(Button::Select),
            left_bumper: pressed(Button::LeftTrigger),
            right_bumper: pressed(Button::RightTrigger),
            left_stick_button: pressed(Button::LeftThumb),
            right_stick_button: pressed(Button::RightThumb),
        }
    }
}

/// XInput gamepads over raw USB. macOS has no driver for them, so they never
/// show up as HID devices (and gilrs can't see them).
#[cfg(target_os = "macos")]
mod xinput_usb {
    use super::{known_model, parse_xinput_report, GamepadManager};
    use nusb::transfer::{In, Interrupt};
    use nusb::{DeviceId, DeviceInfo, MaybeFuture};
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    const NOTICE: &str = "xinput";
    const XINPUT_DEVICES: &[(u16, u16)] = &[(0x046d, 0xc21d), (0x045e, 0x028e)];
    const XINPUT_INTERFACE: u8 = 0;
    const XINPUT_ENDPOINT_IN: u8 = 0x81;
    const SCAN_INTERVAL: Duration = Duration::from_secs(1);
    const READ_TIMEOUT: Duration = Duration::from_millis(250);

    type Endpoint = nusb::Endpoint<Interrupt, In>;

    pub fn spawn(manager: Arc<GamepadManager>) {
        thread::spawn(move || {
            let open: Arc<Mutex<HashSet<DeviceId>>> = Arc::default();
            loop {
                let notice = scan(&manager, &open);
                manager.set_notice(NOTICE, notice);
                thread::sleep(SCAN_INTERVAL);
            }
        });
    }

    /// Open any newly attached XInput gamepads. Returns a user-facing notice
    /// for gamepads that are present but unusable.
    fn scan(manager: &Arc<GamepadManager>, open: &Arc<Mutex<HashSet<DeviceId>>>) -> Option<String> {
        let devices = match nusb::list_devices().wait() {
            Ok(devices) => devices,
            Err(err) => return Some(format!("USB scan failed: {err}")),
        };
        let mut notice = None;
        for info in devices {
            let key = (info.vendor_id(), info.product_id());
            if !XINPUT_DEVICES.contains(&key)
                || open.lock().expect("open lock").contains(&info.id())
            {
                continue;
            }
            let Some(model) = known_model(key.0, key.1) else {
                continue;
            };
            match open_endpoint(&info) {
                Ok(endpoint) => {
                    open.lock().expect("open lock").insert(info.id());
                    let id = manager.add_pad(model.name, model.ps4);
                    spawn_reader(
                        endpoint,
                        id,
                        model.deadzone,
                        info.id(),
                        Arc::clone(manager),
                        Arc::clone(open),
                    );
                }
                Err(err) => notice = Some(format!("Could not open {}: {err}", model.name)),
            }
        }
        notice
    }

    fn open_endpoint(info: &DeviceInfo) -> Result<Endpoint, nusb::Error> {
        let device = info.open().wait()?;
        // With no driver attached, macOS leaves the device unconfigured.
        if device.active_configuration().is_err() {
            device.set_configuration(1).wait()?;
        }
        let interface = device.claim_interface(XINPUT_INTERFACE).wait()?;
        interface.endpoint::<Interrupt, In>(XINPUT_ENDPOINT_IN)
    }

    fn spawn_reader(
        mut endpoint: Endpoint,
        id: i32,
        deadzone: f32,
        device: DeviceId,
        manager: Arc<GamepadManager>,
        open: Arc<Mutex<HashSet<DeviceId>>>,
    ) {
        thread::spawn(move || {
            loop {
                while endpoint.pending() < 2 {
                    let buffer = endpoint.allocate(endpoint.max_packet_size());
                    endpoint.submit(buffer);
                }
                let Some(completion) = endpoint.wait_next_complete(READ_TIMEOUT) else {
                    continue;
                };
                // Any transfer error here means unplugged or a stalled pipe;
                // drop the gamepad and let the next scan reopen it.
                if completion.status.is_err() {
                    break;
                }
                if let Some(state) = parse_xinput_report(&completion.buffer, deadzone) {
                    manager.update_pad(id, state);
                }
                endpoint.submit(completion.buffer);
            }
            manager.remove_pad(id);
            open.lock().expect("open lock").remove(&device);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_matches_sdk_layout() {
        let state = GamepadState {
            left_stick_y: -1.0,
            a: true,
            right_bumper: true,
            ..GamepadState::default()
        };
        let payload = encode_gamepad(7, 1234, 1, GAMEPAD_TYPE_XBOX_360, &state);
        assert_eq!(payload.len(), GAMEPAD_PAYLOAD_SIZE);
        assert_eq!(payload[0], 5);
        assert_eq!(&payload[1..5], &7_i32.to_be_bytes());
        assert_eq!(&payload[5..13], &1234_i64.to_be_bytes());
        assert_eq!(&payload[17..21], &(-1.0_f32).to_be_bytes());
        // a = bit 8, right_bumper = bit 0.
        assert_eq!(&payload[37..41], &0x101_i32.to_be_bytes());
        assert_eq!(&payload[41..44], &[1, 2, 2]);
    }

    #[test]
    fn xinput_report_parses_like_the_ds() {
        // Left stick fully up, A + left bumper, right trigger full.
        let mut report = [0_u8; 20];
        report[1] = 0x14;
        report[3] = 0x11;
        report[5] = 0xff;
        report[8..10].copy_from_slice(&32767_i16.to_le_bytes());
        let state = parse_xinput_report(&report, 0.06).unwrap();
        assert!(state.a && state.left_bumper && !state.b);
        assert_eq!(state.left_stick_y, -1.0);
        assert_eq!(state.right_trigger, 1.0);
        assert_eq!(state.left_stick_x, 0.0);
    }

    #[test]
    fn deadzone_rescales() {
        assert_eq!(clean_motion_value(0.05, 0.06), 0.0);
        assert_eq!(clean_motion_value(-1.2, 0.06), -1.0);
        assert!((clean_motion_value(0.53, 0.06) - 0.5).abs() < 1e-6);
    }
}
