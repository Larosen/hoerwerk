// README screenshots and demo GIF, English and German, against the real server with a demo library.
//
//   npm run build && (cd rust && cargo build --release) && node scripts/screenshots/shoot.mjs
//
// The library, requests and suggestions are mocked in the browser: public-domain books, their covers made of
// public-domain artwork from Wikimedia Commons (art.mjs, credits in docs/screenshots/CREDITS.md) and own typography. Needs ffmpeg for the GIF. PW_CHROMIUM: another Chromium binary.
import { spawn, execFileSync } from "node:child_process";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { chromium } from "@playwright/test";
import { BOOKS } from "./books.mjs";
import { renderCovers } from "./covers.mjs";
import { creditsMarkdown, fetchArt } from "./art.mjs";

const PORT = 8097;
const BASE = `http://127.0.0.1:${PORT}`;
const ROOT = "e2e/.run";
const OWN = { "X-Requested-With": "Hoerwerk" };
const NOW = Math.floor(Date.now() / 1000);
const H = 3600;

execFileSync("node", ["e2e/setup.mjs"], { stdio: "inherit" });
const server = spawn("rust/target/release/hoerwerk", [], {
  stdio: "ignore",
  env: {
    ...process.env, PORT: String(PORT), CONFIG_DIR: `${ROOT}/config`, INBOX_DIR: `${ROOT}/inbox`, LIBRARY_DIR: `${ROOT}/library`,
    MATCH_MODE: "off", METADATA_FALLBACK: "false", LIBRARY_AUTOFIX: "false", AUTH_USER: "admin", AUTH_PASS: "E2E-Admin-Kaffee-1",
  },
});
for (let i = 0; i < 60; i++) {
  try { if ((await fetch(`${BASE}/api/health`)).ok) break; } catch { /* starting */ }
  await new Promise((r) => setTimeout(r, 500));
}

const browser = await chromium.launch(process.env.PW_CHROMIUM ? { executablePath: process.env.PW_CHROMIUM } : {});
const art = await fetchArt(BOOKS);
writeFileSync("docs/screenshots/CREDITS.md", creditsMarkdown(BOOKS, art));
const covers = await renderCovers(browser, BOOKS, art);
const byTitle = (t) => BOOKS.find((b) => b.title === t);
const coverUrl = (b) => `https://covers.invalid/${b.id}.png`;

// -- the demo data -----------------------------------------------------------
const library = BOOKS.map((b, i) => ({
  path: `${b.author}/${b.series ? b.series + "/" + b.pos + " - " : ""}${b.title}`, title: b.title, authors: [b.author], series: b.series,
  series_position: b.pos, asin: null, format: "m4b", files: 1, cover: true, mtime: 1_700_000_000 + i, settled: true,
}));

const NARRATORS = ["Jonas Kellerer", "Mara Eilers", "Tobias Rennert", "Lena Marwitz", "Walter Dreyer", "Ida Brenner"];
const YEARS = { "Der Hund der Baskervilles": 1902, "Dracula": 1897, "Frankenstein": 1818, "Moby Dick": 1851, "Heidi": 1880, "Buddenbrooks": 1901, "Effi Briest": 1895,
  "Die Zeitmaschine": 1895, "Der Krieg der Welten": 1898, "Die Schatzinsel": 1883, "Emma": 1815, "Jane Eyre": 1847, "Der Zauberberg": 1924 };
const BLURBS = {
  "Der Hund der Baskervilles": "Ein Fluch, ein Moor und ein Hund, der nachts heult: Sherlock Holmes und Dr. Watson ermitteln auf Baskerville Hall.",
  "Dracula": "Ein junger Anwalt reist nach Transsilvanien – und bringt etwas mit nach England, das nicht sterben will.",
  "Die Schatzinsel": "Eine Karte, ein Schiff voller Piraten und ein Junge, der den Schatz zuerst finden will.",
};
const suggestion = (title, i, because) => {
  const b = byTitle(title);
  return {
    asin: `B0DEMO${String(i).padStart(4, "0")}`, title: b.title, authors: [b.author], narrators: [NARRATORS[i % NARRATORS.length]],
    rating: [4.6, 4.7, 4.4, 4.5, 4.8, 4.3][i % 6], ratings: [1280, 2140, 860, 640, 3020, 410][i % 6],
    runtime_min: [372, 938, 368, 1140, 412, 515][i % 6], release_date: `${YEARS[title] || 1890}-01-01`, cover_url: coverUrl(b),
    url: "", because, source: "audible", genres: [b.genre], language: "german",
    description: BLURBS[title] || `${b.title} von ${b.author} – ungekürzt gelesen.`,
  };
};
const discover = [
  suggestion("Der Hund der Baskervilles", 0, ["Eine Studie in Scharlachrot", "Das Zeichen der Vier"]),
  suggestion("Dracula", 1, ["Frankenstein"]),
  suggestion("Die Schatzinsel", 2, ["Moby Dick"]),
  suggestion("Effi Briest", 3, ["Buddenbrooks"]),
  suggestion("Die Zeitmaschine", 4, ["Der Krieg der Welten"]),
  suggestion("Heidi", 5, ["Nils Holgersson"]),
  suggestion("Jane Eyre", 6, ["Emma"]),
  suggestion("Der Zauberberg", 7, ["Buddenbrooks"]),
];

function request(id, title, status, user, created, extra = {}) {
  const b = byTitle(title);
  return {
    id, status, user, mine: user === "admin", created: NOW - created, updated: NOW - created / 2, history: [],
    book: { title: b.title, authors: [b.author], cover_url: coverUrl(b), genres: [b.genre], language: "german" }, ...extra,
  };
}
function requests(progress = 0.62, arrived = false) {
  const items = [
    request(1, "Die Schatzinsel", "pending", "Mia", 0.3 * H),
    arrived
      ? request(2, "20.000 Meilen unter dem Meer", "available", "Jonas", 1.2 * H, { done: NOW - 30 })
      : request(2, "20.000 Meilen unter dem Meer", "downloading", "Jonas", 1.2 * H, {
        release: "Jules.Verne-20000.Meilen.unter.dem.Meer.Hoerbuch.Ungekuerzt",
        live: { kind: "download", state: "Downloading", progress, mb: 612, mbleft: Math.round(612 * (1 - progress)), left: `0:0${Math.max(1, Math.round(6 * (1 - progress)))}:10`, speed: 11800 },
      }),
    request(3, "Der Zauberberg", "processing", "Lena", 2 * H, { live: { kind: "import", state: "converting", progress: 0.34 } }),
    request(4, "Moby Dick", "searching", "admin", 5 * H, { last_search: NOW - 600, seen: 14 }),
    ...[["Dracula", "Mia", 3, 0.7], ["Frankenstein", "Jonas", 6, 1.5], ["Heidi", "Lena", 50, 41], ["Effi Briest", "admin", 75, 70],
      ["Emma", "Mia", 98, 92], ["Pinocchio", "Lena", 130, 121], ["Faust", "admin", 170, 165], ["Der Process", "Jonas", 220, 214]]
      .map(([t, u, asked, found], i) => request(10 + i, t, "available", u, asked * H, { done: NOW - found * H })),
  ];
  return { items, quota: { limit: null, used: 0, left: null }, sees_all: true, may_search: true, pending: 1, interval: 30 };
}

// -- one browser page with all mocks -----------------------------------------
let state = { progress: 0.62, arrived: false };
async function openPage(lang, viewport, scale) {
  const ctx = await browser.newContext({ viewport, deviceScaleFactor: scale, locale: lang === "de" ? "de-DE" : "en-GB" });
  await ctx.addInitScript((seen) => {
    localStorage.setItem("br:requestsSeen", String(seen));
    localStorage.setItem("br:requestFilter", JSON.stringify("all"));
    localStorage.setItem("br:bookFilter", JSON.stringify("library"));
    localStorage.setItem("br:libraryView", JSON.stringify("regal"));
  }, NOW - 2 * H);
  const page = await ctx.newPage();
  const res = await page.request.post(`${BASE}/api/auth/login`, { headers: OWN, data: { name: "admin", password: "E2E-Admin-Kaffee-1", remember: false } });
  if (!res.ok()) throw new Error("login");
  await page.request.put(`${BASE}/api/settings`, { headers: OWN, data: { language: lang } });
  await page.request.post(`${BASE}/api/audiobooks/scan`, { headers: OWN });          // status texts in the new language
  // the demo shows the shelf itself: no tidy-up tiles, no selection boxes (the admin tools of the real library)
  await page.addInitScript(() => addEventListener("DOMContentLoaded", () => {
    const s = document.createElement("style");
    s.textContent = "#libTiles, #libBar, .lib-check { display: none !important }";
    document.head.append(s);
  }));
  const png = (r, id) => r.fulfill({ contentType: "image/png", body: covers.get(id) });
  await page.route("**/api/cover?*", (r) => png(r, new URL(r.request().url()).searchParams.get("u").match(/(b\d\d)\.png/)[1]));
  await page.route("**/api/library/cover?*", (r) => {
    const path = new URL(r.request().url()).searchParams.get("path");
    png(r, BOOKS[library.findIndex((b) => b.path === path)].id);
  });
  await page.route("**/api/library/books?*", (r) => r.fulfill({ json: { total: library.length, books: library } }));
  await page.route("**/api/requests", (r) => r.request().method() === "GET" ? r.fulfill({ json: requests(state.progress, state.arrived) }) : r.continue());
  await page.route("**/api/discover?*", (r) => r.fulfill({ json: { items: discover, updated: NOW - 3600, error: null, running: false, personal: true, linked: true } }));
  await page.route("**/api/book/detail?*", (r) => {
    const s = discover.find((x) => x.asin === new URL(r.request().url()).searchParams.get("asin")) || discover[0];
    r.fulfill({ json: { book: s, similar: discover.filter((x) => x !== s).slice(0, 6), region: "de" } });
  });
  return page;
}

const settle = (page, ms = 900) => page.waitForTimeout(ms);
async function covered(page) {        // every cover in view has loaded (lazy ones further down load later)
  await page.waitForFunction(() => [...document.querySelectorAll("img")].every((i) => {
    const r = i.getBoundingClientRect();
    return i.complete || r.bottom < 0 || r.top > innerHeight || r.width === 0;
  }), null, { timeout: 15000 });
  await settle(page, 400);
}

for (const lang of ["en", "de"]) {
  const dir = `docs/screenshots/${lang}`;
  mkdirSync(dir, { recursive: true });
  state = { progress: 0.62, arrived: false };

  // stills: 1440×900 at 1.4× (about 2000 px wide)
  let page = await openPage(lang, { width: 1440, height: 900 }, 1.4);
  await page.goto(`${BASE}/books`);
  await page.waitForSelector(".lib-shelf, .shelf, .lib-row, .lib-book", { timeout: 15000 }).catch(() => {});
  await covered(page);
  await page.screenshot({ path: `${dir}/library.png` });
  await page.goto(`${BASE}/requests`);
  await page.waitForSelector("#requestList li.request");
  await covered(page);
  await page.screenshot({ path: `${dir}/requests.png` });
  await page.goto(`${BASE}/discover`);
  await page.waitForSelector(".suggestion, li.suggestion");
  await covered(page);
  await page.screenshot({ path: `${dir}/discover.png` });
  await page.context().close();

  // the phone: requests
  page = await openPage(lang, { width: 390, height: 844 }, 3);
  await page.goto(`${BASE}/requests`);
  await page.waitForSelector("#requestList li.request");
  await covered(page);
  await page.screenshot({ path: `${dir}/phone-requests.png` });
  await page.context().close();

  // the GIF: library → a request arrives → discover with details
  const frames = join(tmpdir(), `hw-gif-${lang}`);
  rmSync(frames, { recursive: true, force: true });
  mkdirSync(frames);
  let n = 0;
  const shot = async (p, hold = 1) => { const f = await p.screenshot(); for (let i = 0; i < hold; i++) writeFileSync(join(frames, `${String(n++).padStart(4, "0")}.png`), f); };
  state = { progress: 0.62, arrived: false };
  page = await openPage(lang, { width: 1280, height: 800 }, 0.8);
  await page.goto(`${BASE}/books`);
  await covered(page);
  await shot(page, 6);
  for (let y = 1; y <= 4; y++) { await page.mouse.wheel(0, 160); await settle(page, 350); await covered(page); await shot(page); }
  await shot(page, 3);
  await page.goto(`${BASE}/requests`);
  await page.waitForSelector("#requestList li.request");
  await covered(page);
  await shot(page, 4);
  for (const p of [0.7, 0.78, 0.86, 0.93, 0.99]) {
    state.progress = p;
    await page.evaluate(() => Promise.all([loadRequests(), pollBench()]));   // the workbench in the sidebar too
    await settle(page, 300);
    await shot(page, 2);
  }
  state.arrived = true;
  await page.evaluate(() => Promise.all([loadRequests(), pollBench()]));   // the workbench in the sidebar too
  await covered(page);
  await shot(page, 7);
  await page.goto(`${BASE}/discover`);
  await page.waitForSelector("li.suggestion");
  await covered(page);
  await shot(page, 5);
  await page.locator("li.suggestion").nth(1).click();
  await settle(page, 700);
  await covered(page);
  await shot(page, 8);
  await page.context().close();
  execFileSync("ffmpeg", ["-y", "-loglevel", "error", "-framerate", "3", "-i", join(frames, "%04d.png"),
    "-vf", "split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle",
    `${dir}/demo.gif`]);
}

await browser.close();
server.kill();
