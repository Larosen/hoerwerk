// Fresh folders with real audio files (made with the system ffmpeg) – runs right before the server
// starts (webServer.command in playwright.config.ts; Playwright starts the server before globalSetup).
import { execFileSync } from "node:child_process";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const ROOT = process.env.E2E_ROOT || "e2e/.run";

function tone(path, seconds, tags = {}) {
  mkdirSync(dirname(path), { recursive: true });
  const meta = Object.entries(tags).flatMap(([k, v]) => ["-metadata", `${k}=${v}`]);
  execFileSync("ffmpeg", ["-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i", `sine=f=440:d=${seconds}`,
    "-ar", "44100", "-ac", "2", ...meta, path]);
}

rmSync(ROOT, { recursive: true, force: true });
for (const d of ["data", "config", "inbox", "library", "incoming"]) mkdirSync(join(ROOT, d), { recursive: true });
// renamer
mkdirSync(join(ROOT, "data", "Umbenennen"), { recursive: true });
for (const f of ["Erste_Datei.txt", "Zweite_Datei.txt"]) writeFileSync(join(ROOT, "data", "Umbenennen", f), "x");
// „Jetzt zu M4B“ in the renamer
tone(join(ROOT, "data", "Hörbuch Ordner", "01 Anfang.mp3"), 1.5);
tone(join(ROOT, "data", "Hörbuch Ordner", "02 Ende.mp3"), 1.5);
// an older library book that is still MP3, on two "CDs"
tone(join(ROOT, "library", "Max Muster", "Altes Buch", "CD1", "01.mp3"), 1.0, { album: "Altes Buch", artist: "Max Muster" });
tone(join(ROOT, "library", "Max Muster", "Altes Buch", "CD2", "01.mp3"), 1.0, { album: "Altes Buch", artist: "Max Muster" });
