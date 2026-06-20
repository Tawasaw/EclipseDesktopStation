// Sets src-tauri/tauri.conf.json "version" from a release tag.
//
// Usage: node scripts/set-version.mjs <tag>
//   e.g. node scripts/set-version.mjs v0.1.1  ->  version "0.1.1"
//
// Run in CI before `tauri build` so the app's displayed version, bundle file
// names, and the git tag all stay in sync (the tag is the source of truth).
// Tauri reads this `version` field for both package_info().version and the
// bundle names, so updating Cargo.toml is not required.

import { readFileSync, writeFileSync } from "node:fs";

const raw = (process.argv[2] ?? "").trim();
const version = raw.replace(/^v/, "");

if (!/^\d+\.\d+\.\d+(?:[-+].+)?$/.test(version)) {
  console.error(`set-version: refusing to set invalid version from tag "${raw}"`);
  process.exit(1);
}

const path = "src-tauri/tauri.conf.json";
const conf = JSON.parse(readFileSync(path, "utf8"));
conf.version = version;
writeFileSync(path, `${JSON.stringify(conf, null, 2)}\n`);

console.log(`set-version: ${path} version -> ${version}`);
