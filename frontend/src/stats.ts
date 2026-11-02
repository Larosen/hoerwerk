"use strict";
// ------------------------------------------------------------------------- //
// Statistik (/stats, /stats/<id>): who asked for how many books, how many hours, how much space, what was heard.
// * Admins: every account and the totals; a name opens that account (/stats/<id>): its requests and what it heard.
// * Everyone else: their own numbers, their requests and what they heard – next to the family's totals,
//   without who asked or heard how much (the server only sends that much).
// ------------------------------------------------------------------------- //
interface ListenSummary { seconds: number; books: number; finished: number; last: number | null; linked: string[]; errors: string[] }
interface UserStats {
  id: number | null; name: string; role: string; requests: number; books: number; available: number; open: number;
  declined: number; failed: number; series: number; minutes: number; minutes_available: number; bytes: number;
  english: number; wait_median: number | null; last: number | null; top_genres: string[];
  listen?: ListenSummary | null;
}
interface MonthRow { month: string; books: number; minutes: number; mine?: number }
interface ListenTotal { seconds: number; books: number; finished: number; linked: number }
interface StatsAll {
  scope: "all"; period: string; since: number | null; users: UserStats[]; total: UserStats;
  months: MonthRow[]; library_books: number | null; listen: ListenTotal | null; listen_errors?: string[];
}
interface StatsOwn {
  scope: "own"; period: string; since: number | null; me: UserStats; total: UserStats; accounts: number;
  months: MonthRow[]; library_books: number | null; listen: ListenTotal | null;
}
interface StatsRequest {
  id: number; title: string; authors: string[]; series: string | null; series_position: string | null;
  cover_url: string | null; language: string | null; runtime_min: number; status: RequestStatus;
  created: number; updated: number; group: string | null; bytes: number;
}
interface StatsHeard {
  title: string; author: string; progress: number; finished: boolean; finished_at: number | null; last: number | null;
  source: "abs" | "plex";
}
interface StatsAccount {
  period: string; since: number | null; user: UserStats; months: MonthRow[]; requests: StatsRequest[]; heard: StatsHeard[];
}

const stats = { seq: 0 };

function fmtHours(min: number): string {
  if (!min) return "–";
  const h = min / 60;
  return h < 10 ? `${h.toLocaleString(LOCALE, { maximumFractionDigits: 1 })} h` : `${Math.round(h).toLocaleString(LOCALE)} h`;
}

function fmtWait(sec: number | null): string {
  if (sec === null) return "–";
  if (sec < 3600) return t("{n} min", { n: Math.max(1, Math.round(sec / 60)) });
  if (sec < 2 * 86400) return `${Math.round(sec / 3600)} h`;
  return t("{n} Tage", { n: Math.round(sec / 86400) });
}

function fmtDay(ts: number | null | undefined): string {
  return ts ? new Date(ts * 1000).toLocaleDateString(LOCALE) : "–";
}

function monthName(key: string, long = false): string {
  const [y, m] = key.split("-").map(Number);
  return new Date(y, m - 1, 1).toLocaleDateString(LOCALE, long ? { month: "long", year: "numeric" } : { month: "short" });
}

function statTile(value: string, label: string, note = ""): HTMLElement {
  return el("div", { class: "stat-tile" }, el("b", {}, value), el("span", {}, label), note ? el("small", { class: "muted" }, note) : null);
}

function periodLabel(): string {
  return document.querySelector<HTMLInputElement>("#statsPeriod input:checked")?.parentElement?.textContent?.trim() || "";
}

/** The tiles of one person or of everyone: requests, hours, space, waiting – and listening when linked. */
function userTiles(u: UserStats, libraryBooks: number | null, listen: ListenTotal | ListenSummary | null | undefined,
  listenNote: string): HTMLElement[] {
  return [
    statTile(String(u.books), t("Bücher angefragt"), plural(u.requests, t("in 1 Anfrage"), t("in {n} Anfragen", { n: u.requests }))),
    statTile(String(u.available), t("davon verfügbar"), u.open ? t("{n} noch offen", { n: u.open }) : ""),
    statTile(fmtHours(u.minutes), t("Spielzeit angefragt"), u.minutes_available ? t("{h} schon da", { h: fmtHours(u.minutes_available) }) : ""),
    statTile(u.bytes ? fmtSize(u.bytes) : "–", t("Speicher der Anfragen"), libraryBooks !== null ? t("Bibliothek: {n} Bücher", { n: libraryBooks }) : ""),
    statTile(fmtWait(u.wait_median), t("typische Wartezeit"), t("von der Anfrage bis verfügbar")),
    ...(listen ? [statTile(fmtListen(listen.seconds), t("gehört"),
      [plural(listen.books, t("1 Buch"), t("{n} Bücher", { n: listen.books })),
        listen.finished ? t("{n} fertig", { n: listen.finished }) : "", listenNote].filter(Boolean).join(" · "))] : []),
  ];
}

function setStatsHead(title: string, back: boolean): void {
  $("#statsTitle").textContent = title;
  $("#statsBack").hidden = !back;
  document.title = `${title} · Hörwerk`;
}

/** What is shown in which mode – every part starts hidden and the mode turns its parts on. */
function resetStatsParts(): void {
  for (const id of ["#statsTilesHead", "#statsTotalBox", "#statsAccounts", "#statsDetail"]) $(id).hidden = true;
}

// -- admins: every account -------------------------------------------------------
function renderAll(s: StatsAll): void {
  resetStatsParts();
  setStatsHead(t("Statistik"), false);
  const tot = s.total;
  const listening = !!s.listen?.linked;
  $("#statsTiles").replaceChildren(...userTiles(tot, s.library_books, listening ? s.listen : null,
    s.listen ? t("{n} verknüpfte Konten", { n: s.listen.linked }) : ""));
  $("#statsAccounts").hidden = false;
  // the listening columns only when someone is linked to Audiobookshelf or Plex
  for (const th of document.querySelectorAll<HTMLElement>(".stats-table th.listen-col")) th.hidden = !listening;
  $("#statsListenNote").hidden = !listening;
  const listenCells = (u: UserStats): HTMLElement[] => !listening ? [] : u.listen ? [
    el("td", { class: "num", title: u.listen.errors.join(" · ") }, fmtListen(u.listen.seconds)),
    el("td", { class: "num" }, u.listen.books ? `${u.listen.books}${u.listen.finished ? ` (${u.listen.finished})` : ""}` : "–"),
    el("td", { class: "when" }, fmtDay(u.listen.last)),
  ] : [el("td", { class: "num muted", title: t("nicht verknüpft") }, "·"), el("td", { class: "num muted" }, "·"), el("td", { class: "muted" }, "·")];
  const rows = s.users.map((u) => el("tr", { class: u.books || u.listen?.seconds ? "" : "quiet" },
    el("td", { class: "name" }, u.id !== null
      ? el("a", { href: `/stats/${u.id}`, title: t("Anfragen und Gehörtes von {name}", { name: u.name }) }, el("b", {}, u.name))
      : el("b", {}, u.name), u.role === "admin" ? el("span", { class: "muted small" }, " · Admin") : null),
    el("td", { class: "num" }, u.requests ? String(u.requests) : "–", u.series ? el("small", { class: "muted", title: t("davon Reihen") }, ` (${u.series} ${t("Reihen")})`) : null),
    el("td", { class: "num" }, String(u.books || "–")),
    el("td", { class: "num" }, String(u.available || "–")),
    el("td", { class: "num" }, String(u.open || "–")),
    el("td", { class: "num" }, u.declined + u.failed ? `${u.declined} / ${u.failed}` : "–"),
    el("td", { class: "num" }, fmtHours(u.minutes)),
    el("td", { class: "num" }, u.bytes ? fmtSize(u.bytes) : "–"),
    el("td", { class: "num" }, fmtWait(u.wait_median)),
    ...listenCells(u),
    el("td", { class: "num" }, u.books ? `${Math.round((u.english / u.books) * 100)} %` : "–"),
    el("td", { class: "genre", title: u.top_genres.join(", ") }, u.top_genres[0] || "–"),
    el("td", { class: "when" }, fmtDay(u.last))));
  $("#statsRows").replaceChildren(...(rows.length ? rows : [el("tr", {}, el("td", { colspan: "15", class: "empty-row" }, t("Noch keine Konten.")))]));
  renderMonthChart(s.months, t("Angefragte Bücher je Monat"), t("letzte 12 Monate, alle Konten"));
}

// -- a family member: themselves and everyone together -------------------------------------
function renderOwn(s: StatsOwn): void {
  resetStatsParts();
  setStatsHead(t("Statistik"), false);
  $("#statsTilesHead").hidden = false;
  $("#statsTilesHead").textContent = t("Du");
  $("#statsTiles").replaceChildren(...userTiles(s.me, null, s.me.listen, ""));
  $("#statsTotalBox").hidden = false;
  $("#statsTotalNote").textContent = plural(s.accounts, t("1 Konto"), t("{n} Konten", { n: s.accounts }));
  $("#statsTotalTiles").replaceChildren(...userTiles(s.total, s.library_books, s.listen?.linked ? s.listen : null, ""));
  $("#statsListenNote").hidden = !s.me.listen;
  renderMonthChart(s.months, t("Angefragte Bücher je Monat"), t("letzte 12 Monate – dunkel: deine"), true);
}

// -- one account in detail -----------------------------------------------------------------
const STATUS_TONE: Record<string, string> = { available: "ok", declined: "no", failed: "no" };

function requestItem(r: StatsRequest): HTMLElement {
  const label = REQ_STATUS[r.status] ? REQ_STATUS[r.status]() : r.status;
  const vol = r.series ? `${r.series}${r.series_position ? " " + r.series_position : ""}` : "";
  return el("li", {},
    el("div", { class: "stats-item-main" },
      el("b", { title: r.title }, r.title),
      el("span", { class: "muted small" }, [(r.authors || []).join(", "), vol].filter(Boolean).join(" · "))),
    el("div", { class: "stats-item-side" },
      el("span", { class: `tag ${STATUS_TONE[r.status] || "soft"}` }, label),
      el("span", { class: "muted small" }, [fmtDay(r.created), r.runtime_min ? fmtHours(r.runtime_min) : "",
        r.bytes ? fmtSize(r.bytes) : ""].filter(Boolean).join(" · "))));
}

function heardItem(h: StatsHeard): HTMLElement {
  const pct = Math.round(Math.min(1, h.progress) * 100);
  return el("li", {},
    el("div", { class: "stats-item-main" },
      el("b", { title: h.title }, h.title || "–"),
      el("span", { class: "muted small" }, [h.author, h.source === "plex" ? "Plex" : "Audiobookshelf"].filter(Boolean).join(" · "))),
    el("div", { class: "stats-item-side" },
      h.finished ? el("span", { class: "tag ok" }, t("fertig")) : el("span", { class: "progress-mini", title: `${pct} %` },
        el("span", { style: `width:${pct}%` })),
      el("span", { class: "muted small" }, fmtDay(h.finished ? h.finished_at || h.last : h.last))));
}

function renderAccount(d: StatsAccount, own: boolean): void {
  const u = d.user;
  if (!own) {
    resetStatsParts();
    setStatsHead(t("Statistik: {name}", { name: u.name }), true);
    $("#statsTiles").replaceChildren(...userTiles(u, null, u.listen, ""));
    $("#statsListenNote").hidden = !u.listen;
    renderMonthChart(d.months, t("Angefragte Bücher je Monat"), t("letzte 12 Monate, nur {name}", { name: u.name }));
  }
  $("#statsDetail").hidden = false;
  const span = d.period === "all" ? t("insgesamt") : t("im Zeitraum: {period}", { period: periodLabel() });
  $("#statsReqNote").textContent = `${plural(d.requests.length, t("1 Buch"), t("{n} Bücher", { n: d.requests.length }))} ${span}`;
  $("#statsRequests").replaceChildren(...(d.requests.length ? d.requests.map(requestItem)
    : [el("li", { class: "empty" }, t("Nichts angefragt in diesem Zeitraum."))]));
  const linked = !!u.listen;
  $("#statsHeardNote").textContent = linked ? `${plural(d.heard.length, t("1 Buch"), t("{n} Bücher", { n: d.heard.length }))} ${span}` : "";
  $("#statsHeard").replaceChildren(...(d.heard.length ? d.heard.map(heardItem)
    : [el("li", { class: "empty" }, linked ? t("Nichts gehört in diesem Zeitraum.")
      : own ? t("Noch nicht mit Audiobookshelf oder Plex verbunden – das geht unter „Konto“.")
        : t("Nicht mit Audiobookshelf oder Plex verknüpft (Einstellungen → Konten)."))]));
}

/** Books per month: thin bars from the baseline, hover or tap shows the numbers. With ``mine``, the own part
 *  of each bar is darker (one series split in two, not two series side by side). */
function renderMonthChart(months: MonthRow[], head: string, note: string, mine = false): void {
  $("#statsChartHead").textContent = head;
  $("#statsChartNote").textContent = note;
  const max = Math.max(1, ...months.map((m) => m.books));
  const peak = months.reduce((a, b) => (b.books > a.books ? b : a), months[0]);
  const chart = $("#statsChart");
  const tip = el("div", { class: "chart-tip", hidden: true });
  const show = (m: MonthRow, bar: HTMLElement): void => {
    tip.replaceChildren(el("b", {}, monthName(m.month, true)),
      el("span", {}, plural(m.books, t("1 Buch"), t("{n} Bücher", { n: m.books }))),
      ...(mine ? [el("span", {}, t("davon deine: {n}", { n: m.mine || 0 }))] : []),
      el("span", { class: "muted" }, fmtHours(m.minutes)));
    tip.hidden = false;
    const r = bar.getBoundingClientRect(), box = chart.getBoundingClientRect();
    const x = Math.min(Math.max(r.left - box.left + r.width / 2, 60), box.width - 60);
    tip.style.left = `${x}px`;
  };
  chart.setAttribute("aria-label", t("Angefragte Bücher je Monat, höchstens {n}", { n: peak?.books || 0 }));
  chart.replaceChildren(...months.map((m) => {
    // the peak's number sits above its bar (inside it, positioned) – it must not squeeze the bar
    const bar = el("div", { class: "mbar" + (mine ? " shared" : ""), style: `height:${m.books ? Math.max(2, (m.books / max) * 100) : 0}%` },
      m === peak && m.books ? el("span", { class: "peak" }, String(m.books)) : null,
      mine && m.mine ? el("span", { class: "own", style: `height:${(m.mine / m.books) * 100}%` }) : null);
    const col = el("div", { class: "col", tabindex: "0", "aria-label": `${monthName(m.month, true)}: ${m.books}${mine ? ` (${m.mine || 0})` : ""}` },
      el("div", { class: "plot" }, bar),
      el("span", { class: "label" }, monthName(m.month)));
    for (const ev of ["mouseenter", "focus"]) col.addEventListener(ev, () => show(m, bar));
    for (const ev of ["mouseleave", "blur"]) col.addEventListener(ev, () => { tip.hidden = true; });
    return col;
  }), tip);
  $("#statsMonthRows").replaceChildren(...months.slice().reverse().map((m) =>
    el("tr", {}, el("td", {}, monthName(m.month, true)),
      el("td", { class: "num" }, mine ? `${m.books} (${m.mine || 0})` : String(m.books)), el("td", { class: "num" }, fmtHours(m.minutes)))));
}

async function loadStats(): Promise<void> {
  const period = document.querySelector<HTMLInputElement>("#statsPeriod input:checked")?.value || "365";
  const section = router.current?.view === "stats" ? router.current.section : "";
  const seq = ++stats.seq;                      // a slow answer for the last page must not overwrite this one
  const q = `period=${encodeURIComponent(period)}`;
  try {
    if (section) {
      const d = await api<StatsAccount>("GET", `/api/stats/accounts/${encodeURIComponent(section)}?${q}`);
      if (seq === stats.seq) renderAccount(d, false);
      return;
    }
    const s = await api<StatsAll | StatsOwn>("GET", `/api/stats?${q}`);
    if (seq !== stats.seq) return;
    if (s.scope === "all") { renderAll(s); return; }
    renderOwn(s);
    if (typeof ME !== "undefined" && ME) {
      const d = await api<StatsAccount>("GET", `/api/stats/accounts/${ME.id}?${q}`);
      if (seq === stats.seq) renderAccount(d, true);
    }
  } catch (e) { toast(errMsg(e), true); }
}

function initStats(): void {
  const saved = store.get("statsPeriod", "365");
  for (const r of document.querySelectorAll<HTMLInputElement>("#statsPeriod input")) {
    r.checked = r.value === saved;
    r.addEventListener("change", () => { store.set("statsPeriod", r.value); loadStats(); });
  }
  document.addEventListener("routechange", (e) => {
    if ((e as CustomEvent<{ route: Route }>).detail.route.view === "stats") loadStats();
  });
}

initStats();
