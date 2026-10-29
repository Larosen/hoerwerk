import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, readdirSync, renameSync } from "node:fs";
import { dirname, join } from "node:path";
const ROOT = process.env.E2E_ROOT || "e2e/.run";   // prepared by e2e/setup.mjs

declare const live: { ok: boolean };   // global of the page (frontend/src/live.ts)

function tone(path: string, seconds: number, tags: Record<string, string> = {}): void {
  mkdirSync(dirname(path), { recursive: true });
  const meta = Object.entries(tags).flatMap(([k, v]) => ["-metadata", `${k}=${v}`]);
  execFileSync("ffmpeg", ["-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i", `sine=f=440:d=${seconds}`,
    "-ar", "44100", "-ac", "2", ...meta, path]);
}

/** A finished download: built elsewhere, then moved into the inbox in one step (like SABnzbd does). */
function deliver(name: string, files: [string, Record<string, string>][]): void {
  const tmp = join(ROOT, "incoming", name);
  for (const [file, tags] of files) {
    tone(join(tmp, file), 1.5, tags);
    chmodSync(join(tmp, file), 0o666);
  }
  chmodSync(tmp, 0o777);              // like a share with UMASK 000: the app may move it, whoever wrote it
  renameSync(tmp, join(ROOT, "inbox", name));
}

/** Console errors and uncaught exceptions fail every test. */
function watchErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
  return errors;
}

async function open(page: Page): Promise<string[]> {
  const errors = watchErrors(page);
  await page.goto("/");
  await expect.poll(() => page.evaluate(() => live.ok), { message: "Live-Verbindung" }).toBe(true);
  return errors;
}

test.describe.configure({ mode: "serial" });

test("Umbenennen mit Vorschau und Rückgängig", async ({ page }) => {
  const errors = await open(page);
  await page.getByRole("button", { name: "📁 Umbenennen" }).click();
  // the default rule "Aufräumen" turns underscores into spaces
  await expect(page.locator("#previewBody tr[data-name='Erste_Datei.txt'] td.new")).toHaveText("Erste Datei.txt");
  await expect(page.locator("#summary")).toContainText("2 von 2");
  await page.locator("#applyBtn").click();
  await expect(page.locator(".toast")).toContainText("2 Einträge umbenannt");
  expect(readdirSync(join(ROOT, "data", "Umbenennen")).sort()).toEqual(["Erste Datei.txt", "Zweite Datei.txt"]);
  await page.locator("#undoBtn").click();
  await expect(page.locator(".toast")).toContainText("zurückbenannt");
  expect(readdirSync(join(ROOT, "data", "Umbenennen")).sort()).toEqual(["Erste_Datei.txt", "Zweite_Datei.txt"]);
  expect(errors).toEqual([]);
});

test("„Jetzt zu M4B“ erscheint live in den Konvertierungen", async ({ page }) => {
  const errors = await open(page);
  await page.getByRole("button", { name: "📁 Hörbuch Ordner" }).click();
  await page.locator("#convertNowBtn").click();
  // no reload, no polling: the job list updates through the live connection
  await expect(page.locator("#jobList .job.done")).toContainText("Hörbuch Ordner.m4b");
  expect(existsSync(join(ROOT, "data", "Hörbuch Ordner", "Hörbuch Ordner.m4b"))).toBe(true);
  await expect(page.locator("#previewBody")).toContainText("Hörbuch Ordner.m4b");
  expect(errors).toEqual([]);
});

test("Eingang → M4B → Bibliothek, Doppeltes wird erkannt", async ({ page }) => {
  const errors = await open(page);
  await page.getByRole("tab", { name: /Hörbücher/ }).click();
  const tags = { album: "Neues Buch", artist: "Erika Muster" };
  deliver("Erika Muster - Neues Buch", [["01.mp3", tags], ["02.mp3", tags]]);
  await page.locator("#scanBtn").click();
  const card = page.locator("#bookList .book", { hasText: "Neues Buch" });
  await expect(card).toHaveClass(/done/);
  await expect(card.locator(".book-dest")).toContainText("Erika Muster/Neues Buch/Neues Buch.m4b");
  expect(existsSync(join(ROOT, "library", "Erika Muster", "Neues Buch", "metadata.json"))).toBe(true);

  // the same book again: not filed a second time, a person decides
  deliver("Neues Buch nochmal", [["01.mp3", tags]]);
  await page.locator("#scanBtn").click();
  await expect(page.locator("#reviewBadge")).toHaveText("1");
  const dup = page.locator("#bookList .book.review");
  await expect(dup.locator(".dup-box")).toContainText("Erika Muster/Neues Buch");
  await dup.getByRole("button", { name: "Verwerfen" }).click();
  await expect(page.locator("#bookList .book", { hasText: "Neues Buch nochmal" })).toHaveClass(/cancelled/);
  expect(errors).toEqual([]);
});

test("Altes MP3-Buch mit eigenen Kapitelnamen zu M4B", async ({ page }) => {
  const errors = await open(page);
  await page.getByRole("tab", { name: /Hörbücher/ }).click();
  await page.locator("#bookFilter").getByRole("button", { name: /Nicht als M4B/ }).click();
  const book = page.locator("#bookList .book.uc", { hasText: "Altes Buch" });
  await book.getByRole("button", { name: "Kapitel & Namen …" }).click();
  await expect(page.locator("#tracksBody input")).toHaveCount(2);
  await page.locator("[data-preset=numbered]").click();
  await expect(page.locator("#tracksBody input").first()).toHaveValue("Kapitel 1");
  await page.locator("#tracksBody input").nth(1).fill("Das Ende");
  await page.locator("#tracksRename").check();
  await page.locator("#tracksGo").click();
  await page.locator("#bookFilter").getByRole("button", { name: /Fertig/ }).click();
  await expect(page.locator("#bookList .book.done", { hasText: "Altes Buch" })).toBeVisible();
  expect(existsSync(join(ROOT, "library", "Max Muster", "Altes Buch", "Altes Buch.m4b"))).toBe(true);
  // the old files went to _ersetzt, renamed as chosen
  expect(readdirSync(join(ROOT, "inbox", "_ersetzt", "Max Muster", "Altes Buch", "CD2"))).toEqual(["02 - Das Ende.mp3"]);
  expect(errors).toEqual([]);
});

test("Einstellungen zeigen ffmpeg und Sandbox, Design bleibt nach Neuladen", async ({ page }) => {
  const errors = await open(page);
  await page.locator("#settingsBtn").click();
  await expect(page.locator("#systemInfo")).toContainText("ffmpeg");
  await page.locator("#settingsDialog button[value=cancel]").click();
  await page.locator("#themeBtn").click();          // auto -> light
  await page.locator("#themeBtn").click();          // light -> dark
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  const bg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
  expect(bg).toBe("rgb(16, 18, 22)");
  expect(errors).toEqual([]);
});

test("Fremde Seiten dürfen nichts ändern (CSRF)", async ({ page, request }) => {
  await open(page);
  const res = await request.post("/api/rename", {
    data: { path: "Umbenennen", files: ["Erste_Datei.txt"], rules: [] },
    headers: { Origin: "http://evil.example" },
  });
  expect(res.status()).toBe(403);
});
