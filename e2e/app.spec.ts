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

test("Bibliothek: alle Bücher, mit Suche", async ({ page }) => {
  const errors = await open(page);
  await expect(page.getByRole("tab", { name: /Hörbücher/ })).toHaveAttribute("aria-selected", "true");   // start page
  await page.locator("#bookFilter").getByRole("button", { name: /Bibliothek/ }).click();
  await expect(page.locator(".lib-row")).toHaveCount(1);
  await expect(page.locator(".lib-row")).toContainText("Altes Buch");
  await page.locator(".lib-search input").fill("gibtsnicht");
  await expect(page.locator(".lib-row")).toHaveCount(0);
  await page.locator(".lib-search input").fill("muster");
  await expect(page.locator(".lib-row")).toHaveCount(1);
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
  expect(bg).toBe("rgb(19, 20, 18)");
  expect(errors).toEqual([]);
});

test("Oberfläche auf Englisch und zurück", async ({ page }) => {
  const errors = await open(page);
  await page.locator("#settingsBtn").click();
  await page.locator("#settingsDialog select[name=language]").selectOption("en");
  await page.locator("#settingsDialog button[value=save]").click();
  await expect(page.getByRole("tab", { name: /Audiobooks/ })).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await expect(page.locator("#settingsBtn")).toContainText("Settings");
  // nothing German left in the visible chrome of the page
  const text = await page.locator("header.topbar").innerText();
  expect(text).not.toMatch(/Einstellungen|Hörbücher|Suchen|Protokoll/);
  await page.locator("#settingsBtn").click();
  await expect(page.locator("#settingsDialog legend").first()).toHaveText("M4B conversion");
  await page.locator("#settingsDialog select[name=language]").selectOption("de");
  await page.locator("#settingsDialog button[value=save]").click();
  await expect(page.getByRole("tab", { name: /Hörbücher/ })).toBeVisible();
  expect(errors).toEqual([]);
});

test("Fremde Seiten dürfen nichts ändern (CSRF)", async ({ page, request }) => {
  await open(page);
  const res = await request.put("/api/settings", {
    data: { language: "en" },
    headers: { Origin: "http://evil.example" },
  });
  expect(res.status()).toBe(403);
  await expect(page.getByRole("tab", { name: /Hörbücher/ })).toBeVisible();   // still German
});

test("Entdecken: Vorschläge ansehen und direkt suchen", async ({ page }) => {
  const suggestion = {
    asin: "B0TEST0001", title: "Das stille Haus", authors: ["Anna Autorin"], narrators: ["Sam Sprecher"],
    rating: 4.7, ratings: 1234, runtime_min: 600, release_date: "2026-03-01", cover_url: null,
    url: "https://www.audible.de/pd/B0TEST0001", because: ["Altes Buch"], source: "audible",
  };
  await page.route("**/api/discover?*", (r) => r.fulfill({ json: { items: [suggestion], updated: 1_700_000_000, error: null, running: false } }));
  await page.route("**/api/search", (r) => r.fulfill({ json: { results: [], errors: [], queries: ["Autorin Das stille Haus"] } }));
  const errors = await open(page);
  await page.getByRole("tab", { name: "Entdecken" }).click();
  const card = page.locator(".suggestion");
  await expect(card).toHaveCount(1);
  await expect(card).toContainText("Das stille Haus");
  await expect(card).toContainText("★ 4.7");
  await expect(card).toContainText("Altes Buch");
  await card.getByRole("button", { name: "Suchen" }).click();
  await expect(page.getByRole("tab", { name: "Suchen" })).toHaveAttribute("aria-selected", "true");
  await expect(page.locator("#searchQuery")).toHaveValue("Autorin Das stille Haus");
  await expect(page.locator("#bookPick")).toContainText("B0TEST0001");
  expect(errors).toEqual([]);
});
