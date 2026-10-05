//! Live Robot Controller smoke test. See `scripts/hub-check.sh`.

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    std::process::exit(eclipse_desktop_station_lib::hub_check::run(&args));
}
