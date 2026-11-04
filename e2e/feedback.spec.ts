import { expect, test, type Page, type Route } from "@playwright/test";

// Feedback everywhere: a button shows its running action and blocks a second click, messages stack and errors
// stay, lists show placeholders and "Erneut versuchen", the live connection comes back by itself.
// Slow, failing and broken answers are simulated in the browser (page.route) – the server stays untouched.

declare const live: { ok: boolean };
declare function toast(msg: string, kind?: boolean | string): void;

const ADMIN = { name: "admin", password: "E2E-Admin-Kaffee-1" };
const OWN = { "X-Requested-With": "Hoerwerk" };

async function open(page: Page, path = "/"): Promise<string[]> {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    // failed requests we simulate on purpose show up as console errors of the browser
    if (m.type() === "error" && !/status of (500|503)|net::ERR_FAILED|Failed to load resource/.test(m.text())) errors.push(m.text());
  });
  const res = await page.request.post("/api/auth/login", { headers: OWN, data: { ...ADMIN, remember: false } });
  expect(res.status()).toBe(200);
  await page.goto(path);
  await expect.poll(() => page.evaluate(() => live.ok), { message: "Live-Verbindung" }).toBe(true);
  return errors;
}

/** Hold an answer until ``release()`` – a slow server. */
function hold(): { route: (r: Route) => Promise<void>; release: () => void; count: () => number } {
  let open: () => void = () => undefined;
  const gate = new Promise<void>((r) => { open = r; });
  let n = 0;
  return {
    route: async (r: Route) => { n += 1; await gate; await r.continue(); },
    release: () => open(),
    count: () => n,
  };
}

test.describe.configure({ mode: "serial" });

test("Knopf: zeigt die laufende Aktion, ein zweiter Klick schickt nichts doppelt, danach Erfolg", async ({ page }) => {
  const errors = await open(page, "/discover");
  const slow = hold();
  await page.route("**/api/discover/refresh", slow.route);
  const btn = page.locator("#discoverRefresh");
  const size = await btn.boundingBox();
  await btn.click();
  await expect(btn).toHaveAttribute("aria-busy", "true");
  await expect(btn).toContainText("Aktualisiert …");
  await expect(btn.locator(".spinner")).toBeVisible();
  const busySize = await btn.boundingBox();
  expect(Math.round(busySize?.width || 0)).toBeGreaterThanOrEqual(Math.round(size?.width || 0));   // keeps its size
  expect(Math.round(busySize?.height || 0)).toBe(Math.round(size?.height || 0));
  await btn.click({ force: true });                                        // impatient: clicked again …
  await btn.dblclick({ force: true });                                     // … and again
  slow.release();
  await expect(btn).not.toHaveAttribute("aria-busy", "true");
  expect(slow.count()).toBe(1);                                            // … still one request
  await expect(btn).toContainText("Aktualisieren");                       // the label is back
  await expect(btn).toHaveClass(/did-work/);
  expect(errors).toEqual([]);
});

test("Fehler: verständliche Meldung, bleibt stehen, lässt sich schließen; der Knopf geht wieder", async ({ page }) => {
  const errors = await open(page, "/discover");
  await page.route("**/api/discover/refresh", (r) => r.fulfill({ status: 500, contentType: "application/json",
    body: JSON.stringify({ error: "Interner Fehler (Nr. 1a2b3c4d) – Einzelheiten stehen im Protokoll", request_id: "1a2b3c4d" }) }));
  const btn = page.locator("#discoverRefresh");
  await btn.click();
  const alert = page.locator("#toast .toast-item.error");
  await expect(alert).toHaveAttribute("role", "alert");
  await expect(alert).toContainText("Nr. 1a2b3c4d");
  await expect(btn).not.toHaveAttribute("aria-busy", "true");
  await expect(btn).toHaveClass(/did-fail/);
  await page.waitForTimeout(4000);
  await expect(alert).toBeVisible();                                        // errors stay longer than successes
  await alert.getByRole("button", { name: "Meldung schließen" }).click();
  await expect(alert).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("Netzwerk weg: eine Meldung ohne Technik, kein automatisches Wiederholen", async ({ page }) => {
  await open(page, "/discover");
  let calls = 0;
  await page.route("**/api/discover/refresh", (r) => { calls += 1; return r.abort("failed"); });
  await page.locator("#discoverRefresh").click();
  await expect(page.locator("#toast .toast-item.error")).toContainText("Keine Verbindung zu Hörwerk");
  await page.waitForTimeout(1000);
  expect(calls).toBe(1);                                                    // a change is never sent twice by itself
});

test("Liste: Platzhalter beim ersten Laden, leer ist nicht ‚lädt noch‘, Fehler mit ‚Erneut versuchen‘", async ({ page }) => {
  const slow = hold();
  let fail = true;
  await page.route("**/api/requests", async (r) => {
    if (r.request().method() !== "GET") return r.continue();
    if (fail) return r.fulfill({ status: 503, body: "Zu viele Verbindungen" });
    return slow.route(r);
  });
  await open(page, "/requests");                                            // the list has never loaded here
  const list = page.locator("#requestList");
  const error = list.locator(".load-error");
  await expect(error).toContainText("ausgelastet");                        // a readable reason, not "HTTP 503"
  fail = false;
  await error.getByRole("button", { name: "Erneut versuchen" }).click();
  await expect(list.locator(".skeleton").first()).toBeVisible();          // loading looks like loading …
  await expect(list).toHaveAttribute("aria-busy", "true");
  slow.release();
  await expect(list.locator(".skeleton")).toHaveCount(0);
  await expect(list).not.toHaveAttribute("aria-busy", "true");
  await expect(list.locator(".load-error")).toHaveCount(0);               // … and the result replaces it
});

test("Meldungen: stapeln geordnet, höchstens vier, Erfolg verschwindet von selbst", async ({ page }) => {
  await open(page, "/discover");
  await page.evaluate(() => {
    for (let i = 1; i <= 6; i++) toast(`Meldung ${i}`, i === 2 ? "error" : "success");
    toast("Meldung 6");                                                     // the same again: no twin
  });
  const items = page.locator("#toast .toast-item");
  await expect(items).toHaveCount(4);
  await expect(items.last()).toContainText("Meldung 6");
  await expect(page.locator("#toast")).toHaveAttribute("aria-live", "polite");
  await expect(items.filter({ hasText: "Meldung 6" })).toHaveAttribute("role", "status");
  await expect(items.filter({ hasText: "Meldung 6" })).toHaveCount(0, { timeout: 6000 });
});

test("Live-Verbindung: nach einer Abweisung (503) verbindet sich die Seite selbst neu", async ({ page }) => {
  let refused = 0;
  await page.route("**/api/events**", (r) => {
    if (refused < 2) { refused += 1; return r.fulfill({ status: 503, body: "busy" }); }
    return r.continue();
  });
  const res = await page.request.post("/api/auth/login", { headers: OWN, data: { ...ADMIN, remember: false } });
  expect(res.status()).toBe(200);
  await page.goto("/discover");
  await expect.poll(() => page.evaluate(() => live.ok), { timeout: 20_000 }).toBe(true);
  expect(refused).toBe(2);
});

test("Handy und Tastatur: Knopf per Enter, Meldung per Tastatur schließen", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await open(page, "/discover");
  const slow = hold();
  await page.route("**/api/discover/refresh", slow.route);
  const btn = page.locator("#discoverRefresh");
  await btn.focus();
  await page.keyboard.press("Enter");
  await expect(btn).toHaveAttribute("aria-busy", "true");
  await page.keyboard.press("Enter");                                       // held down: still one request
  slow.release();
  await expect(btn).not.toHaveAttribute("aria-busy", "true");
  expect(slow.count()).toBe(1);
  await page.evaluate(() => toast("Fehler zum Schließen", "error"));
  const close = page.locator("#toast .toast-item.error .toast-x");
  await close.focus();
  await page.keyboard.press("Enter");
  await expect(page.locator("#toast .toast-item.error")).toHaveCount(0);
  const box = await page.locator("#toast").boundingBox();
  expect((box?.x || 0) >= 0 && (box?.x || 0) + (box?.width || 0) <= 390).toBe(true);   // fits the phone
});
