import { devices, expect, test } from "@playwright/test";

// The phone flows that broke on iOS before. In the default (Chromium) run this is an emulation of the phone;
// the "webkit-iphone" project (PW_WEBKIT=1, run in CI) runs the same file in real WebKit, the engine of Safari.
const { defaultBrowserType: _browser, ...phone } = devices["iPhone 13"];
test.use(phone);

const ADMIN = { name: "admin", password: "E2E-Admin-Kaffee-1" };
const OWN = { "X-Requested-With": "Hoerwerk" };

declare function openReleases(r: unknown): Promise<void>;
declare const requests: { data: { items: unknown[] } };

test("iPhone: anmelden und mit der unteren Leiste zwischen den Seiten wechseln", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto("/login");
  const form = page.locator("#loginForm");
  await form.locator("input[name=name]").fill(ADMIN.name);
  await form.locator("input[name=password]").fill(ADMIN.password);
  await form.locator("button[type=submit]").click();
  await expect(page).not.toHaveURL(/\/login/);
  const tabs = page.locator("nav.tabs");
  await expect(tabs).toBeVisible();
  await page.goto("/books");
  await expect(page.locator("#viewBooks")).toBeVisible();
  await page.goto("/requests");
  await expect(page.locator("#viewRequests")).toBeVisible();
  expect(errors).toEqual([]);
});

test("iPhone: die NZB-Auswahl lässt jede Datei zu (iOS kennt .nzb nicht und graut sie sonst aus)", async ({ page }) => {
  await page.request.post("/api/auth/login", { headers: OWN, data: { ...ADMIN, remember: false } });
  const made = await page.request.post("/api/requests", {
    headers: OWN, data: { book: { asin: "B0MOBILE01", title: "Mobiltest", authors: ["Anna Autorin"], language: "german" } } });
  expect([200, 201, 409]).toContain(made.status());
  await page.goto("/requests");
  await expect.poll(() => page.evaluate(() => requests.data?.items.length ?? 0)).toBeGreaterThan(0);
  await page.evaluate(() => openReleases(requests.data.items[0]));
  const pick = page.locator("#releasesNzb");
  await expect(pick).toHaveCount(1);
  expect(await pick.getAttribute("accept")).toBeNull();           // no type filter: that is what greyed files out
  // a file of another kind is refused with a hint, nothing is sent
  const sent: string[] = [];
  page.on("request", (r) => { if (r.url().includes("/nzb")) sent.push(r.url()); });
  await pick.setInputFiles({ name: "notiz.txt", mimeType: "text/plain", buffer: Buffer.from("kein nzb") });
  await expect(page.locator("#toast .toast-item.error")).toContainText(".nzb");
  expect(sent).toEqual([]);
});
