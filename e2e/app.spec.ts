import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { createHmac } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, readdirSync, renameSync } from "node:fs";
import { dirname, join } from "node:path";
const ROOT = process.env.E2E_ROOT || "e2e/.run";   // prepared by e2e/setup.mjs

declare const live: { ok: boolean };   // global of the page (frontend/src/live.ts)
declare let versionChecked: number;
declare function pollBench(): Promise<void>;
declare const books: { data: unknown };
declare function applyBooks(d: unknown): void;   // global of the page (frontend/src/live.ts)

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

/** The detail column asks for the book's details; offline there is no shop – answer with the book itself. */
async function mockAnyDetail(page: Page): Promise<void> {
  await page.route("**/api/book/detail?*", (r) => r.fulfill({ json: { book: { asin: new URL(r.request().url()).searchParams.get("asin"), title: "Details", authors: [], description: "" }, similar: [], region: "de" } }));
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

test("Nicht als M4B: „Alle“ wählt aus und wieder ab", async ({ page }) => {
  const errors = await open(page);
  await page.locator("nav.tabs").getByRole("link", { name: /Hörbücher/ }).click();
  await page.locator("#bookFilter").getByRole("button", { name: /Nicht als M4B/ }).click();
  const all = page.locator(".uc-bar input[type=checkbox]").first();
  const rows = page.locator("#bookList .book.uc input[type=checkbox]");
  await all.check();
  await expect(rows.first()).toBeChecked();
  await page.locator(".uc-bar input[type=checkbox]").first().uncheck();      // focus stays on the box: the list must still rebuild
  await expect(rows.first()).not.toBeChecked();
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

test("Kapitel-Editor: zusammenführen, speichern, beim Öffnen ist die Änderung da", async ({ page }) => {
  const errors = await open(page);
  await page.locator("nav.tabs").getByRole("link", { name: /Hörbücher/ }).click();
  await page.locator("#bookFilter").getByRole("button", { name: /Bibliothek/ }).click();
  const row = page.locator("#bookList .lib-row", { hasText: "Altes Buch" });
  await row.getByRole("button", { name: /Kapitel/ }).click();
  const rows = page.locator("#chaptersList .chapter-row");
  await expect(rows).toHaveCount(2);
  await expect(page.locator("#chaptersSave")).toBeDisabled();              // nothing changed yet
  await rows.nth(1).getByRole("button", { name: "Zusammenführen" }).click();
  await expect(rows).toHaveCount(1);
  await rows.first().locator(".chapter-name").fill("");
  await expect(page.locator("#chaptersSave")).toBeDisabled();              // a chapter without a name can not be saved
  await rows.first().locator(".chapter-name").fill("Alles in einem");
  await page.locator("#chaptersSave").click();
  await expect(page.locator("#chaptersDialog")).not.toBeVisible();
  await row.getByRole("button", { name: /Kapitel/ }).click();
  await expect(rows).toHaveCount(1);
  await expect(rows.first().locator(".chapter-name")).toHaveValue("Alles in einem");
  expect(errors).toEqual([]);
});

test("Einstellungen: Übersicht, Systemprüfung mit ffmpeg und Proxy, Design bleibt nach Neuladen", async ({ page }) => {
  const errors = await open(page);
  await page.locator("#settingsBtn").click();
  await expect(page).toHaveURL(/\/settings$/);
  await expect(page.locator("#settingsHub .hub-tile")).toHaveCount(12);
  await page.locator("#settingsHub").getByRole("link", { name: /Systemprüfung/ }).click();
  await expect(page).toHaveURL(/\/settings\/health$/);
  await expect(page.locator("#healthList")).toContainText("ffmpeg");
  await expect(page.locator("#healthList")).toContainText("Hörwerk sieht dich als 127.0.0.1");
  await expect(page.locator("#healthList")).toContainText("Live-Aktualisierung funktioniert");
  await page.locator("#settingsNav").getByRole("link", { name: /M4B-Konvertierung/ }).click();
  await expect(page.locator("#settingsForm fieldset:not([hidden]) legend")).toHaveText(["M4B-Konvertierung"]);
  await page.goBack();
  await expect(page).toHaveURL(/\/settings\/health$/);
  await expect(page.locator("#healthList")).toBeVisible();
  await page.goBack();
  await expect(page.locator("#settingsHub")).toBeVisible();
  // colour scheme: chosen under Konto → Anzeige, saved in the account – it stays after a reload
  await page.locator("#accountBtn").click();
  await expect(page).toHaveURL(/\/account$/);
  await page.locator("#themeCards").getByRole("button", { name: /Dunkel/ }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  const bg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
  expect(bg).toBe("rgb(29, 25, 22)");
  // OLED: true black, and only on this device when asked for
  await page.locator("#themeScope").getByRole("radio", { name: /Nur dieses Gerät/ }).click();
  await page.locator("#themeCards").getByRole("button", { name: /OLED/ }).click();
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "oled");
  expect(await page.evaluate(() => getComputedStyle(document.body).backgroundColor)).toBe("rgb(0, 0, 0)");
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

test("Entdecken: Vorschläge ansehen, Details öffnen, nichts wird von selbst gesucht", async ({ page }) => {
  const suggestion = {
    asin: "B0TEST0001", title: "Das stille Haus", authors: ["Anna Autorin"], narrators: ["Sam Sprecher"],
    rating: 4.7, ratings: 1234, runtime_min: 600, release_date: "2026-03-01", cover_url: null,
    url: "https://www.audible.de/pd/B0TEST0001", because: ["Altes Buch"], source: "review",       // tips from critics carry an ASIN too: the details load for them
    genres: ["Krimis & Thriller", "Literatur & Belletristik"],
  };
  await page.route("**/api/discover?*", (r) => r.fulfill({ json: { items: [suggestion], updated: 1_700_000_000, error: null, running: false } }));
  await page.route("**/api/book/detail?*", (r) => r.fulfill({ json: { book: { ...suggestion, description: "Ein Haus, das schweigt." }, similar: [], region: "de" } }));
  let indexerSearches = 0;
  await page.route("**/api/search", (r) => { indexerSearches++; return r.fulfill({ json: { results: [], errors: [], queries: [] } }); });
  const errors = await open(page);
  await page.locator("nav.tabs").getByRole("link", { name: "Entdecken" }).click();
  const card = page.locator(".suggestion");
  await expect(card).toHaveCount(1);
  await expect(card).toContainText("Das stille Haus");
  await expect(card).toContainText("★ 4.7");
  await expect(card.locator(".genre-tag")).toHaveText("Krimis & Thriller");          // the main category
  await expect(card).toContainText("Altes Buch");
  await expect(card.getByRole("button", { name: "Anfragen" })).toBeVisible();
  await expect(card.getByRole("button", { name: "Suchen" })).toHaveCount(0);          // the request does the searching
  await card.locator(".s-title").click();
  await expect(page.locator("#bookDetail .detail-blurb")).toHaveText("Ein Haus, das schweigt.");
  expect(indexerSearches).toBe(0);
  expect(errors).toEqual([]);
});

test("Entdecken: Genres eingeklappt, Haupt- und Unterkategorie wählen und wieder abwählen, Länge und Jahr filtern", async ({ page }) => {
  const genres = [
    { id: "1", name: "Geschichte", nonfiction: true, children: [{ id: "11", name: "Antike" }] },
    { id: "2", name: "Literatur & Belletristik", nonfiction: false, children: [{ id: "21", name: "Klassiker" }] },
  ];
  const asked: string[] = [];
  await mockAnyDetail(page);        // the first card opens in the detail column on a wide screen
  await page.route("**/api/discover/genres", (r) => r.fulfill({ json: { genres } }));
  const year = new Date().getFullYear();
  const mine = [
    { asin: "B0FORYOU01", title: "Kurzer Klassiker", authors: ["A"], genres: ["Literatur & Belletristik", "Klassiker"], runtime_min: 200, release_date: `${year}-01-01`, source: "audible" },
    { asin: "B0FORYOU02", title: "Langes Geschichtsbuch", authors: ["B"], genres: ["Geschichte", "Antike"], runtime_min: 900, release_date: "2001-01-01", source: "audible" },
  ];
  await page.route("**/api/discover?*", (r) => {
    const q = new URL(r.request().url()).searchParams;
    const genre = q.get("genre") || "";
    if (q.get("section") === "genre") asked.push(genre);
    const item = { asin: `B0GENRE00${genre}`, title: `Buch aus ${genre}`, authors: ["Anna Autorin"], rating: 4.5, ratings: 80, source: "audible" };
    r.fulfill({ json: { items: q.get("section") === "genre" ? [item] : mine, updated: 1_700_000_000, error: null, running: false } });
  });
  const errors = await open(page);
  await page.locator("nav.tabs").getByRole("link", { name: "Entdecken" }).click();
  const cards = page.locator(".suggestion");
  await expect(cards).toHaveCount(2);
  const panel = page.locator("#genrePanel");
  await expect(panel).toBeHidden();                                                     // folded away: no wall of chips
  await expect(page.locator("#genreLabel")).toHaveText("Alle");
  // Für dich: the genre is a filter on the list
  await page.locator("#genreToggle").click();
  await panel.getByRole("button", { name: /^Geschichte/ }).click();
  await expect(panel.locator("#discoverSubGenres button")).toHaveText(["Alle in Geschichte", "Antike"]);
  await expect(cards).toHaveCount(1);
  await expect(cards).toContainText("Langes Geschichtsbuch");
  await panel.getByRole("button", { name: /^Geschichte/ }).click();                     // again: taken back
  await expect(cards).toHaveCount(2);
  await panel.getByRole("button", { name: /^Literatur/ }).click();
  await panel.getByRole("button", { name: "Klassiker" }).click();                       // a subcategory closes the panel
  await expect(panel).toBeHidden();
  await expect(page.locator("#genreLabel")).toHaveText("Literatur & Belletristik › Klassiker");
  await expect(cards).toHaveCount(1);
  await page.locator("#genreClear").click();
  await expect(page.locator("#genreLabel")).toHaveText("Alle");
  await expect(cards).toHaveCount(2);
  // length and year
  await page.locator("#lengthFilter").selectOption("long");
  await expect(cards).toHaveCount(1);
  await expect(cards).toContainText("Langes Geschichtsbuch");
  await page.locator("#yearFilter").selectOption("2");
  await expect(page.locator("#discoverList")).toContainText("Nichts passt zu den Filtern.");
  await page.getByRole("button", { name: "Filter zurücksetzen" }).click();
  await expect(cards).toHaveCount(2);
  // Nach Genre: no genre yet – it asks, the panel opens; the genre picks the list from the server
  await page.locator("#discoverSections button[data-section=genre]").click();
  await expect(page.locator("#discoverList")).toContainText("Genre wählen.");
  await expect(panel).toBeVisible();
  await panel.getByRole("button", { name: /^Geschichte/ }).click();
  await expect(cards).toContainText("Buch aus 1");
  await expect(page.locator("#discoverNote")).toBeVisible();                             // non-fiction: stars are no fact check
  await panel.getByRole("button", { name: "Antike" }).click();
  await expect(cards).toContainText("Buch aus 11");
  await page.locator("#genreToggle").click();
  await panel.getByRole("button", { name: "Alle Genres" }).click();
  await expect(page.locator("#discoverList")).toContainText("Genre wählen.");
  expect(asked).not.toContain("");
  expect(errors).toEqual([]);
});

test("Suchen: Deutsch oder Englisch – die Wahl geht mit, jede Kachel zeigt die Sprache, die Indexer bleiben in Ruhe", async ({ page }) => {
  const asked: { query: string; language: string }[] = [];
  await page.route("**/api/search/books", async (r) => {
    const body = r.request().postDataJSON();
    asked.push(body);
    const english = body.language === "en";
    await r.fulfill({ json: { region: "de", books: [{
      asin: english ? "B004V0T0VK" : "B00TEST001", title: english ? "Twilight" : "Bis(s) zum Morgengrauen",
      authors: ["Stephenie Meyer"], language: english ? "English" : "German", genres: ["Jugendliche & Heranwachsende"],
      series: "Twilight", series_position: "1", runtime_min: 700, source: "audible" }] } });
  });
  await mockAnyDetail(page);
  const indexerSearch: unknown[] = [];
  await page.route("**/api/search", (r) => { indexerSearch.push(r.request().postDataJSON()); return r.fulfill({ json: { results: [], errors: [], queries: [] } }); });
  const errors = await open(page);
  await page.locator("nav.tabs").getByRole("link", { name: "Suchen" }).click();
  await page.locator("#searchQuery").fill("Twilight");
  await page.locator("#bookLookupBtn").click();
  await expect(page.locator("#bookPick .suggestion")).toContainText("Bis(s) zum Morgengrauen");
  await expect(page.locator("#bookPick .lang-tag")).toHaveCount(0);                   // German is the normal case: no mark
  await page.locator("#bookLang").getByText("Englisch").click();                       // searches again at once
  await expect(page.locator("#bookPick .suggestion")).toContainText("Twilight");
  await expect(page.locator("#bookPick .lang-tag")).toHaveText("Englisch");
  expect(asked.map((a) => a.language)).toEqual(["de", "en"]);
  await expect(page.locator("#releaseTools")).toBeHidden();                            // looking at hits never starts a release search
  expect(indexerSearch).toEqual([]);
  await page.locator("#bookLang").getByText("Deutsch").click();                         // back for the next tests
  expect(errors).toEqual([]);
});

test("Suchen: nichts gefunden – erst dann die Suche bei den Indexern, ohne Hinweise vorher", async ({ page }) => {
  await page.route("**/api/search/books", (r) => r.fulfill({ json: { region: "de", books: [] } }));
  await page.route("**/api/search/free", (r) => r.fulfill({ json: { books: [] } }));
  const asked: { query: string }[] = [];
  await page.route("**/api/search", (r) => { asked.push(r.request().postDataJSON()); return r.fulfill({ json: { results: [], errors: [], queries: [] } }); });
  const errors = await open(page);
  await page.locator("nav.tabs").getByRole("link", { name: "Suchen" }).click();
  await expect(page.locator("#freePick")).toBeHidden();
  await page.locator("#searchQuery").fill("Gibt es nicht");
  await page.locator("#bookLookupBtn").click();
  await expect(page.locator("#bookPick")).toContainText("Nichts gefunden");
  expect(asked).toEqual([]);                                                           // nothing at the indexers by itself
  await page.locator("#bookPick").getByRole("button", { name: /Direkt bei den Indexern/ }).click();
  await expect.poll(() => asked.map((a) => a.query)).toEqual(["Gibt es nicht"]);
  await expect(page.locator("#releaseTools")).toBeVisible();
  expect(errors).toEqual([]);
});

test("Suchen: ganze Reihe anfragen – Bände wählen, eine Karte unter Anfragen", async ({ page }) => {
  const vol = (n: number) => ({ asin: `B0E2ESER0${n}`, title: `Glaswald ${n}`, authors: ["Mira Testfeld"], language: "English",
    series: "Glaswald-Chroniken", series_asin: "B0E2ESERIE", series_position: String(n), runtime_min: 500, source: "audible", region: "us" });
  await page.route("**/api/search/books", (r) => r.fulfill({ json: { region: "de", books: [vol(1)] } }));
  await mockAnyDetail(page);
  const asked: { asin: string; region: string; language: string }[] = [];
  await page.route("**/api/search/series", (r) => {
    asked.push(r.request().postDataJSON());
    return r.fulfill({ json: { series: { asin: "B0E2ESERIE", title: "Glaswald-Chroniken", region: "us" },
      books: [vol(1), vol(2), { ...vol(3), owned: { title: "Glaswald 3" } }] } });
  });
  const errors = await open(page);
  await page.locator("nav.tabs").getByRole("link", { name: "Suchen" }).click();
  await page.locator("#searchQuery").fill("Glaswald");
  await page.locator("#bookLookupBtn").click();
  const chip = page.locator("#bookPick .series-chip");
  await expect(chip).toContainText("Glaswald-Chroniken");
  await expect(chip.locator(".lang-tag")).toHaveText("Englisch");                     // an English series is marked
  await chip.click();
  const d = page.locator("#seriesDialog");
  await expect(d.locator(".series-vol")).toHaveCount(3);
  expect(asked[0]).toMatchObject({ asin: "B0E2ESERIE", region: "us", language: "en" });
  await expect(d.locator(".series-vol").nth(2).locator("input")).toBeDisabled();     // already in the library
  await expect(page.locator("#seriesOk")).toHaveText("2 Bände anfragen");
  await page.locator("#seriesOk").click();
  await expect(page.locator("#toast")).toContainText("Reihe angefragt (2 Bände)");
  await page.locator("nav.tabs").getByRole("link", { name: "Anfragen" }).click();
  const card = page.locator("li.request-group", { hasText: "Glaswald-Chroniken" });
  await expect(card).toHaveCount(1);                                                  // one card, not two
  await card.locator("summary").click();
  await expect(card.locator(".group-vol")).toHaveText([/Band 1 · Glaswald 1/, /Band 2 · Glaswald 2/]);
  await card.locator(".s-actions").getByRole("button", { name: "Reihe löschen" }).click();
  await page.locator("#confirmDialog button[value=ok]").click();
  await expect(card).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("Downloads: eigene Seite in den Einstellungen, Fertige ausblenden und aufräumen", async ({ page }) => {
  const errors = await open(page);
  let items = [
    { id: "a", name: "Buch A", status: "Downloading", progress: 0.42, done: false, size: "400 MB", left: "0:01:10", finished: null, error: null },
    { id: "c", name: "Buch C", status: "Completed", progress: 1, done: true, size: "300 MB", left: "", finished: 1761000000, error: null },
    { id: "d", name: "Buch D", status: "Failed", progress: 0, done: true, size: "1 GB", left: "", finished: null, error: "Out of retention" },
  ];
  const cleared: string[][] = [];
  await page.route("**/api/downloads?*", (r) => r.fulfill({ json: { configured: true, speed: "12.3 M", paused: false, items } }));
  await page.route("**/api/downloads/clear", async (r) => {
    const ids: string[] = r.request().postDataJSON().ids;
    cleared.push(ids);
    const gone = ids.length ? ids : items.filter((i) => i.done).map((i) => i.id);
    items = items.filter((i) => !gone.includes(i.id));
    await r.fulfill({ json: { removed: gone.length, kept: 0 } });
  });
  await page.goto("/search");
  await expect(page.locator("#dlList")).toBeHidden();                              // no longer part of the search
  await page.goto("/settings/transfers");
  await expect(page.locator("#settingsTitle")).toHaveText("Downloads");
  await expect(page.locator("#dlList .job")).toHaveCount(3);
  await page.locator("#dlHideDone").check();
  await expect(page.locator("#dlList .job")).toHaveCount(1);
  await page.locator("#dlHideDone").uncheck();
  await page.locator("#dlList .job", { hasText: "Buch D" }).locator(".icon-btn").click();   // one entry
  await expect(page.locator("#dlList .job")).toHaveCount(2);
  await page.locator("#dlClear").click();                                          // all finished – after asking
  await page.locator("#confirmDialog button[value=ok]").click();
  await expect(page.locator("#dlList .job")).toHaveCount(1);
  expect(cleared).toEqual([["d"], []]);
  await expect(page.locator("#dlClear")).toBeDisabled();
  expect(errors).toEqual([]);
});

test("Statistik: je Konto, ein Konto im Detail, Zeitraum wählbar, Monate als Balken und als Tabelle", async ({ page }) => {
  const errors = await open(page);
  await page.goto("/settings/stats");                                               // the old address still leads there
  await expect(page).toHaveURL(/\/stats$/);
  await expect(page.locator("#statsTitle")).toHaveText("Statistik");
  await expect(page.locator("#statsTiles .stat-tile")).toHaveCount(4);
  await expect(page.locator("#statsPeople")).toContainText("niemand etwas angefragt oder gehört");   // nothing asked or heard yet: no cards
  await expect(page.locator("#statsChart .col")).toHaveCount(12);
  const asked: string[] = [];
  page.on("request", (r) => { if (r.url().includes("/api/stats")) asked.push(new URL(r.url()).searchParams.get("period") || ""); });
  await page.locator("#statsPeriod").getByText("Gesamt").click();
  await expect.poll(() => asked).toContain("all");
  await page.locator(".stats-months summary").click();
  await expect(page.locator("#statsMonthRows tr")).toHaveCount(12);
  // an account's own page: its requests and what it heard (from a card, here directly)
  await page.goto("/stats/1");
  await expect(page.locator("#statsTitle")).toHaveText(/Statistik: admin/);
  await expect(page.locator("#statsAccounts")).toBeHidden();
  await expect(page.locator("#statsDetail")).toBeVisible();
  await page.locator("#statsBack").click();
  await expect(page.locator("#statsAccounts")).toBeVisible();
  await page.locator("#statsPeriod").getByText("12 Monate").click();               // back for the next tests
  expect(errors).toEqual([]);
});

test("Hörverlauf: Audiobookshelf im Konto verbinden, Für dich sagt, wie es persönlich wird", async ({ page }) => {
  let linked = false;
  const sent: { username: string; password: string }[] = [];
  await page.route("**/api/me/listening", (r) => r.fulfill({ json: {
    available: { abs: true, plex: true },
    links: { abs: linked ? { id: "u-1", name: "admin-abs" } : null, plex: null },
    recent: linked ? { linked: ["abs"], seconds: 5 * 3600, books: 2, finished: 1, last: null, errors: [] } : null } }));
  await page.route("**/api/me/listening/abs", (r) => {
    const body = r.request().postDataJSON();
    sent.push(body);
    if (body.password !== "richtig") return r.fulfill({ status: 400, json: { error: "Audiobookshelf: Benutzername oder Passwort falsch" } });
    linked = true;
    return r.fulfill({ json: { links: { abs: { id: "u-1", name: "admin-abs" }, plex: null } } });
  });
  const errors = await open(page);
  await page.locator("nav.tabs").getByRole("link", { name: "Entdecken" }).click();
  await expect(page.locator("#discoverPersonal")).toContainText("Persönlich werden sie");
  await page.goto("/account");
  const box = page.locator("#listenBox");
  await expect(page.locator("#listenFieldset")).toBeVisible();
  await expect(box).toContainText("Plex-Profil ordnet ein Admin");
  await box.getByLabel("Benutzername in Audiobookshelf").fill("admin-abs");
  await box.getByLabel("Passwort in Audiobookshelf").fill("falsch");
  await box.getByRole("button", { name: "Verbinden" }).click();
  await expect(page.locator("#toast")).toContainText("Passwort falsch");
  await box.getByLabel("Passwort in Audiobookshelf").fill("richtig");
  await box.getByLabel("Passwort in Audiobookshelf").press("Enter");
  await expect(box).toContainText("Verbunden mit dem Audiobookshelf-Konto „admin-abs“");
  await expect(box).toContainText("5 h gehört, 2 Hörbücher");
  expect(sent.map((x) => x.password)).toEqual(["falsch", "richtig"]);
  expect(errors.filter((e) => !e.includes("status of 400"))).toEqual([]);   // the wrong password on purpose
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
  await expect(page.locator("#sessionList")).toContainText("Dieses Gerät");
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
  await mockAnyDetail(mia);
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
  // sorted by size: the small one first, then back to "best first"
  await page.locator("#releasesSort").getByRole("button", { name: "Kleinste" }).click();
  await expect(rows.first()).toContainText("GEKUERZT");
  await page.locator("#releasesSort").getByRole("button", { name: "Beste" }).click();
  await expect(rows.first()).toContainText("Sprache passt");
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
  // her statistics: herself, everyone together without names, her own requests – not the admin's page of accounts
  await mia.locator("#statsBtn").click();
  await expect(mia.locator("#statsTilesHead")).toHaveText("Du");
  await expect(mia.locator("#statsTotalBox")).toBeVisible();
  await expect(mia.locator("#statsAccounts")).toBeHidden();
  await expect(mia.locator("#statsRequests")).toContainText("Die leise Stadt");
  expect((await mia.request.get("/api/stats")).ok()).toBe(true);
  expect(JSON.stringify(await (await mia.request.get("/api/stats")).json())).not.toContain("admin");
  expect((await mia.request.get("/api/requests?scope=all")).status()).toBe(200);
  // a 502 is the cover cache failing to reach the made-up cover address of this offline test – the card falls back to a stand-in tile
  expect(admin.concat(miaErrors).filter((e) => !/40[34]|status of 502/.test(e))).toEqual([]);
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
  await expect(row).toContainText("Offen");
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
  await expect(row).toContainText("Benutzt");
  await expect(row).toContainText("von Oma");
  // the members list says where Oma came from, and that she was here just now – not "never"
  const member = page.locator("#userRows tr", { hasText: "Oma" });
  await expect(member).toContainText("Über Einladung „Oma“");
  await expect(member).not.toContainText("Noch nie");
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
  await expect(ole).toHaveURL(/\/discover$/);                   // signed in: the start page ("/" leads there)
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

test("App auf dem Home-Bildschirm: Manifest und Symbole ohne Anmeldung, sonst nichts", async ({ request }) => {
  const manifest = await request.get("/manifest.webmanifest");
  expect(manifest.status()).toBe(200);
  expect(manifest.headers()["content-type"]).toContain("manifest+json");
  const m = await manifest.json();
  expect(m.name).toBe("Hörwerk");
  expect(m.display).toBe("standalone");
  for (const icon of [...m.icons.map((i: { src: string }) => i.src), "/apple-touch-icon.png"]) {
    const r = await request.get(icon);
    expect(r.status(), icon).toBe(200);
    expect(r.headers()["content-type"]).toBe("image/png");
  }
  expect((await request.get("/api/settings")).status()).toBe(401);                 // the data stays behind the sign-in
  expect((await request.get("/books", { maxRedirects: 0 })).status()).toBe(303);   // so does every page
});

test("Handy: Tab-Leiste unten, Mehr führt zu Statistik, Konto und Abmelden", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const errors = await open(page);
  const tabs = page.locator("nav.tabs");
  expect(await tabs.evaluate((e) => getComputedStyle(e).position)).toBe("fixed");
  expect((await tabs.boundingBox())!.y).toBeGreaterThan(700);                       // at the bottom, where the thumb is
  await tabs.getByRole("link", { name: "Mehr" }).click();
  await expect(page).toHaveURL(/\/more$/);
  const menu = page.locator("#viewMore");
  await expect(menu.getByRole("link", { name: /Protokoll/ })).toBeVisible();
  await expect(menu.getByRole("link", { name: /Einstellungen/ })).toBeVisible();
  await expect(menu.getByRole("button", { name: "Abmelden" })).toBeVisible();
  await menu.getByRole("link", { name: /Statistik/ }).click();
  await expect(page).toHaveURL(/\/stats$/);
  await expect(tabs.getByRole("link", { name: "Mehr" })).toHaveAttribute("aria-current", "page");   // still "behind Mehr"
  expect(errors).toEqual([]);
});

test("Protokoll: Zeitraum wählbar, die letzte Stunde ist voreingestellt", async ({ page }) => {
  const errors = await open(page);
  await page.goto("/settings/logs");
  const win = page.locator("#logWindow");
  await expect(win.getByRole("button", { name: "60 Min." })).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator("#logView .log-line").first()).toBeVisible();           // what happened in the last hour
  await win.getByRole("button", { name: "15 Min." }).click();
  await expect(win.getByRole("button", { name: "15 Min." })).toHaveAttribute("aria-pressed", "true");
  await win.getByRole("button", { name: "Alles" }).click();
  await expect(win.getByRole("button", { name: "Alles" })).toHaveAttribute("aria-pressed", "true");
  await page.reload();
  await expect(win.getByRole("button", { name: "Alles" })).toHaveAttribute("aria-pressed", "true");   // remembered
  expect(errors).toEqual([]);
});

const detailBook = (asin: string, title: string) => ({ asin, title, authors: ["Anna Autorin"], narrators: ["Sven Sprecher"], rating: 4.5, ratings: 80,
  source: "audible", runtime_min: 300, release_date: "2020-01-01", language: "german", genres: ["Krimi"], cover_url: null });

async function mockDetail(page: Page): Promise<void> {
  const items = [detailBook("B0DETAIL01", "Das erste Hörbuch"), detailBook("B0DETAIL02", "Das zweite Hörbuch")];
  await page.route("**/api/discover?*", (r) => r.fulfill({ json: { items, updated: 1_700_000_000, error: null, running: false, personal: true, linked: true } }));
  await page.route("**/api/book/detail?*", (r) => {
    const asin = new URL(r.request().url()).searchParams.get("asin");
    const b = items.find((x) => x.asin === asin)!;
    r.fulfill({ json: { book: { ...b, description: `Beschreibung von ${b.title}` }, similar: items.filter((x) => x.asin !== asin), region: "de" } });
  });
}

test("Entdecken am Mac: Detailspalte mit Beschreibung und „Ähnlich wie“ – ein ähnliches Hörbuch öffnet sich dort", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await mockDetail(page);
  const errors = await open(page);
  await page.locator("nav.tabs").getByRole("link", { name: "Entdecken" }).click();
  const col = page.locator("#bookDetail");
  await expect(col.locator(".detail-title")).toHaveText("Das erste Hörbuch");                  // wide: the first one is shown at once
  await expect(col.locator(".detail-blurb")).toHaveText("Beschreibung von Das erste Hörbuch");
  await expect(col.locator(".detail-similar")).toContainText("Ähnlich wie „Das erste Hörbuch“");
  await col.locator(".sim", { hasText: "Das zweite Hörbuch" }).click();
  await expect(col.locator(".detail-title")).toHaveText("Das zweite Hörbuch");
  await col.getByRole("button", { name: /Zurück/ }).click();
  await expect(col.locator(".detail-title")).toHaveText("Das erste Hörbuch");
  await page.locator("nav.tabs").getByRole("link", { name: "Suchen" }).click();                   // the book belongs to the page it was opened on
  await expect(col).toBeHidden();
  expect(errors).toEqual([]);
});

test("Entdecken auf dem Handy: Details als Blatt von unten, Escape schließt es", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await mockDetail(page);
  const errors = await open(page);
  await page.locator("nav.tabs").getByRole("link", { name: "Entdecken" }).click();
  const sheet = page.locator("#bookDetail");
  await expect(page.locator(".suggestion")).toHaveCount(2);
  await expect(sheet).toBeHidden();                                                                // narrow: nothing opens by itself
  await page.locator(".suggestion .s-title", { hasText: "Das zweite Hörbuch" }).click();
  await expect(sheet.locator(".detail-title")).toHaveText("Das zweite Hörbuch");
  expect(await sheet.evaluate((e) => getComputedStyle(e).position)).toBe("fixed");
  await expect(sheet.locator(".detail-blurb")).toHaveText("Beschreibung von Das zweite Hörbuch");
  await page.keyboard.press("Escape");
  await expect(sheet).toBeHidden();
  expect(errors).toEqual([]);
});

test("Handy: das Detail-Blatt ist bis unten in der Farbe des Hörbuchs, auch beim Scrollen", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const item = { asin: "B0TEST0001", title: "Hier geht's lang!", authors: ["Elke Heidenreich"], narrators: ["Elke Heidenreich"], rating: 4.9, ratings: 266,
    runtime_min: 198, release_date: "2021-01-01", cover_url: null, source: "audible", genres: ["Biografien & Erinnerungen"] };
  await page.route("**/api/discover/genres", (r) => r.fulfill({ json: { genres: [] } }));     // no chip rows pushing the card under the tab bar
  await page.route("**/api/discover?*", (r) => r.fulfill({ json: { items: [item], updated: 1, error: null, running: false } }));
  await page.route("**/api/book/detail?*", (r) => r.fulfill({ json: { book: { ...item, description: "Eine lebenslange Liebesgeschichte. ".repeat(8) },
    similar: [1, 2, 3, 4].map((n) => ({ ...item, asin: `B0SIM0000${n}`, title: `Ähnlich ${n}` })), region: "de" } }));
  const errors = await open(page);
  await page.locator("nav.tabs").getByRole("link", { name: "Entdecken" }).click();
  await expect(page.locator(".suggestion")).toHaveCount(1);
  await page.locator(".suggestion .s-title").click();
  const sheet = page.locator("#bookDetail");
  await expect(sheet.locator(".detail-similar")).toBeVisible();
  const [scroll, inner] = await sheet.evaluate((s) => [s.scrollHeight, s.querySelector(".detail-inner")!.getBoundingClientRect().height]);
  expect(inner).toBeGreaterThanOrEqual(scroll - 1);                        // the tinted box is not cut off at the sheet's height
  expect(errors).toEqual([]);
});

test("Detailspalte: ein Wechsel auf eine andere Seite schließt sie, am Mac bleiben Menü und Spalte beim Scrollen stehen", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 700 });
  const items = Array.from({ length: 12 }, (_, i) => ({ asin: `B0TEST00${String(i).padStart(2, "0")}`, title: `Hörbuch ${i}`, authors: ["Anna Autorin"],
    rating: 4, ratings: 10, runtime_min: 100, cover_url: null, source: "review", review: { source: "Perlentaucher", link: "https://example.org", date: "" } }));
  await page.route("**/api/discover/genres", (r) => r.fulfill({ json: { genres: [] } }));
  await page.route("**/api/discover?*", (r) => r.fulfill({ json: { items, updated: 1, error: null, running: false } }));
  await mockAnyDetail(page);
  const errors = await open(page);
  await page.locator("nav.tabs").getByRole("link", { name: "Entdecken" }).click();
  const col = page.locator("#bookDetail");
  await expect(col).toBeVisible();                                                   // wide: the first one opens by itself
  await expect(page.locator(".suggestion").first()).not.toContainText("via review");
  await page.mouse.wheel(0, 2000);
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBeGreaterThan(300);
  for (const sel of ["#bookDetail", ".topbar"]) {
    expect(await page.locator(sel).evaluate((e) => e.getBoundingClientRect().top), sel).toBe(0);      // still at the top of the window
  }
  await page.locator("nav.tabs").getByRole("link", { name: "Suchen" }).click();
  await expect(col).toBeHidden();
  expect(errors).toEqual([]);
});

test("Neue Version auf dem Server: ein offener Tab bietet einmal das Neuladen an", async ({ page }) => {
  const errors = await open(page);
  await expect(page.locator("#updateBar")).toHaveCount(0);                          // same version: nothing
  await page.route("**/api/health", (r) => r.fulfill({ json: { ok: true, version: "99.0.0" } }));
  await page.evaluate(() => { versionChecked = 0; document.dispatchEvent(new CustomEvent("livechange", { detail: true })); });
  const bar = page.locator("#updateBar");
  await expect(bar).toContainText("Hörwerk wurde aktualisiert (99.0.0).");
  await bar.getByRole("button", { name: "Schließen" }).click();
  await expect(bar).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("Statistik: Hörer ohne Hörwerk-Konto zählen mit, Audiobookshelf und Plex lassen sich zusammenführen", async ({ page }) => {
  const months = Array.from({ length: 12 }, (_, i) => `2026-${String(i + 1).padStart(2, "0")}`);
  const blank = { requests: 0, books: 0, available: 0, open: 0, declined: 0, failed: 0, series: 0, minutes: 0, minutes_available: 0, bytes: 0,
    english: 0, wait_median: null, last: null, top_genres: [], genres: [] };
  const listen = (sec: number) => ({ seconds: sec, books: 2, finished: 1, last: 1_700_000_000, linked: [], errors: [], genres: [{ name: "Krimis & Thriller", books: 2 }] });
  await page.route(/\/api\/stats(\?|$)/, (r) => r.fulfill({ json: { scope: "all", period: "365", since: null,
    users: [{ ...blank, id: 1, name: "admin", role: "admin", listen: null }, { ...blank, id: 2, name: "Bob", role: "user", books: 1, minutes: 60, listen: null },
      { ...blank, id: null, name: "Oma", role: "listener", listen: listen(36000), listener: { key: "abs:o", person: null, abs: true, plex: false } }],
    total: { ...blank, id: null, name: "", role: "user" }, months: months.map((m) => ({ month: m, books: 0, minutes: 0 })), library_books: 3,
    listen: { seconds: 36000, books: 2, finished: 1, linked: 0, listeners: 1 }, listen_months: months.map((m) => ({ month: m, seconds: 3600 })),
    listen_genres: [{ name: "Krimis & Thriller", books: 2 }] } }));
  let merged: unknown = null;
  await page.route("**/api/listening/accounts", (r) => r.fulfill({ json: { available: { abs: true, plex: true }, errors: [],
    abs: [{ id: "o", name: "oma" }], plex: [{ id: "9", name: "Oma Plex" }], users: [{ id: 1, name: "admin", links: { abs: null, plex: null } }], people: [] } }));
  await page.route("**/api/listening/people", (r) => { merged = r.request().postDataJSON(); return r.fulfill({ json: { people: [] } }); });
  const errors = await open(page);
  await page.goto("/stats");
  const oma = page.locator(".person", { hasText: "Oma" });
  await expect(oma).toContainText("Audiobookshelf");
  await expect(oma).toContainText("10 h");
  await expect(oma.locator("a")).toHaveCount(0);                                     // no account page for a listener
  await expect(page.locator(".person", { hasText: "admin" })).toHaveCount(0);       // nothing asked, nothing heard: no card
  await expect(page.locator(".person", { hasText: "Bob" })).toHaveCount(1);
  await expect(page.locator("#statsGenresNote")).toHaveText("der gehörten Hörbücher");
  await page.locator("#statsMerge summary").click();
  await page.locator("#mergeAbs").selectOption("o");
  await page.locator("#mergePlex").selectOption("9");
  await page.locator("#mergeName").fill("Oma");
  await page.locator("#mergeGo").click();
  await expect.poll(() => merged).toEqual({ abs: "o", plex: "9", name: "Oma" });
  expect(errors).toEqual([]);
});

test("Werkbank: links was gerade läuft, auf dem Handy eine Leiste über den Tabs", async ({ page }) => {
  const req = (id: number, title: string, status: string, live: object) => ({ id, book: { title, authors: ["A"] }, status, created: 1, updated: 1, mine: true, live });
  let items = [req(1, "Afrika", "downloading", { kind: "download", state: "Downloading", progress: 0.42, mb: 1000, mbleft: 580 }),
    req(2, "Krabat", "processing", { kind: "import", state: "converting", progress: 0.71 })];
  await page.route("**/api/requests", (r) => r.fulfill({ json: { items, may_search: true } }));
  await page.setViewportSize({ width: 1440, height: 900 });
  const errors = await open(page);
  const bench = page.locator("#workbench");
  await expect(bench.locator(".wb-item")).toHaveCount(2);
  await expect(bench.locator(".wb-item").first()).toContainText("Afrika");
  await expect(bench.locator(".wb-item").first()).toContainText("Lädt · 42 %");
  await expect(bench.locator(".wb-item").nth(1)).toContainText("Wandelt in M4B um · 71 %");
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(bench).toBeVisible();
  await expect(bench.locator(".wb-plus")).toHaveText(" · +1");
  items = [];
  await page.evaluate(() => pollBench());
  await expect(bench).toBeHidden();                                                  // phone: nothing on the bench, no bar
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(bench).toContainText("Gerade nichts in Arbeit");
  expect(errors).toEqual([]);
});

test("Löschen: Anfrage mit laufendem Download abbrechen, fertiges Hörbuch auch aus der Bibliothek, Fertige nur ausblenden", async ({ page }) => {
  const req = (id: number, title: string, status: string, live: object | null) => ({ id, book: { title, authors: ["A"] }, status, created: 1, updated: 1, mine: true, live });
  const deleted: string[] = [];
  await page.route("**/api/requests", (r) => r.fulfill({ json: { items: [
    req(1, "Afrika", "downloading", { kind: "download", state: "Downloading", progress: 0.4 }),
    req(2, "Krabat", "available", null)], may_search: true, sees_all: true, pending: 0, interval: 6,
    quota: { limit: null, used: 0, left: null, period: "month" } } }));
  await page.route(/\/api\/requests\/\d+/, (r) => { deleted.push(new URL(r.request().url()).pathname + new URL(r.request().url()).search); return r.fulfill({ json: { ok: true, deleted: 1 } }); });
  const errors = await open(page);
  await page.goto("/requests");
  const afrika = page.locator("li.request", { hasText: "Afrika" });
  await afrika.getByTitle("Abbrechen und löschen").click();
  await expect(page.locator("#confirmText")).toContainText("Download abbrechen und Anfrage löschen?");
  await page.locator("#confirmOk").click();
  await expect.poll(() => deleted).toContain("/api/requests/1");
  await page.getByRole("button", { name: /^Verfügbar/ }).click();                    // a book that arrived
  const krabat = page.locator("li.request", { hasText: "Krabat" });
  await krabat.getByTitle("Anfrage löschen").click();
  await page.locator("#confirmOk").click();                                           // delete the request …
  await expect(page.locator("#confirmText")).toContainText("Auch das Hörbuch „Krabat“ aus der Bibliothek löschen?");
  await page.locator("#confirmOk").click();                                           // … and the book
  await expect.poll(() => deleted).toContain("/api/requests/2?files=1");
  // "Fertige ausblenden" only hides: a second click shows them again
  await page.unroute("**/api/requests");
  await page.goto("/books");
  await page.locator("#bookFilter").getByRole("button", { name: /Aufträge/ }).click();
  const done = page.locator("#bookList .book.done");
  const before = await done.count();
  if (before) {
    await page.locator("#clearBooksBtn").click();
    await expect(done).toHaveCount(0);
    await expect(page.locator("#clearBooksBtn")).toHaveText("Fertige einblenden");
    await page.locator("#clearBooksBtn").click();
    await expect(done).toHaveCount(before);
  }
  expect(errors).toEqual([]);
});

test("Prüfen: ein anderes Buch als angefragt wird nicht einfach abgelegt – löschen und weitersuchen", async ({ page }) => {
  const job = { id: "job1", source: "/inbox/x", rel: "x", name: "Homer-De Odyssee op Koelsch-WALKMAN", origin: "inbox", status: "review", stage: "convert",
    message: "1 mögliche Treffer, keiner eindeutig", progress: 0, files: ["01.mp3"], duration: 3600, hints: { request_id: 7 },
    candidates: [{ id: "c1", asin: "B0KOELSCH1", title: "De Odyssee op Kölsch", authors: ["Homer"], region: "de", source: "audible", match: { score: 0.9 } }],
    match: null, region: "de", dest: null, error: null, converted_info: {}, duplicate: null };
  await page.route(/\/api\/audiobooks(\?.*)?$/, (r) => r.fulfill({ json: { enabled: true, inbox: "/inbox", library: "/library", inbox_ok: true, library_ok: true,
    last_scan: 1, scan_note: "", waiting: [], jobs: [job], review: 1 } }));
  await page.route("**/api/events*", (r) => r.abort());                              // no live updates over the mocked list
  let chosen = 0, wrong = 0;
  await page.route("**/api/audiobooks/job1/choose", (r) => { chosen++; return r.fulfill({ status: 409, json: { error: "„De Odyssee op Kölsch“ ist nicht das angefragte Hörbuch „Odyssee“." } }); });
  await page.route("**/api/audiobooks/job1/wrong-release", (r) => { wrong++; return r.fulfill({ json: { ok: true } }); });
  const errors = watchErrors(page);
  const res = await page.request.post("/api/auth/login", { headers: OWN, data: { ...ADMIN, remember: false } });
  expect(res.status()).toBe(200);
  await page.goto("/books");
  await page.locator("#bookFilter").getByRole("button", { name: /Zu prüfen/ }).click();
  await page.locator("#bookList .book.review").getByRole("button", { name: "Übernehmen" }).click();
  await expect(page.locator("#confirmText")).toContainText("ist nicht das angefragte Hörbuch „Odyssee“");
  await page.locator("#confirmOk").click();
  await expect.poll(() => wrong).toBe(1);
  expect(chosen).toBe(1);
  expect(errors.filter((e) => !e.includes("409") && !e.includes("ERR_FAILED"))).toEqual([]);
});

test("Fassungen: das Releases-Fenster zeigt die anderen Aufnahmen, ein unpassender Download fragt – verwerfen oder behalten", async ({ page }) => {
  await page.route("**/api/events*", (r) => r.abort());
  const request = { id: 9, book: { title: "Odyssee", authors: ["Homer"], narrators: ["Manfred Schradi"], release_date: "2013-05-01", runtime_min: 735 },
    status: "searching", created: 1, updated: 1, mine: true, live: null };
  await page.route("**/api/requests", (r) => r.fulfill({ json: { items: [request], may_search: true, sees_all: true, pending: 0, interval: 6,
    quota: { limit: null, used: 0, left: null } } }));
  await page.route("**/api/requests/9/releases", (r) => r.fulfill({ json: { query: "Homer Odyssee", queries: ["Homer Odyssee"], errors: [],
    editions: [{ title: "Odyssee", narrators: ["Christian Brückner"], year: 2009, runtime_min: 1014 }],
    results: [{ id: "r1", title: "Homer.-.Odyssee.DE.2009.ABook.MP3-PaZ", size: 1, indexer: "x",
      verdict: { ok: false, score: 0, reason: "Andere Fassung: 2009 erschien die von Christian Brückner (2009)", notes: [], kbps: null } }] } }));
  const job = { id: "job2", source: "/inbox/y", rel: "y", name: "Homer.-.Odyssee.DE.2009", origin: "inbox", status: "review", stage: "convert",
    message: "", progress: 0, files: ["01.mp3"], duration: 60840, hints: { request_id: 9, request_conflict: "Die Länge passt zur Fassung von Christian Brückner (2009), nicht zur angefragten" },
    candidates: [], match: null, region: "de", dest: null, error: null, converted_info: {}, duplicate: null };
  await page.route(/\/api\/audiobooks(\?.*)?$/, (r) => r.fulfill({ json: { enabled: true, inbox: "/inbox", library: "/library", inbox_ok: true, library_ok: true,
    last_scan: 1, scan_note: "", waiting: [], jobs: [job], review: 1 } }));
  let kept = 0;
  await page.route("**/api/audiobooks/job2/keep", (r) => { kept++; return r.fulfill({ json: job }); });
  const errors = watchErrors(page);
  expect((await page.request.post("/api/auth/login", { headers: OWN, data: { ...ADMIN, remember: false } })).status()).toBe(200);
  await page.goto("/requests");
  await page.locator("li.request", { hasText: "Odyssee" }).getByRole("button", { name: "Releases" }).click();
  const box = page.locator("#releasesEditions");
  await expect(box).toContainText("2 Fassungen");
  await expect(box.locator("li.wanted")).toContainText("Manfred Schradi");
  await expect(box).toContainText("Christian Brückner");
  await expect(page.locator("#releasesRows")).toContainText("Andere Fassung");
  await page.keyboard.press("Escape");
  await page.goto("/books");
  await page.locator("#bookFilter").getByRole("button", { name: /Zu prüfen/ }).click();
  const conflict = page.locator("#bookList .conflict-box");
  await expect(conflict).toContainText("Passt nicht zur Anfrage: Die Länge passt zur Fassung von Christian Brückner");
  await conflict.getByRole("button", { name: /Behalten/ }).click();
  await expect.poll(() => kept).toBe(1);
  expect(errors.filter((e) => !e.includes("ERR_FAILED"))).toEqual([]);
});

test("Bibliothek: Live-Updates der Aufträge bauen die Liste nicht neu (kein Sprung nach oben beim Scrollen)", async ({ page }) => {
  const errors = await open(page);
  await page.goto("/books");
  await page.locator("#bookFilter .chip").first().click();                                  // Bibliothek
  await expect(page.locator(".lib-row").first()).toBeVisible();
  await page.evaluate(() => { document.querySelector(".lib-row")!.setAttribute("data-mark", "1"); });
  // a conversion's progress arrives: the jobs change, the library stays as it is
  await page.evaluate(() => { const d = JSON.parse(JSON.stringify(books.data)); d.scan_note = String(Math.random()); applyBooks(d); });
  await expect(page.locator(".lib-row[data-mark='1']")).toHaveCount(1);
  expect(errors).toEqual([]);
});

test("Robust: kaputter Einladungslink und unbekannte Adressen zeigen Seiten statt JSON, lange Suchbegriffe bringen den Server nicht zum Absturz", async ({ page, request }) => {
  await page.goto("/invite/abc");
  await expect(page.locator("body")).toContainText("Diese Einladung gilt nicht (mehr)");
  await page.goto("/gibt-es-nicht");
  await expect(page).toHaveURL(/\/login$/);                                         // the start (not signed in: the sign-in)
  const errors = await open(page);
  const res = await page.request.post("/api/search/books", { headers: OWN, data: { query: "a".repeat(5000) } });
  expect(res.status()).toBeLessThan(500);
  expect((await request.get("/api/health")).status()).toBe(200);                    // still there
  expect(errors).toEqual([]);
});

test("Hörbücher: Details aus der Bibliothek öffnen", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await mockAnyDetail(page);
  const errors = await open(page);
  await page.goto("/books");
  await page.locator("#bookFilter .chip").first().click();                                  // Bibliothek
  const row = page.locator(".lib-row", { hasText: "Altes Buch" });
  await expect(row).toBeVisible();
  await row.locator(".lib-open").click();
  const col = page.locator("#bookDetail");
  await expect(col.locator(".detail-title")).toHaveText("Altes Buch");
  await expect(col).toContainText("Max Muster");
  await expect(col).toContainText("In der Bibliothek");                                       // owned: no request button
  await expect(col.getByRole("button", { name: "Anfragen" })).toHaveCount(0);
  await page.locator("nav.tabs").getByRole("link", { name: "Anfragen" }).click();
  await expect(col).toBeHidden();
  expect(errors).toEqual([]);
});

test("Statistik: Genres als Band und gehörte Stunden je Monat", async ({ page }) => {
  const months = ["2026-01", "2026-02", "2026-03", "2026-04", "2026-05", "2026-06", "2026-07", "2026-08", "2026-09", "2026-10", "2026-11", "2026-12"];
  const user = { id: null, name: "", role: "user", requests: 3, books: 3, available: 2, open: 1, declined: 0, failed: 0, series: 0, minutes: 600,
    minutes_available: 400, bytes: 1e9, english: 0, wait_median: 3600, last: 1_700_000_000, top_genres: ["Krimi"], listen: null,
    genres: [{ name: "Krimi", books: 6 }, { name: "Klassiker", books: 3 }, { name: "Märchen", books: 1 }] };
  await page.route(/\/api\/stats(\?|$)/, (r) => r.fulfill({ json: { scope: "all", period: "365", since: null, users: [], total: user,
    months: months.map((m) => ({ month: m, books: 1, minutes: 60 })), library_books: 3, listen: { seconds: 36000, books: 4, finished: 2, linked: 1 },
    listen_months: months.map((m, i) => ({ month: m, seconds: i === 10 ? 25 * 3600 : 3600 })) } }));
  const errors = await open(page);
  await page.goto("/stats");
  await expect(page.locator("#statsGenres .genre-ribbon b")).toHaveCount(3);
  await expect(page.locator("#statsGenres .genre-legend")).toContainText("Krimi 60 %");
  await expect(page.locator("#statsListenChart .col")).toHaveCount(12);
  await expect(page.locator("#statsListenChart .peak")).toHaveText("25");                    // the month with most hours
  expect(errors).toEqual([]);
});
