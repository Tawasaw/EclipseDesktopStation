#!/usr/bin/env bash
# Smoke-test EclipseDesktopStation against a live Control Hub.
#
#   scripts/hub-check.sh                       # read-only checks, 192.168.43.1
#   scripts/hub-check.sh --opmode "My TeleOp"  # also init/start/stop (robot may move!)
#   scripts/hub-check.sh 192.168.49.1          # different RC address
#
# Build once while online (`scripts/hub-check.sh --build`), then run on the hub's
# Wi-Fi. Close the desktop app and the phone Driver Station first. The report is
# written to hub-check-report.txt in the repo root.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
manifest="$root/src-tauri/Cargo.toml"

if [[ "${1:-}" == "--build" ]]; then
  exec cargo build --manifest-path "$manifest" --example hub_check
fi

cd "$root"
exec cargo run --quiet --offline --manifest-path "$manifest" --example hub_check -- \
  --report "$root/hub-check-report.txt" "$@"
