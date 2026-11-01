import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { createHmac } from "node:crypto";
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

const ADMIN = { name: "admin", password: "E2E-Admin-Kaffee-1" };
const OWN = { "X-Requested-With": "Hoerwerk" };

/** Sign in through the API (cookie lands in the page's context), then open the app. */
async function open(page: Page, who: { name: string; password: string } = ADMIN): Promise<string[]> {
  const errors = watchErrors(page);
  const res = await page.request.post("/api/auth/login", { headers: OWN, data: { ...who, remember: false } });
  expect(res.status()).toBe(200);
  await page.goto("/");
  if (who !== ADMIN) return errors;          // family accounts have no live connection
  await expect.poll(() => page.evaluate(() => live.ok), { message: "Live-Verbindung" }).toBe(true);
  return errors;
}

test.describe.configure({ mode: "serial" });

test("Bibliothek: alle Bücher, mit Suche", async ({ page }) => {
  const errors = await open(page);
  await expect(page).toHaveURL(/\/discover$/);                                        // everyone starts with Entdecken
  await expect(page.locator("#welcome")).toBeHidden();                                 // the guide is for family accounts
  await page.locator("nav.tabs").getByRole("link", { name: /Hörbücher/ }).click();
  await expect(page.locator("nav.tabs").getByRole("link", { name: /Hörbücher/ })).toHaveAttribute("aria-current", "page");
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
  await page.locator("nav.tabs").getByRole("link", { name: /Hörbücher/ }).click();
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
  await page.locator("nav.tabs").getByRole("link", { name: /Hörbücher/ }).click();
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

test("Einstellungen: Übersicht, Healthcheck mit ffmpeg und Proxy, Design bleibt nach Neuladen", async ({ page }) => {
  const errors = await open(page);
  await page.locator("#settingsBtn").click();
  await expect(page).toHaveURL(/\/settings$/);
  await expect(page.locator("#settingsHub .hub-tile")).toHaveCount(10);
  await page.locator("#settingsHub").getByRole("link", { name: /Healthcheck/ }).click();
  await expect(page).toHaveURL(/\/settings\/health$/);
  await expect(page.locator("#healthList")).toContainText("ffmpeg");
  await expect(page.locator("#healthList")).toContainText("Hörwerk sieht dich als 127.0.0.1");
  await expect(page.locator("#healthList")).toContainText("Live-Updates kommen an");
  await page.locator("#settingsNav").getByRole("link", { name: /M4B-Konvertierung/ }).click();
  await expect(page.locator("#settingsForm fieldset:not([hidden]) legend")).toHaveText(["M4B-Konvertierung"]);
  await page.goBack();
  await expect(page).toHaveURL(/\/settings\/health$/);
  await expect(page.locator("#healthList")).toBeVisible();
  await page.goBack();
  await expect(page.locator("#settingsHub")).toBeVisible();
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
  const ntfy = { url: "https://ntfy.example", topic: "hoerwerk-e2e", events: {} };
  expect((await page.request.put("/api/notify", { headers: OWN, data: ntfy })).status()).toBe(200);
  // a slow server: the saved values arrive after the page is shown – they must not overwrite a choice
  await page.route("**/api/settings", async (r) => {
    if (r.request().method() === "GET") await new Promise((done) => setTimeout(done, 800));
    await r.continue();
  });
  await page.goto("/settings/general");
  await page.locator("#settingsForm select[name=language]").selectOption("en");
  await page.locator("#settingsForm button[type=submit]").click();
  await expect(page.locator("nav.tabs").getByRole("link", { name: /Audiobooks/ })).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await expect(page.locator("#settingsBtn")).toContainText("Settings");
  // nothing German left in the visible chrome of the page
  const text = await page.locator("header.topbar").innerText();
  expect(text).not.toMatch(/Einstellungen|Hörbücher|Suchen|Protokoll/);
  await page.locator("#settingsBtn").click();
  await page.locator("#settingsHub").getByRole("link", { name: /M4B conversion/ }).click();
  await expect(page.locator("#settingsForm fieldset:not([hidden]) legend")).toHaveText(["M4B conversion"]);
  await expect(page).toHaveTitle("M4B conversion · Hörwerk");
  await page.locator("#settingsNav").getByRole("link", { name: /General/ }).click();
  await page.locator("#settingsForm select[name=language]").selectOption("de");
  await page.locator("#settingsForm button[type=submit]").click();
  await expect(page.locator("nav.tabs").getByRole("link", { name: /Hörbücher/ })).toBeVisible();
  // saving on another page left ntfy as it was
  const saved = await (await page.request.get("/api/notify")).json();
  expect([saved.url, saved.topic]).toEqual([ntfy.url, ntfy.topic]);
  expect((await page.request.put("/api/notify", { headers: OWN, data: { url: "", topic: "", events: {} } })).status()).toBe(200);
  expect(errors).toEqual([]);
});

test("Seiten mit eigener Adresse: vor, zurück, Lesezeichen", async ({ page, browser }) => {
  const errors = await open(page);
  await expect(page).toHaveURL(/\/discover$/);
  await page.locator("nav.tabs").getByRole("link", { name: "Suchen" }).click();
  await expect(page).toHaveURL(/\/search$/);
  await page.locator("nav.tabs").getByRole("link", { name: "Anfragen" }).click();
  await page.locator("nav.tabs").getByRole("link", { name: "Entdecken" }).click();
  await expect(page).toHaveURL(/\/discover$/);
  await expect(page).toHaveTitle("Entdecken · Hörwerk");
  await page.goBack();
  await expect(page).toHaveURL(/\/requests$/);
  await page.goBack();
  await expect(page).toHaveURL(/\/search$/);
  await expect(page.locator("#viewSearch")).toBeVisible();
  await expect(page.locator("#viewDiscover")).toBeHidden();
  await page.goForward();
  await page.goForward();
  await expect(page.locator("#viewDiscover")).toBeVisible();
  // a bookmark without being signed in: first the login, then exactly that page
  const ctx = await browser.newContext();
  const other = await ctx.newPage();
  await other.goto("/settings/logs");
  await expect(other).toHaveURL(/\/login\?next=\/settings\/logs$/);
  const form = other.locator("#loginForm");
  await form.getByLabel("Name").fill(ADMIN.name);
  await form.getByLabel("Passwort").fill(ADMIN.password);
  await form.getByRole("button", { name: "Anmelden" }).click();
  await expect(other).toHaveURL(/\/settings\/logs$/);
  await expect(other.locator("#logView")).toBeVisible();
  // a foreign "next" is ignored
  await other.goto("/login?next=https://evil.example/");
  await expect(other).toHaveURL(/\/discover$/);
  await ctx.close();
  expect(errors).toEqual([]);
});

test("Fremde Seiten dürfen nichts ändern (CSRF)", async ({ page, request }) => {
  await open(page);
  const res = await request.put("/api/settings", {
    data: { language: "en" },
    headers: { Origin: "http://evil.example" },
  });
  expect(res.status()).toBe(403);
  await expect(page.locator("nav.tabs").getByRole("link", { name: /Hörbücher/ })).toBeVisible();   // still German
});

test("Entdecken: Vorschläge ansehen und direkt suchen", async ({ page }) => {
  const suggestion = {
    asin: "B0TEST0001", title: "Das stille Haus", authors: ["Anna Autorin"], narrators: ["Sam Sprecher"],
    rating: 4.7, ratings: 1234, runtime_min: 600, release_date: "2026-03-01", cover_url: null,
    url: "https://www.audible.de/pd/B0TEST0001", because: ["Altes Buch"], source: "audible",
    genres: ["Krimis & Thriller", "Literatur & Belletristik"],
  };
  await page.route("**/api/discover?*", (r) => r.fulfill({ json: { items: [suggestion], updated: 1_700_000_000, error: null, running: false } }));
  await page.route("**/api/search", (r) => r.fulfill({ json: { results: [], errors: [], queries: ["Autorin Das stille Haus"] } }));
  const errors = await open(page);
  await page.locator("nav.tabs").getByRole("link", { name: "Entdecken" }).click();
  const card = page.locator(".suggestion");
  await expect(card).toHaveCount(1);
  await expect(card).toContainText("Das stille Haus");
  await expect(card).toContainText("★ 4.7");
  await expect(card.locator(".genre-tag")).toHaveText("Krimis & Thriller");          // the main category
  await expect(card).toContainText("Altes Buch");
  await card.getByRole("button", { name: "Suchen" }).click();
  await expect(page.locator("nav.tabs").getByRole("link", { name: "Suchen" })).toHaveAttribute("aria-current", "page");
  await expect(page.locator("#releaseQuery")).toHaveValue("Autorin Das stille Haus");
  await expect(page.locator("#bookPick")).toContainText("B0TEST0001");
  expect(errors).toEqual([]);
});

test("Entdecken: Nach Genre startet mit einem Genre, Unterkategorien wie bei Audible", async ({ page }) => {
  const genres = [
    { id: "1", name: "Geschichte", nonfiction: true, children: [{ id: "11", name: "Antike" }] },
    { id: "2", name: "Literatur & Belletristik", nonfiction: false, children: [{ id: "21", name: "Klassiker" }] },
  ];
  const asked: string[] = [];
  await page.route("**/api/discover/genres", (r) => r.fulfill({ json: { genres } }));
  await page.route("**/api/discover?*", (r) => {
    const q = new URL(r.request().url()).searchParams;
    const genre = q.get("section") === "genre" ? q.get("genre") || "" : "";
    if (q.get("section") === "genre") asked.push(genre);
    const item = { asin: `B0GENRE00${genre}`, title: `Buch aus ${genre}`, authors: ["Anna Autorin"], rating: 4.5, ratings: 80, source: "audible" };
    r.fulfill({ json: { items: genre ? [item] : [], updated: 1_700_000_000, error: null, running: false } });
  });
  const errors = await open(page);
  await page.locator("nav.tabs").getByRole("link", { name: "Entdecken" }).click();
  await page.locator("#discoverSections button[data-section=genre]").click();
  const sel = page.locator("#discoverGenre");
  await expect(sel).toHaveValue("2");                                   // no empty "–": fiction to start with
  await expect(page.locator(".suggestion")).toContainText("Buch aus 2");
  await expect(sel.locator("optgroup")).toHaveCount(2);
  await expect(sel.locator("option")).toHaveText(["Geschichte – alle", "Antike", "Literatur & Belletristik – alle", "Klassiker"]);
  await expect(page.locator("#discoverNote")).toBeHidden();
  await sel.selectOption("11");
  await expect(page.locator(".suggestion")).toContainText("Buch aus 11");
  await expect(page.locator("#discoverNote")).toBeVisible();            // non-fiction: stars are no fact check
  expect(asked).not.toContain("");
  expect(errors).toEqual([]);
});

test("Anmeldung: falsches Passwort, dann richtig; ohne Anmeldung keine Daten", async ({ page, request }) => {
  const errors = watchErrors(page);
  expect((await request.get("/api/settings")).status()).toBe(401);
  await page.goto("/");
  await expect(page).toHaveURL(/\/login$/);
  const form = page.locator("#loginForm");
  await form.getByLabel("Name").fill("admin");
  await form.getByLabel("Passwort").fill("falsch-falsch-falsch");
  await form.getByRole("button", { name: "Anmelden" }).click();
  await expect(page.locator("#loginError")).toHaveText("Name oder Passwort falsch");
  await form.getByLabel("Passwort").fill(ADMIN.password);
  await form.getByRole("button", { name: "Anmelden" }).click();
  await expect(page).toHaveURL(/\/discover$/);
  await expect(page.locator("nav.tabs").getByRole("link", { name: /Hörbücher/ })).toBeVisible();
  await page.locator("#accountBtn").click();
  await expect(page).toHaveURL(/\/account$/);
  await expect(page.locator("#whoami")).toContainText("127.0.0.1");
  await expect(page.locator("#sessionList")).toContainText("dieses Gerät");
  await page.locator("#logoutBtn").click();
  await expect(page).toHaveURL(/\/login$/);
  expect(errors.filter((e) => !e.includes("401"))).toEqual([]);      // the failed login itself answers 401
});

test("Konten: Admin legt Familienkonto an, das sieht nur Entdecken", async ({ page, browser }) => {
  const errors = await open(page);
  await page.goto("/settings/users");
  await page.locator("#newUserName").fill("Eva");
  await page.locator("#newUserPw").fill("Gruene-Tasse-72");
  await page.locator("#newUserSave").click();
  await expect(page.locator("#userRows")).toContainText("Eva");
  expect(errors).toEqual([]);

  const eva = await browser.newPage();
  const evaErrors = await open(eva, { name: "eva", password: "Gruene-Tasse-72" });
  await expect(eva.locator("nav.tabs").getByRole("link", { name: "Entdecken" })).toHaveAttribute("aria-current", "page");
  // a short guide for family accounts, once – and none of the admin boxes
  await expect(eva.locator("#welcome")).toBeVisible();
  await expect(eva.locator("#welcome")).toContainText("Wünschen");
  await expect(eva.locator("#scanBtn")).toBeHidden();
  await expect(eva.locator("#discoverRefresh")).toBeHidden();
  await eva.locator("#welcomeOk").click();
  await eva.reload();
  await expect(eva.locator("#welcome")).toBeHidden();
  await expect(eva.locator("#settingsBtn")).toBeHidden();
  await eva.goto("/settings/users");                       // no settings for family accounts: back to the start
  await expect(eva).toHaveURL(/\/discover$/);
  await expect(eva.locator("nav.tabs").getByRole("link", { name: /Hörbücher/ })).toBeHidden();
  await expect(eva.locator("#accountName")).toHaveText("Eva");
  // the server refuses admin functions, whatever the page shows
  expect((await eva.request.get("/api/sources")).status()).toBe(403);
  expect((await eva.request.put("/api/settings", { headers: OWN, data: { language: "en" } })).status()).toBe(403);
  expect(evaErrors.filter((e) => !e.includes("403"))).toEqual([]);
  await eva.close();
});

/** RFC 6238 code for a base32 secret (what an authenticator app shows). */
function totp(secret: string, offset = 0): string {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
  let bits = "";
  for (const c of secret.replace(/=+$/, "")) bits += alphabet.indexOf(c.toUpperCase()).toString(2).padStart(5, "0");
  const key = Buffer.from(bits.match(/.{8}/g)!.map((b) => parseInt(b, 2)));
  const step = Buffer.alloc(8);
  step.writeBigUInt64BE(BigInt(Math.floor(Date.now() / 30000) + offset));
  const h = createHmac("sha1", key).update(step).digest();
  const o = h[h.length - 1] & 15;
  return String((h.readUInt32BE(o) & 0x7fffffff) % 1_000_000).padStart(6, "0");
}

test("Anfragen: Familienkonto fragt an, Admin gibt frei, Releases mit Urteil", async ({ page, browser }) => {
  const admin = await open(page);
  // a family account (created through the API of the admin session)
  expect((await page.request.post("/api/users", { headers: OWN, data: { name: "Mia", password: "Gelbe-Tasse-123" } })).status()).toBe(200);
  const mia = await browser.newPage();
  const suggestion = { asin: "B0TEST0002", title: "Die leise Stadt", authors: ["Bea Beispiel"], narrators: [], rating: 4.6,
    ratings: 500, runtime_min: 480, cover_url: null, url: null, because: [], source: "audible", requested: null };
  await mia.route("**/api/discover?*", (r) => r.fulfill({ json: { items: [suggestion], updated: 1_700_000_000, error: null, running: false } }));
  const miaErrors = await open(mia, { name: "mia", password: "Gelbe-Tasse-123" });
  await mia.locator("nav.tabs").getByRole("link", { name: "Entdecken" }).click();
  await mia.locator(".suggestion").getByRole("button", { name: "Anfragen" }).click();
  await expect(mia.locator(".suggestion")).toContainText("Wartet auf Freigabe");
  await mia.locator("nav.tabs").getByRole("link", { name: /Anfragen/ }).click();
  await expect(mia.locator(".request")).toContainText("Die leise Stadt");
  await expect(mia.locator("#requestQuota")).toContainText("noch 4 von 5");
  await expect(mia.locator(".request").getByRole("button", { name: "Releases" })).toHaveCount(0);   // no right for that

  // the admin sees the badge, approves; without an indexer the request then waits in "searching"
  await page.reload();
  await expect(page.locator("#requestBadge")).toHaveText("1");
  await page.locator("nav.tabs").getByRole("link", { name: /Anfragen/ }).click();
  // admins always see everyone's requests – there is no switch that could hide a waiting approval
  await expect(page.locator("#requestScope")).toHaveCount(0);
  const req = page.locator(".request", { hasText: "Die leise Stadt" });
  await expect(req).toContainText("von Mia");
  await req.getByRole("button", { name: "Freigeben" }).click();
  await expect(req).toContainText("Wird gesucht");

  // interactive search: every release with its verdict (indexer answer simulated)
  await page.route("**/api/requests/*/releases", (r) => r.fulfill({ json: { query: "Beispiel Die leise Stadt", queries: [], errors: [], results: [
    { id: "a", title: "Bea.Beispiel-Die.leise.Stadt.German.M4B", size: 300 << 20, indexer: "X", format: "M4B", language: "de",
      verdict: { ok: true, score: 40, reason: null, notes: ["M4B", "Sprache passt"], kbps: 87 } },
    { id: "b", title: "Bea.Beispiel-Die.leise.Stadt.GEKUERZT", size: 90 << 20, indexer: "Y",
      verdict: { ok: false, score: 0, reason: "gekürzte Fassung", notes: [], kbps: 26 } }] } }));
  await req.getByRole("button", { name: "Releases" }).click();
  const rows = page.locator("#releasesRows tr");
  await expect(rows).toHaveCount(2);
  await expect(rows.first()).toContainText("Sprache passt");
  await expect(rows.nth(1)).toContainText("gekürzte Fassung");
  await expect(page.locator("#releasesNote")).toContainText("2 Releases, davon 1 passend");
  // the whole release name is readable (wrapped, not cut off)
  const name = rows.first().locator(".c-title span").first();
  expect(await name.evaluate((n) => n.scrollWidth <= n.clientWidth + 1)).toBe(true);
  // Enter in the search field searches again – the window stays open
  let searches = 0;
  page.on("request", (q) => { if (q.url().endsWith("/releases")) searches++; });
  await page.locator("#releasesQuery").fill("Beispiel Stadt");
  await page.locator("#releasesQuery").press("Enter");
  await expect.poll(() => searches).toBe(1);
  await expect(page.locator("#releasesDialog")).toBeVisible();
  await page.locator("#releasesDialog button[value=cancel]").click();

  // Mia sees the new state – and can't see other people's requests
  await mia.reload();
  await expect(mia.locator(".request")).toContainText("Wird gesucht");
  expect((await mia.request.get("/api/requests?scope=all")).status()).toBe(200);
  expect(admin.concat(miaErrors).filter((e) => !/40[34]/.test(e))).toEqual([]);
  await mia.close();
});

test("Einladung per Link: Admin erstellt, Eingeladene wählt Name und Passwort", async ({ page, browser }) => {
  const errors = await open(page);
  // without a public address there are no links – here the test server is that address
  await page.goto("/settings/users");
  await expect(page.locator("#invitePublic")).toBeVisible();
  await expect(page.locator("#inviteCreate")).toBeDisabled();
  await page.goto("/settings/general");
  await page.locator("#settingsForm input[name=public_url]").fill(new URL(page.url()).origin);
  await page.locator("#settingsForm button[type=submit]").click();
  await expect(page.locator("#toast")).toContainText("gespeichert");
  await page.goto("/settings/users");
  await expect(page.locator("#invitePublic")).toBeHidden();
  await page.locator("#inviteNote").fill("Oma");
  await page.locator("#invitePerms input[data-perm=auto_approve]").check();       // Oma's wishes go straight off
  await page.locator("#inviteCreate").click();
  const row = page.locator("#inviteList li", { hasText: "Oma" });
  await expect(row).toContainText("offen");
  await expect(row).toContainText("Startrechte: ohne Freigabe");
  const link = await row.locator("input").inputValue();
  expect(link).toMatch(/\/invite\/[A-Za-z0-9_-]{30,}$/);
  expect(errors).toEqual([]);

  const ctx = await browser.newContext();
  const oma = await ctx.newPage();
  const omaErrors = watchErrors(oma);
  await oma.goto(link);
  const form = oma.locator("#inviteForm");
  await expect(form).toBeVisible();
  await expect(oma.locator("#loginForm")).toBeHidden();
  await form.getByLabel("Dein Name").fill("Oma");
  // too weak: the checklist says what is missing, the server refuses it as well
  await form.getByLabel("Passwort", { exact: false }).first().fill("Pudding2024!");
  await expect(form.locator(".pw-rules")).toContainText("nicht nur ein Wort");
  await form.getByLabel("Passwort wiederholen").fill("Pudding2024!");
  await form.getByRole("button", { name: "Konto anlegen" }).click();
  await expect(oma.locator("#loginError")).toContainText("Passwort zu schwach");
  await form.getByLabel("Passwort", { exact: false }).first().fill("Rote-Tasse-74!");
  await form.getByLabel("Passwort wiederholen").fill("Rote-Tasse-74!");
  await form.getByRole("button", { name: "Konto anlegen" }).click();
  await expect(oma).toHaveURL(/\/discover$/);                       // signed in, straight to Entdecken
  await expect(oma.locator("#accountName")).toHaveText("Oma");
  await expect(oma.locator("#welcome")).toBeVisible();
  expect(omaErrors.filter((e) => !e.includes("400"))).toEqual([]);   // the refused weak password answers 400
  // the link is used up now
  const again = await ctx.newPage();
  await again.context().clearCookies();
  await again.goto(link);
  await expect(again.locator("#loginError")).toContainText("gilt nicht");
  await expect(again.locator("#inviteForm")).toBeHidden();
  await ctx.close();
  await page.reload();
  await expect(row).toContainText("benutzt");
  await expect(row).toContainText("von Oma");
  // the members list says where Oma came from, and that she was here just now – not "never"
  const member = page.locator("#userRows tr", { hasText: "Oma" });
  await expect(member).toContainText("über Einladung „Oma“");
  await expect(member).not.toContainText("noch nie");
  // the second-factor rule sits with the accounts now, not under Entdecken
  await expect(page.locator("#adminTotpRequired")).toBeVisible();
  await expect(page.locator("#settingsForm [name=admin_totp_required]")).toHaveCount(0);
  await expect(page.locator("#inviteUses option")).toHaveCount(22);                   // 1 … 20, 30, unbegrenzt
});

test("Zweitfaktor: einrichten, dann Anmeldung nur mit Code", async ({ page, browser }) => {
  const errors = await open(page);
  expect((await page.request.post("/api/users", { headers: OWN, data: { name: "Ole", password: "Blaue-Tasse-73" } })).status()).toBe(200);
  const ole = await browser.newPage();
  await open(ole, { name: "ole", password: "Blaue-Tasse-73" });
  await ole.locator("#accountBtn").click();
  await ole.locator("#totpBox").getByRole("button", { name: "Einrichten" }).click();
  const secret = (await ole.locator("code.secret").innerText()).replace(/\s/g, "");
  await ole.locator("#totpBox input.code").fill(totp(secret));
  await ole.locator("#totpBox").getByRole("button", { name: "Bestätigen" }).click();
  await expect(ole.locator("#totpBox")).toContainText("Aktiv");
  await ole.locator("#logoutBtn").click();
  await expect(ole).toHaveURL(/\/login$/);
  const form = ole.locator("#loginForm");
  await form.getByLabel("Name").fill("ole");
  await form.getByLabel("Passwort").fill("Blaue-Tasse-73");
  await form.getByRole("button", { name: "Anmelden" }).click();
  const code = ole.locator("#totpForm");
  await expect(code).toBeVisible();
  await code.getByLabel("Code").fill("000000");
  await code.getByRole("button", { name: "Bestätigen" }).click();
  await expect(ole.locator("#loginError")).toHaveText("Code falsch");
  await code.getByLabel("Code").fill(totp(secret, 1));          // the next code (the current one was used)
  await code.getByRole("button", { name: "Bestätigen" }).click();
  await expect(ole).toHaveURL(/\/$/);
  await expect(ole.locator("#accountName")).toHaveText("Ole");
  expect(errors).toEqual([]);
  await ole.close();
});

test("Hochladen: CD-Hörbuch als Ordner hochladen, landet als M4B in der Bibliothek", async ({ page }) => {
  const errors = await open(page);
  const dir = join(ROOT, "upload-src", "Clara Clever - Die Funkenstadt");
  const tags = { album: "Die Funkenstadt", artist: "Clara Clever" };
  tone(join(dir, "CD1", "01 Anfang.mp3"), 1.0, tags);
  tone(join(dir, "CD2", "01 Ende.mp3"), 1.0, tags);
  await page.locator("nav.tabs").getByRole("link", { name: /Hörbücher/ }).click();
  await page.locator("#uploadBtn").click();
  await page.locator("#uploadFolder").setInputFiles(dir);
  await expect(page.locator("#uploadList")).toContainText("Clara Clever - Die Funkenstadt");
  await expect(page.locator("#uploadList")).toContainText("2 Dateien");
  await page.locator("#uploadGo").click();
  await expect(page.locator("#uploadDialog")).not.toBeVisible();
  // no waiting for "settled" files: the inbox takes the upload at once and files it
  await expect.poll(() => existsSync(join(ROOT, "library", "Clara Clever", "Die Funkenstadt", "Die Funkenstadt.m4b")),
    { timeout: 45_000, message: "M4B in der Bibliothek" }).toBe(true);
  expect(errors).toEqual([]);
});
