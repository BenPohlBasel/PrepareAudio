#!/usr/bin/env node
// Das DMG notarisieren und das Ticket anheften (Tauri reicht nur die .app ein;
// ein nicht notarisiertes DMG meldet Gatekeeper als „Unnotarized Developer ID“).
//
// Aufruf: node scripts/notarize-dmg.mjs [pfad.dmg]   (ohne Pfad: neuestes DMG im Bundle-Ordner)
// Zugang: APPLE_KEYCHAIN_PROFILE oder APPLE_ID + APPLE_TEAM_ID + APPLE_PASSWORD.
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const DMG_DIR = path.join(ROOT, "src-tauri/target/release/bundle/dmg");
const neuestes = () =>
  fs.existsSync(DMG_DIR)
    ? fs.readdirSync(DMG_DIR).filter((n) => n.endsWith(".dmg")).map((n) => path.join(DMG_DIR, n))
        .sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs)[0] ?? null
    : null;
const dmg = process.argv[2] ?? neuestes();
if (!dmg || !fs.existsSync(dmg)) {
  console.error(`Kein DMG gefunden (${DMG_DIR}) — erst \`node scripts/build-dmg.mjs\`.`);
  process.exit(1);
}
if (spawnSync("/usr/bin/xcrun", ["stapler", "validate", dmg], { encoding: "utf8" }).status === 0) {
  console.log(`Ticket hängt bereits an ${path.basename(dmg)} — nichts zu tun.`);
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
console.log(`Reiche ein: ${path.basename(dmg)} (${(fs.statSync(dmg).size / 1e6).toFixed(0)} MB).`);
try {
  execFileSync("/usr/bin/xcrun", ["notarytool", "submit", dmg, ...zugang, "--wait", "--timeout", "2h"], { stdio: "inherit" });
  execFileSync("/usr/bin/xcrun", ["stapler", "staple", dmg], { stdio: "inherit" });
} catch {
  console.error("\nNotarisierung des DMG fehlgeschlagen — Grund: xcrun notarytool log <submission-id> …");
  process.exit(1);
}
const urteil = spawnSync("/usr/sbin/spctl", ["-a", "-t", "open", "--context", "context:primary-signature", "-vv", dmg], { encoding: "utf8" });
console.log((urteil.stderr || urteil.stdout).trim());
