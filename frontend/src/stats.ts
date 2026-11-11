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

/** Four tiles in plain words: asked for, became available, heard, waiting time. */
function userTiles(u: UserStats, listen: ListenTotal | ListenSummary | null | undefined, listenNote: string): HTMLElement[] {
  return [
    statTile(String(u.books || 0), plural(u.books, t("Buch angefragt"), t("Bücher angefragt")),
      u.minutes ? t("{h} Spielzeit", { h: fmtHours(u.minutes) }) : ""),
    statTile(String(u.available || 0), t("davon schon da"), u.open ? t("{n} noch offen", { n: u.open }) : ""),
    listen ? statTile(fmtListen(listen.seconds), t("gehört"), [plural(listen.books, t("1 Buch"), t("{n} Bücher", { n: listen.books })),
      listenNote].filter(Boolean).join(" · "))
      : statTile("–", t("gehört"), t("nicht verknüpft")),
    statTile(fmtWait(u.wait_median), t("Wartezeit"), t("bis ein Buch da ist")),
  ];
}

/** The smaller numbers – there when wanted, out of the way otherwise. */
function moreFacts(u: UserStats, libraryBooks: number | null): void {
  const facts: [string, string][] = [
    [t("Anfragen"), u.requests ? String(u.requests) + (u.series ? ` (${t("davon Reihen")}: ${u.series})` : "") : "–"],
    [t("Abgelehnt"), String(u.declined || 0)],
    [t("Fehlgeschlagen"), String(u.failed || 0)],
    [t("Speicher der angefragten Bücher"), u.bytes ? fmtSize(u.bytes) : "–"],
    [t("Englisch"), u.books ? `${Math.round((u.english / u.books) * 100)} %` : "–"],
    [t("Häufigste Genres"), u.top_genres.join(", ") || "–"],
    [t("Zuletzt angefragt"), fmtDay(u.last)],
    ...(u.listen ? [[t("Zuletzt gehört"), fmtDay(u.listen.last)] as [string, string]] : []),
    ...(libraryBooks !== null ? [[t("Bücher in der Bibliothek"), String(libraryBooks)] as [string, string]] : []),
  ];
  $("#statsFacts").replaceChildren(...facts.flatMap(([k, v]) => [el("dt", {}, k), el("dd", {}, v)]));
  $("#statsMore").hidden = false;
}

function setStatsHead(title: string, back: boolean): void {
  $("#statsTitle").textContent = title;
  $("#statsBack").hidden = !back;
  document.title = `${title} · Hörwerk`;
}

/** What is shown in which mode – every part starts hidden and the mode turns its parts on. */
function resetStatsParts(): void {
  for (const id of ["#statsTilesHead", "#statsTotalBox", "#statsAccounts", "#statsDetail", "#statsMore"]) $(id).hidden = true;
}

// -- admins: every account -------------------------------------------------------
/** One card per person: asked-for hours against heard hours, both on the same scale – who asks, who listens. */
function personCard(u: UserStats, scale: number): HTMLElement {
  const reqH = u.minutes / 60, heardH = (u.listen?.seconds || 0) / 3600;
  const bar = (cls: string, hours: number): HTMLElement =>
    el("span", { class: "pbar" }, el("span", { class: `pfill ${cls}`, style: `width:${scale ? Math.max(hours ? 2 : 0, (hours / scale) * 100) : 0}%` }));
  const active = Math.max(u.last || 0, u.listen?.last || 0) || null;
  const body = [
    el("div", { class: "person-head" },
      el("b", {}, u.name), u.role === "admin" ? el("span", { class: "muted small" }, "Admin") : null,
      el("span", { class: "muted small person-when" }, active ? t("aktiv {when}", { when: fmtDay(active) }) : t("noch nicht aktiv")),
      u.id !== null ? icon("chevron") : null),
    el("div", { class: "person-row" }, el("span", { class: "plabel" }, t("angefragt")), bar("req", reqH),
      el("span", { class: "pval" }, u.books ? `${plural(u.books, t("1 Buch"), t("{n} Bücher", { n: u.books }))} · ${fmtHours(u.minutes)}` : "–")),
    el("div", { class: "person-row" }, el("span", { class: "plabel" }, t("gehört")), bar("heard", heardH),
      el("span", { class: "pval" + (u.listen ? "" : " muted") }, u.listen
        ? (u.listen.seconds ? `${fmtListen(u.listen.seconds)} · ${plural(u.listen.books, t("1 Buch"), t("{n} Bücher", { n: u.listen.books }))}` : "–")
        : t("nicht verknüpft"))),
  ];
  return el("li", { class: "person" + (u.books || u.listen?.seconds ? "" : " quiet") },
    u.id !== null ? el("a", { href: `/stats/${u.id}`, class: "person-link", title: t("Anfragen und Gehörtes von {name}", { name: u.name }) }, ...body)
      : el("div", { class: "person-link" }, ...body));
}

function renderAll(s: StatsAll): void {
  resetStatsParts();
  setStatsHead(t("Statistik"), false);
  const linked = s.listen?.linked || 0;
  $("#statsTiles").replaceChildren(...userTiles(s.total, linked ? s.listen : null,
    linked ? plural(linked, t("1 Konto verknüpft"), t("{n} Konten verknüpft", { n: linked })) : ""));
  $("#statsAccounts").hidden = false;
  const scale = Math.max(1, ...s.users.map((u) => Math.max(u.minutes / 60, (u.listen?.seconds || 0) / 3600)));
  const people = [...s.users].sort((a, b) =>
    (b.minutes / 60 + (b.listen?.seconds || 0) / 3600) - (a.minutes / 60 + (a.listen?.seconds || 0) / 3600));
  $("#statsPeople").replaceChildren(...(people.length ? people.map((u) => personCard(u, scale))
    : [el("li", { class: "empty" }, t("Noch keine Konten."))]));
  moreFacts(s.total, s.library_books);
  renderMonthChart(s.months, t("Angefragte Bücher je Monat"), t("letzte 12 Monate, alle Konten"));
}

// -- a family member: themselves and everyone together -------------------------------------
function renderOwn(s: StatsOwn): void {
  resetStatsParts();
  setStatsHead(t("Statistik"), false);
  $("#statsTilesHead").hidden = false;
  $("#statsTilesHead").textContent = t("Du");
  $("#statsTiles").replaceChildren(...userTiles(s.me, s.me.listen, ""));
  $("#statsTotalBox").hidden = false;
  $("#statsTotalNote").textContent = plural(s.accounts, t("1 Konto"), t("{n} Konten", { n: s.accounts }));
  $("#statsTotalTiles").replaceChildren(...userTiles(s.total, s.listen?.linked ? s.listen : null, ""));
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
      h.finished ? el("span", { class: "tag ok" }, t("Fertig")) : el("span", { class: "progress-mini", title: `${pct} %` },
        el("span", { style: `width:${pct}%` })),
      el("span", { class: "muted small" }, fmtDay(h.finished ? h.finished_at || h.last : h.last))));
}

function renderAccount(d: StatsAccount, own: boolean): void {
  const u = d.user;
  if (!own) {
    resetStatsParts();
    setStatsHead(t("Statistik: {name}", { name: u.name }), true);
    $("#statsTiles").replaceChildren(...userTiles(u, u.listen, ""));
    moreFacts(u, null);
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
  const tiles = $("#statsTiles");
  if (!tiles.children.length) skeleton(tiles, 4, "line");
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
  } catch (e) {
    if (tiles.querySelector(":scope > .skeleton")) {
      const retry = el("button", { type: "button", class: "btn small" }, icon("refresh"), t("Erneut versuchen"));
      retry.addEventListener("click", () => { tiles.replaceChildren(); void loadStats(); });
      tiles.replaceChildren(el("div", { class: "load-error", role: "alert" },
        el("span", {}, t("Laden fehlgeschlagen: {error}", { error: errMsg(e) })), retry));
    } else toast(errMsg(e), true);
  }
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
