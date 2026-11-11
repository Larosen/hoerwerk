"use strict";
// ------------------------------------------------------------------------- //
// Einstellungen (/settings): an overview of all areas, one area per page (/settings/<area>),
// and the system check (/settings/health) that shows whether everything is set up right.
// ------------------------------------------------------------------------- //
interface HealthCheck {
  id: string; group: string; status: "ok" | "warn" | "error" | "info"; title: string; detail?: string; hint?: string;
}
interface HealthAnswer {
  checks: HealthCheck[]; server_time: number; app_url: string; version: string;
  services: { indexers: { id: string; name: string }[]; sab: boolean; abs: boolean; plex: boolean };
}

const HEALTH_GROUPS: [string, string][] = [
  ["connection", t("Verbindung & Proxy")], ["security", t("Sicherheit")], ["storage", t("Speicher")],
  ["services", t("Dienste")], ["system", t("System")],
];
const HEALTH_ICON: Record<HealthCheck["status"], string> = { ok: "check", warn: "alert", error: "x", info: "chevron" };

function sectionLinks(): HTMLAnchorElement[] {
  return [...document.querySelectorAll<HTMLAnchorElement>("#settingsNav a[data-section]")];
}

/** The overview: one tile per area (made from the side navigation, so both always match). */
function renderSettingsHub(): void {
  // every link of the navigation – also those leading out of the settings (Statistik is its own page)
  const links = [...document.querySelectorAll<HTMLAnchorElement>("#settingsNav a[href]")];
  $("#settingsHub").replaceChildren(...links.map((a) => {
    const small = a.querySelector("small")?.textContent || "";
    const name = (a.firstChild?.textContent || "").trim();
    return el("a", { class: "hub-tile", href: a.getAttribute("href") || "", "data-section": a.dataset.section || "" },
      el("b", {}, name), el("span", { class: a.dataset.section === "health" ? "muted small hub-status" : "muted small" }, small));
  }));
}

/** Controls stay locked until the saved values are in – otherwise a late answer would overwrite what was just typed. */
async function whileLoading(form: HTMLFormElement, load: () => Promise<unknown>): Promise<void> {
  const controls = [...form.elements].filter((c): c is HTMLInputElement => "disabled" in c && !(c as HTMLInputElement).disabled);
  form.setAttribute("aria-busy", "true");
  for (const c of controls) c.disabled = true;
  try { await load(); } catch { /* shown on the page itself */ } finally {
    for (const c of controls) c.disabled = false;
    form.removeAttribute("aria-busy");
  }
}

function showSettingsPage(section: string, entering: boolean): void {
  const view = $("#viewSettings");
  view.classList.toggle("hub", !section);
  $("#settingsHub").hidden = !!section;
  const link = sectionLinks().find((a) => a.dataset.section === section);
  $("#settingsTitle").textContent = link ? (link.firstChild?.textContent || "").trim() : t("Einstellungen");
  for (const a of sectionLinks()) {
    if (a.dataset.section === section) a.setAttribute("aria-current", "true"); else a.removeAttribute("aria-current");
  }
  // the two forms: show only the fieldsets of this area
  for (const form of [$<HTMLFormElement>("#settingsForm"), $<HTMLFormElement>("#sourcesForm")]) {
    let any = false;
    for (const fs of form.querySelectorAll<HTMLElement>("fieldset[data-section]")) {
      fs.hidden = fs.dataset.section !== section;
      any ||= !fs.hidden;
    }
    form.hidden = !any;
  }
  for (const page of view.querySelectorAll<HTMLElement>("[data-page]")) page.hidden = page.dataset.page !== section;
  document.title = section && link ? `${$("#settingsTitle").textContent} · Hörwerk` : `${t("Einstellungen")} · Hörwerk`;

  if (entering) {                          // fresh values when coming from another page – not between two areas
    whileLoading($<HTMLFormElement>("#settingsForm"), () => Promise.all([fillSettings(), loadNotify()]));
    whileLoading($<HTMLFormElement>("#sourcesForm"), () => loadSources().then(fillSources));
  }
  if (section === "library") showLibrarySettings();
  if (section === "users") { loadUsers(); loadListenMap(); }
  if (section === "logs") showLogs();
  if (section === "transfers") pollDownloads(true);
  if (section === "health") runHealth();
  if (!section) refreshHealthTile();
}

// -- health check ------------------------------------------------------------
function healthRow(c: HealthCheck): HTMLElement {
  return el("div", { class: `health-row ${c.status}` },
    el("span", { class: "health-icon" }, icon(HEALTH_ICON[c.status])),
    el("div", { class: "health-text" },
      el("b", {}, c.title),
      c.detail ? el("span", { class: "muted small" }, c.detail) : null,
      c.hint ? el("code", { class: "health-hint" }, c.hint) : null));
}

/** Checks only the browser can do: clock, live updates through the proxy, the address in notifications. */
function browserChecks(res: HealthAnswer): HealthCheck[] {
  const out: HealthCheck[] = [];
  const drift = Math.round(Date.now() / 1000 - res.server_time);
  out.push(Math.abs(drift) > 30
    ? { id: "clock", group: "system", status: "warn", title: t("Uhrzeit weicht {n} s vom Browser ab", { n: drift }),
      detail: t("Codes der Authenticator-App brauchen die richtige Uhrzeit. NTP prüfen.") }
    : { id: "clock", group: "system", status: "ok", title: t("Uhrzeit stimmt (Abweichung {n} s)", { n: drift }) });
  out.push(live.ok
    ? { id: "live", group: "connection", status: "ok", title: t("Live-Aktualisierung funktioniert") }
    : { id: "live", group: "connection", status: "warn", title: t("Live-Aktualisierung funktioniert nicht"),
      detail: t("Der Proxy puffert vermutlich /api/events. Die Seiten aktualisieren sich dadurch langsamer.") });
  if (res.app_url) {
    let same = false;
    try { same = new URL(res.app_url).origin === location.origin; } catch { /* invalid address */ }
    out.push(same
      ? { id: "app_url", group: "connection", status: "ok", title: t("Adresse in Benachrichtigungen passt: {url}", { url: res.app_url }) }
      : { id: "app_url", group: "connection", status: "info", title: t("Benachrichtigungen verlinken auf {url}", { url: res.app_url }),
        detail: t("Aufgerufen über {here}. Passt das nicht, unter Benachrichtigungen ändern.", { here: location.origin }) });
  }
  return out;
}

function renderHealth(checks: HealthCheck[]): void {
  const order = { error: 0, warn: 1, info: 2, ok: 3 };
  const list = $("#healthList");
  list.replaceChildren(...HEALTH_GROUPS.flatMap(([g, name]) => {
    const rows = checks.filter((c) => c.group === g).sort((a, b) => order[a.status] - order[b.status]);
    return rows.length ? [el("section", { class: "health-group" }, el("h3", {}, name), ...rows.map(healthRow))] : [];
  }));
  $("#healthSummary").textContent = healthSummary(checks).text;
}

/** One line for the result: used on the check page and on its tile in the overview. */
function healthSummary(checks: HealthCheck[]): { text: string; level: "ok" | "warn" | "error" } {
  const bad = checks.filter((c) => c.status === "error").length, warn = checks.filter((c) => c.status === "warn").length;
  const notes = warn === 1 ? t("1 Hinweis") : t("{w} Hinweise", { w: warn });
  if (bad) return { text: `${t("{n} Fehler", { n: bad })}, ${notes}`, level: "error" };
  return warn ? { text: t("Alles läuft – {notes}", { notes }), level: "warn" } : { text: t("Alles in Ordnung"), level: "ok" };
}

/** The tile of the health check shows the result of the quick checks (not the connection tests to the services). */
async function refreshHealthTile(): Promise<void> {
  const tile = document.querySelector<HTMLElement>('#settingsHub .hub-tile[data-section="health"]');
  const line = tile?.querySelector<HTMLElement>(".hub-status");
  if (!tile || !line) return;
  try {
    const res = await api<HealthAnswer>("GET", "/api/diagnostics");
    const { text, level } = healthSummary([...res.checks, ...browserChecks(res)]);
    line.textContent = text;
    tile.dataset.level = level;
  } catch { /* the tile keeps its question */ }
}

type TestResult = { ok: boolean; message: string };

async function runHealth(): Promise<void> {
  $("#healthSummary").textContent = t("Prüfe …");
  let res: HealthAnswer;
  try { res = await api<HealthAnswer>("GET", "/api/diagnostics"); } catch (e) { $("#healthSummary").textContent = errMsg(e); return; }
  const checks = [...res.checks, ...browserChecks(res)];
  renderHealth(checks);
  // the connected services answer one after the other – each result is added as it comes
  const svc = res.services;
  const tests: [string, Promise<TestResult>][] = [
    ...svc.indexers.map((i): [string, Promise<TestResult>] => [i.name, api<TestResult>("POST", "/api/sources/test", { kind: "indexer", id: i.id })]),
    ...(svc.sab ? [["SABnzbd", api<TestResult>("POST", "/api/sources/test", { kind: "sab" })] as [string, Promise<TestResult>]] : []),
    ...(svc.abs ? [["Audiobookshelf", api<TestResult>("POST", "/api/sources/test", { kind: "abs" })] as [string, Promise<TestResult>]] : []),
    ...(svc.plex ? [["Plex", api<TestResult>("POST", "/api/sources/test", { kind: "plex" })] as [string, Promise<TestResult>]] : []),
  ];
  for (const [name, p] of tests) {
    p.then((r) => ({ id: "svc-" + name, group: "services", status: r.ok ? "ok" : "error", title: `${name}: ${r.message}` }) as HealthCheck)
      .catch((e) => ({ id: "svc-" + name, group: "services", status: "error", title: `${name}: ${errMsg(e)}` }) as HealthCheck)
      .then((c) => { checks.push(c); renderHealth(checks); });
  }
}

function initSettings(): void {
  if (!IS_ADMIN) return;
  renderSettingsHub();
  $("#healthRun").addEventListener("click", runHealth);
  document.addEventListener("routechange", (e) => {
    const { route, previous } = (e as CustomEvent<{ route: Route; previous: Route | null }>).detail;
    if (route.view !== "settings") {
      if (previous?.view === "settings") clearTimeout(logs.timer);
      return;
    }
    showSettingsPage(route.section, previous?.view !== "settings");
  });
}

initSettings();
