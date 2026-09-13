#!/usr/bin/env node
// Die gebaute .app notarisieren und das Ticket anheften — außerhalb von
// `tauri build` (wie bei LocalTranscript: Apples Warteschlange kann länger
// dauern als Tauris interne Wartezeit). `tauri build` läuft dafür ohne
// APPLE_*-Variablen und signiert nur.
//
// Aufruf: node scripts/notarize-app.mjs [pfad.app]
// Zugang: APPLE_KEYCHAIN_PROFILE oder APPLE_ID + APPLE_TEAM_ID + APPLE_PASSWORD.
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const app = process.argv[2] ?? path.join(ROOT, "src-tauri/target/release/bundle/macos/PrepareAudio.app");
if (!fs.existsSync(app)) {
  console.error(`Keine App unter ${app} — erst \`tauri build\`.`);
  process.exit(1);
}
if (spawnSync("/usr/bin/xcrun", ["stapler", "validate", app], { encoding: "utf8" }).status === 0) {
  console.log(`Ticket hängt bereits an ${path.basename(app)} — nichts zu tun.`);
  process.exit(0);
}
const profil = process.env.APPLE_KEYCHAIN_PROFILE;
const zugang = profil
  ? ["--keychain-profile", profil]
  : ["--apple-id", process.env.APPLE_ID ?? "", "--team-id", process.env.APPLE_TEAM_ID ?? "", "--password", process.env.APPLE_PASSWORD ?? ""];
if (!profil && zugang.some((v) => v === "")) {
  console.error("Zugang fehlt: APPLE_KEYCHAIN_PROFILE oder APPLE_ID + APPLE_TEAM_ID + APPLE_PASSWORD.");
  process.exit(1);
}
const zip = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "pa-notar-")), "PrepareAudio.zip");
execFileSync("/usr/bin/ditto", ["-c", "-k", "--keepParent", app, zip], { stdio: "inherit" });
console.log(`Reiche ein: ${path.basename(app)} (${(fs.statSync(zip).size / 1e6).toFixed(0)} MB) — Timeout 2 h.`);
try {
  execFileSync("/usr/bin/xcrun", ["notarytool", "submit", zip, ...zugang, "--wait", "--timeout", "2h"], { stdio: "inherit" });
  execFileSync("/usr/bin/xcrun", ["stapler", "staple", app], { stdio: "inherit" });
} catch {
  console.error("\nNotarisierung der App fehlgeschlagen — Grund: xcrun notarytool log <submission-id> …");
  process.exit(1);
} finally {
  fs.rmSync(path.dirname(zip), { recursive: true, force: true });
}
const urteil = spawnSync("/usr/sbin/spctl", ["-a", "-vv", "-t", "exec", app], { encoding: "utf8" });
console.log((urteil.stderr || urteil.stdout).trim());
