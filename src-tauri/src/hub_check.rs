//! Offline smoke test against a live Robot Controller, driven through the same
//! `RobocolClient` the app uses. Run with `scripts/hub-check.sh` while joined to
//! the Control Hub's Wi-Fi; it writes a plain-text report for later review.
//!
//! Read-only by default. OpModes are only initialized/started when one is named
//! with `--opmode`, because that can move the robot.

use crate::debug_log::SessionLog;
use crate::robocol::{RobocolClient, RobotSnapshot, DEFAULT_OP_MODE_NAME, ROBOCOL_VERSION};
use crate::xml_config::validate_robot_xml;
use std::fmt::Write as _;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const EXPECTED_SDK: &str = "12.0";
const DEFAULT_IP: &str = "192.168.43.1";
const DEFAULT_REPORT: &str = "hub-check-report.txt";

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Pass,
    Warn,
    Fail,
    Skip,
    Info,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::Warn => "WARN",
            Status::Fail => "FAIL",
            Status::Skip => "SKIP",
            Status::Info => "INFO",
        }
    }
}

struct Report {
    text: String,
    counts: [usize; 5],
}

impl Report {
    fn line(&mut self, status: Status, name: &str, detail: impl AsRef<str>) {
        self.counts[status as usize] += 1;
        let line = format!("[{}] {name}: {}", status.label(), detail.as_ref());
        println!("{line}");
        let _ = writeln!(self.text, "{line}");
    }

    fn check(
        &mut self,
        ok: bool,
        name: &str,
        pass: impl AsRef<str>,
        fail: impl AsRef<str>,
    ) -> bool {
        if ok {
            self.line(Status::Pass, name, pass);
        } else {
            self.line(Status::Fail, name, fail);
        }
        ok
    }

    fn failed(&self) -> bool {
        self.counts[Status::Fail as usize] > 0
    }
}

struct Options {
    ip: String,
    op_mode: Option<String>,
    report_path: String,
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        ip: DEFAULT_IP.to_string(),
        op_mode: None,
        report_path: DEFAULT_REPORT.to_string(),
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--opmode" => {
                options.op_mode = Some(iter.next().ok_or("--opmode needs a name")?.clone())
            }
            "--report" => options.report_path = iter.next().ok_or("--report needs a path")?.clone(),
            "-h" | "--help" => return Err(String::new()),
            other if other.starts_with('-') => return Err(format!("unknown option {other}")),
            other => options.ip = other.to_string(),
        }
    }
    Ok(options)
}

/// Entry point for the `hub_check` example. Returns the process exit code:
/// 0 when no check failed, 1 when any did, 2 for bad arguments.
pub fn run(args: &[String]) -> i32 {
    let options = match parse_args(args) {
        Ok(options) => options,
        Err(message) => {
            if !message.is_empty() {
                eprintln!("error: {message}");
            }
            eprintln!("usage: hub-check [IP] [--opmode NAME] [--report PATH]");
            return 2;
        }
    };

    let mut report = Report {
        text: String::new(),
        counts: [0; 5],
    };
    let _ = writeln!(
        report.text,
        "EclipseDesktopStation hub check\napp version {}  expected RC SDK {EXPECTED_SDK}  robocol {ROBOCOL_VERSION}\nrobot {}  started {} (unix ms)\n",
        env!("CARGO_PKG_VERSION"),
        options.ip,
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
    );
    print!("{}", report.text);

    let log = Arc::new(SessionLog::new());
    let log_path = log.enable();
    report.line(Status::Info, "session log", &log_path);

    run_checks(&mut report, &options, &log);

    let summary = format!(
        "\nSUMMARY: {} pass, {} warn, {} fail, {} skip",
        report.counts[Status::Pass as usize],
        report.counts[Status::Warn as usize],
        report.counts[Status::Fail as usize],
        report.counts[Status::Skip as usize],
    );
    println!("{summary}");
    let _ = writeln!(report.text, "{summary}");
    match std::fs::write(&options.report_path, &report.text) {
        Ok(()) => println!("report written to {}", options.report_path),
        Err(err) => eprintln!("could not write report {}: {err}", options.report_path),
    }
    if report.failed() {
        1
    } else {
        0
    }
}

fn run_checks(report: &mut Report, options: &Options, log: &Arc<SessionLog>) {
    let client = match RobocolClient::connect(&options.ip, Arc::clone(log)) {
        Ok(client) => client,
        Err(err) => {
            report.line(Status::Fail, "open socket", err.to_string());
            return;
        }
    };
    let local_port = client.snapshot().local_port;
    report.line(
        if local_port == Some(crate::robocol::ROBOCOL_PORT) {
            Status::Info
        } else {
            Status::Warn
        },
        "local port",
        format!("{local_port:?} (20884 busy? close the desktop app before running this check)"),
    );

    let linked = wait_for(&client, 6, |s| s.connected).is_some();
    if !report.check(
        linked,
        "robot reachable",
        "receiving Robocol packets",
        format!(
            "no packets from {} in 6s — joined the Control Hub Wi-Fi? correct IP?",
            options.ip
        ),
    ) {
        client.disconnect();
        return;
    }

    check_peer_discovery(report, &client);
    check_metadata(report, &client);
    check_battery(report, &client);
    check_config_download(report, &client);
    match &options.op_mode {
        Some(name) => check_op_mode_lifecycle(report, &client, name),
        None => report.line(
            Status::Skip,
            "opmode lifecycle",
            "pass --opmode NAME to init/start/stop an OpMode (robot may move)",
        ),
    }

    let snapshot = client.snapshot();
    if let Some(error) = &snapshot.robot_error {
        report.line(Status::Info, "robot error banner", one_line(error));
    }
    if let Some(notice) = &snapshot.robot_notice {
        report.line(Status::Info, "robot toast", one_line(notice));
    }
    if let Some(error) = &snapshot.last_error {
        report.line(Status::Warn, "socket error", error);
    }
    client.disconnect();
}

fn check_peer_discovery(report: &mut Report, client: &RobocolClient) {
    let Some(snapshot) = wait_for(client, 4, |s| s.robot_sdk.is_some()) else {
        report.line(
            Status::Fail,
            "peer discovery",
            "no PeerDiscovery packet from RC in 4s",
        );
        return;
    };
    report.check(
        !snapshot.peer_conflict,
        "no other driver station",
        "RC accepted this station",
        "RC reports another DS/Driver Hub already connected — close the phone DS app and rerun",
    );
    let robocol = snapshot.robot_robocol_version.unwrap_or_default();
    report.check(
        robocol == ROBOCOL_VERSION,
        "robocol version",
        format!("{robocol}"),
        format!("RC speaks {robocol}, app speaks {ROBOCOL_VERSION}"),
    );
    let sdk = snapshot.robot_sdk.unwrap_or_default();
    if sdk.starts_with(EXPECTED_SDK) {
        report.line(Status::Pass, "RC SDK version", sdk);
    } else {
        report.line(
            Status::Warn,
            "RC SDK version",
            format!("{sdk} — expected {EXPECTED_SDK}; update the Robot Controller app/Control Hub"),
        );
    }
}

fn check_metadata(report: &mut Report, client: &RobocolClient) {
    match wait_for(client, 4, |s| s.robot_state != "Unknown") {
        Some(s) => report.line(
            Status::Pass,
            "heartbeat",
            format!("robot state {}", s.robot_state),
        ),
        None => report.line(
            Status::Fail,
            "heartbeat",
            "no heartbeat with robot state in 4s",
        ),
    }

    let snapshot = wait_for(client, 4, |s| {
        s.active_config.is_some() && !s.configs.is_empty() && !s.op_modes.is_empty()
    })
    .unwrap_or_else(|| {
        client.refresh_metadata();
        thread::sleep(Duration::from_secs(2));
        client.snapshot()
    });

    let active = snapshot.active_config.as_ref().map(|c| c.name.clone());
    report.check(
        active.is_some(),
        "active config",
        active.clone().unwrap_or_default(),
        "no CMD_NOTIFY_ACTIVE_CONFIGURATION received",
    );
    let names = snapshot
        .configs
        .iter()
        .map(|c| c.name.as_str())
        .collect::<Vec<_>>();
    if report.check(
        !names.is_empty(),
        "config list",
        format!("{} config(s): {}", names.len(), names.join(", ")),
        "no CMD_REQUEST_CONFIGURATIONS_RESP received",
    ) {
        if let Some(active) = &active {
            if !names.iter().any(|n| n.eq_ignore_ascii_case(active)) {
                report.line(
                    Status::Warn,
                    "active config listed",
                    format!("'{active}' not in config list"),
                );
            }
        }
    }

    let modes = &snapshot.op_modes;
    if !report.check(
        !modes.is_empty(),
        "opmode list",
        format!("{} OpMode(s)", modes.len()),
        "no CMD_NOTIFY_OP_MODE_LIST received (or every OpMode was SYSTEM)",
    ) {
        return;
    }
    for mode in modes {
        let mut detail = format!("{:<10} {}", mode.flavor, mode.name);
        if !mode.group.is_empty() && mode.group != "$$$$$$$" {
            let _ = write!(detail, "  [group {}]", mode.group);
        }
        if let Some(description) = mode.description.as_deref().filter(|d| !d.is_empty()) {
            let _ = write!(detail, "  — {}", one_line(description));
        }
        report.line(Status::Info, "  opmode", detail);
    }
    let first_utility = modes.iter().position(|m| m.flavor == "UTILITY");
    match first_utility {
        Some(index) => {
            let ordered = modes[index..].iter().all(|m| m.flavor == "UTILITY");
            report.check(
                ordered,
                "utility opmodes",
                format!("{} listed after match OpModes", modes.len() - index),
                "UTILITY OpModes are interleaved with match OpModes",
            );
        }
        None => report.line(
            Status::Warn,
            "utility opmodes",
            "none reported — SDK 11.2+ RCs normally list TestHardware/TestGamepad",
        ),
    }
}

fn check_battery(report: &mut Report, client: &RobocolClient) {
    // SDK 12.0 fixed battery voltage not reaching the DS when the OpMode sends
    // no telemetry, so it should arrive even with no OpMode running.
    match wait_for(client, 6, |s| s.robot_battery.is_some()).and_then(|s| s.robot_battery) {
        Some(value) if value.trim().parse::<f64>().is_ok() => {
            report.line(Status::Pass, "battery voltage", format!("{value} V"))
        }
        Some(value) => report.line(
            Status::Warn,
            "battery voltage",
            format!("RC sent '{value}' — main battery not connected/switched on, or hub on USB power only"),
        ),
        None => report.line(
            Status::Warn,
            "battery voltage",
            "no battery telemetry in 6s (expected on SDK 12.0 even when idle)",
        ),
    }
}

fn check_config_download(report: &mut Report, client: &RobocolClient) {
    let Some(active) = client.snapshot().active_config.map(|c| c.name) else {
        report.line(Status::Skip, "config download", "no active config");
        return;
    };
    match client.download_config_xml(&active) {
        Ok(xml) => match validate_robot_xml(&xml) {
            Ok(validation) if validation.ok => {
                let mut detail = format!("'{active}' {} bytes, valid XML", xml.len());
                if !validation.warnings.is_empty() {
                    let _ = write!(detail, " (warnings: {})", validation.warnings.join("; "));
                }
                report.line(Status::Pass, "config download", detail);
            }
            Ok(validation) => report.line(
                Status::Fail,
                "config download",
                format!(
                    "'{active}' failed validation: {}",
                    validation.warnings.join("; ")
                ),
            ),
            Err(err) => report.line(
                Status::Fail,
                "config download",
                format!("'{active}': {err}"),
            ),
        },
        Err(err) => report.line(
            Status::Fail,
            "config download",
            format!("'{active}': {err}"),
        ),
    }
}

fn check_op_mode_lifecycle(report: &mut Report, client: &RobocolClient, name: &str) {
    let snapshot = client.snapshot();
    if !snapshot.op_modes.iter().any(|m| m.name == name) {
        report.line(
            Status::Fail,
            "opmode lifecycle",
            format!("'{name}' is not in the OpMode list"),
        );
        return;
    }
    client.clear_robot_messages();

    let _ = client.init_op_mode(name);
    let inited = wait_for(client, 6, |s| s.active_op_mode.as_deref() == Some(name)).is_some();
    if !report.check(
        inited,
        "opmode init",
        format!("RC confirmed '{name}' initialized"),
        format!("no CMD_NOTIFY_INIT_OP_MODE for '{name}' in 6s"),
    ) {
        let _ = client.stop_op_mode();
        return;
    }

    thread::sleep(Duration::from_secs(1));
    let _ = client.run_op_mode(name);
    let started = wait_for(client, 6, |s| s.op_mode_running).is_some();
    report.check(
        started,
        "opmode start",
        "RC confirmed running",
        "no CMD_NOTIFY_RUN_OP_MODE in 6s",
    );

    thread::sleep(Duration::from_secs(3));
    let telemetry = client.snapshot().telemetry;
    report.line(
        if telemetry.is_empty() {
            Status::Warn
        } else {
            Status::Pass
        },
        "opmode telemetry",
        format!("{} line(s)", telemetry.len()),
    );
    for line in telemetry.iter().take(15) {
        report.line(
            Status::Info,
            "  telemetry",
            format!("{} = {}", line.key, one_line(&line.value)),
        );
    }

    let _ = client.stop_op_mode();
    let stopped = wait_for(client, 6, |s| {
        s.active_op_mode.as_deref() == Some(DEFAULT_OP_MODE_NAME) && !s.op_mode_running
    })
    .is_some();
    report.check(
        stopped,
        "opmode stop",
        "RC returned to idle",
        "RC did not report $Stop$Robot$ within 6s — check the robot!",
    );
    if let Some(error) = client.snapshot().robot_error {
        report.line(Status::Fail, "opmode errors", one_line(&error));
    }
}

fn wait_for(
    client: &RobocolClient,
    seconds: u64,
    condition: impl Fn(&RobotSnapshot) -> bool,
) -> Option<RobotSnapshot> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let snapshot = client.snapshot();
        if condition(&snapshot) {
            return Some(snapshot);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn one_line(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 300 {
        format!("{}…", flat.chars().take(300).collect::<String>())
    } else {
        flat
    }
}
