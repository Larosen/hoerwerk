"use strict";
// Protokoll (/settings/logs): the app log, live while the page is open.

interface LogEntry { id: number; time: number; level: "info" | "warn" | "error"; source: string; text: string }

const logs = {
  entries: [] as LogEntry[],
  last: 0,
  level: "info",
  window: 60,                                    // minutes shown (0 = everything the server still has)
  timer: undefined as number | undefined,
};

const LOG_LEVEL_NAMES: Record<string, string> = { info: "", warn: t("Warnung"), error: t("Fehler") };

function fmtLogTime(ts: number): string {
  const d = new Date(ts * 1000);
  const today = new Date().toDateString() === d.toDateString();
  return d.toLocaleString(LOCALE, today ? { timeStyle: "medium" } : { dateStyle: "short", timeStyle: "medium" });
}

async function loadLogs(reset = false): Promise<void> {
  if (reset) { logs.entries = []; logs.last = 0; }
  try {
    const res = await api<{ entries: LogEntry[]; last: number }>("GET",
      `/api/logs?after=${logs.last}&level=${encodeURIComponent(logs.level)}`);
    logs.entries = logs.entries.concat(res.entries).slice(-3000);
    logs.last = res.last;
    if (reset || res.entries.length) renderLogs();
  } catch (e) { $("#logView").textContent = errMsg(e); }
}

function renderLogs(): void {
  const view = $("#logView");
  const atBottom = view.scrollHeight - view.scrollTop - view.clientHeight < 40;
  const words = $<HTMLInputElement>("#logFilter").value.toLowerCase().split(/\s+/).filter(Boolean);
  const cutoff = logs.window ? Date.now() / 1000 - logs.window * 60 : 0;
  const inWindow = logs.entries.filter((e) => e.time >= cutoff);
  const older = logs.entries.length - inWindow.length;
  const shown = inWindow.filter((e) => {
    const text = `${e.source} ${e.text}`.toLowerCase();
    return words.every((w) => text.includes(w));
  });
  view.replaceChildren(...shown.map((e) => el("div", { class: "log-line " + e.level },
    el("span", { class: "log-time" }, fmtLogTime(e.time)),
    e.level !== "info" ? el("span", { class: "log-level" }, LOG_LEVEL_NAMES[e.level]) : null,
    e.source ? el("span", { class: "log-source" }, e.source) : null,
    el("span", { class: "log-text" }, e.text))));
  if (!shown.length) view.append(el("div", { class: "muted log-empty" }, words.length ? t("Nichts gefunden.") : older ? t("Nichts in diesem Zeitraum.") : t("Noch keine Einträge.")));
  if (older) {
    view.append(el("div", { class: "muted log-empty log-older" }, plural(older, t("1 älterer Eintrag ausgeblendet"), t("{n} ältere Einträge ausgeblendet", { n: older })), " ",
      el("button", { type: "button", class: "linkish", onclick: () => setLogWindow(logs.window === 15 ? 60 : logs.window === 60 ? 1440 : 0) }, t("Zeitraum erweitern"))));
  }
  if (atBottom || !view.dataset.seen) { view.scrollTop = view.scrollHeight; view.dataset.seen = "1"; }
}

function setLogWindow(minutes: number): void {
  logs.window = minutes;
  store.set("logWindow", minutes);
  for (const b of document.querySelectorAll<HTMLElement>("#logWindow button")) b.setAttribute("aria-pressed", String(Number(b.dataset.min) === minutes));
  renderLogs();
}

function followLogs(): void {
  clearTimeout(logs.timer);
  if (!logsVisible() || !$<HTMLInputElement>("#logFollow").checked) return;
  logs.timer = setTimeout(async () => { await loadLogs(); if (logs.window) renderLogs(); followLogs(); }, 2000);
}

function downloadLogs(): void {
  const text = logs.entries.map((e) => `${new Date(e.time * 1000).toISOString()} ${e.level.toUpperCase().padEnd(5)}`
    + `${e.source ? ` [${e.source}]` : ""} ${e.text}`).join("\n") + "\n";
  const a = el("a", { href: URL.createObjectURL(new Blob([text], { type: "text/plain" })),
    download: `hoerwerk-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-")}.log` });
  document.body.append(a);
  a.click();
  setTimeout(() => { URL.revokeObjectURL(a.href); a.remove(); }, 1000);
}

function logsVisible(): boolean {
  return router.current?.view === "settings" && router.current.section === "logs";
}

/** The log page was opened: newest entries, then follow live. */
async function showLogs(): Promise<void> {
  delete $("#logView").dataset.seen;
  await loadLogs(true);
  followLogs();
}

function initLogs(): void {
  const saved = store.get("logWindow", 60);
  logs.window = [15, 60, 1440, 0].includes(saved) ? saved : 60;       // the last hour: what just happened
  for (const b of document.querySelectorAll<HTMLElement>("#logWindow button")) {
    b.setAttribute("aria-pressed", String(Number(b.dataset.min) === logs.window));
    b.addEventListener("click", () => setLogWindow(Number(b.dataset.min)));
  }
  $<HTMLSelectElement>("#logLevel").addEventListener("change", (e) => {
    logs.level = (e.target as HTMLSelectElement).value;
    loadLogs(true);
  });
  $("#logFilter").addEventListener("input", renderLogs);
  $("#logFollow").addEventListener("change", followLogs);
  $("#logDownload").addEventListener("click", downloadLogs);
}

initLogs();
