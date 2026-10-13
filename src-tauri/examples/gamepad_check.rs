//! Print the state of attached USB gamepads as the Driver Station would send it.
//!
//!     cargo run --example gamepad_check
use eclipse_desktop_station_lib::gamepad::GamepadManager;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

fn main() {
    let log = Arc::new(eclipse_desktop_station_lib::debug_log::SessionLog::new());
    let manager = GamepadManager::start(log);
    loop {
        let status = manager.status();
        if let Some(notice) = &status.notice {
            println!("notice: {notice}");
        }
        if status.pads.is_empty() {
            println!("no gamepads");
        }
        for pad in &status.pads {
            let s = &pad.state;
            println!(
                "#{} {} user={:?} L=({:+.2},{:+.2}) R=({:+.2},{:+.2}) LT={:.2} RT={:.2} a={} b={} x={} y={} lb={} rb={} dpad={}{}{}{} start={} back={} guide={} ls={} rs={}",
                pad.id, pad.name, pad.user, s.left_stick_x, s.left_stick_y, s.right_stick_x,
                s.right_stick_y, s.left_trigger, s.right_trigger, s.a as u8, s.b as u8, s.x as u8,
                s.y as u8, s.left_bumper as u8, s.right_bumper as u8, s.dpad_up as u8,
                s.dpad_down as u8, s.dpad_left as u8, s.dpad_right as u8, s.start as u8,
                s.back as u8, s.guide as u8, s.left_stick_button as u8, s.right_stick_button as u8
            );
        }
        thread::sleep(Duration::from_millis(250));
    }
}
