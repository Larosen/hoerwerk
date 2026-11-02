"use strict";
// ------------------------------------------------------------------------- //
// Statistik (/settings/stats): who asked for how many books, how many hours, how much space.
// ------------------------------------------------------------------------- //
interface UserStats {
  id: number | null; name: string; role: string; requests: number; books: number; available: number; open: number;
  declined: number; failed: number; series: number; minutes: number; minutes_available: number; bytes: number;
  english: number; wait_median: number | null; last: number | null; top_genres: string[];
  listen?: { seconds: number; books: number; finished: number; last: number | null; linked: string[]; errors: string[] } | null;
}
interface StatsAnswer {
  period: string; since: number | null; users: UserStats[]; total: UserStats;
  months: { month: string; books: number; minutes: number }[]; library_books: number | null;
  listen: { seconds: number; books: number; finished: number; linked: number } | null; listen_errors?: string[];
}

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

function monthName(key: string, long = false): string {
  const [y, m] = key.split("-").map(Number);
  return new Date(y, m - 1, 1).toLocaleDateString(LOCALE, long ? { month: "long", year: "numeric" } : { month: "short" });
}

function statTile(value: string, label: string, note = ""): HTMLElement {
  return el("div", { class: "stat-tile" }, el("b", {}, value), el("span", {}, label), note ? el("small", { class: "muted" }, note) : null);
}

function renderStats(s: StatsAnswer): void {
  const tot = s.total;
  $("#statsTiles").replaceChildren(
    statTile(String(tot.books), t("Bücher angefragt"), plural(tot.requests, t("in 1 Anfrage"), t("in {n} Anfragen", { n: tot.requests }))),
    statTile(String(tot.available), t("davon verfügbar"), tot.open ? t("{n} noch offen", { n: tot.open }) : ""),
    statTile(fmtHours(tot.minutes), t("Spielzeit angefragt"), tot.minutes_available ? t("{h} schon da", { h: fmtHours(tot.minutes_available) }) : ""),
    statTile(tot.bytes ? fmtSize(tot.bytes) : "–", t("Speicher der Anfragen"), s.library_books !== null ? t("Bibliothek: {n} Bücher", { n: s.library_books }) : ""),
    statTile(fmtWait(tot.wait_median), t("typische Wartezeit"), t("von der Anfrage bis verfügbar")),
    ...(s.listen?.linked ? [statTile(fmtListen(s.listen.seconds), t("gehört"),
      t("{books} · {n} verknüpfte Konten", { books: plural(s.listen.books, t("1 Buch"), t("{n} Bücher", { n: s.listen.books })), n: s.listen.linked }))] : []));
  // the listening columns only when someone is linked to Audiobookshelf or Plex
  const listening = !!s.listen?.linked;
  for (const th of document.querySelectorAll<HTMLElement>(".stats-table th.listen-col")) th.hidden = !listening;
  $("#statsListenNote").hidden = !listening && !s.listen;
  const listenCells = (u: UserStats): HTMLElement[] => !listening ? [] : u.listen ? [
    el("td", { class: "num", title: u.listen.errors.join(" · ") }, fmtListen(u.listen.seconds)),
    el("td", { class: "num" }, u.listen.books ? `${u.listen.books}${u.listen.finished ? ` (${u.listen.finished})` : ""}` : "–"),
    el("td", { class: "when" }, u.listen.last ? new Date(u.listen.last * 1000).toLocaleDateString(LOCALE) : "–"),
  ] : [el("td", { class: "num muted", title: t("nicht verknüpft") }, "·"), el("td", { class: "num muted" }, "·"), el("td", { class: "muted" }, "·")];
  const rows = s.users.map((u) => el("tr", { class: u.books || u.listen?.seconds ? "" : "quiet" },
    el("td", { class: "name" }, el("b", {}, u.name), u.role === "admin" ? el("span", { class: "muted small" }, " · Admin") : null),
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
    el("td", { class: "when" }, u.last ? new Date(u.last * 1000).toLocaleDateString(LOCALE) : "–")));
  $("#statsRows").replaceChildren(...(rows.length ? rows : [el("tr", {}, el("td", { colspan: "15", class: "empty-row" }, t("Noch keine Konten.")))]));
  renderMonthChart(s.months);
}

/** One series (books per month): thin bars from the baseline, hover or tap shows the numbers. */
function renderMonthChart(months: StatsAnswer["months"]): void {
  const max = Math.max(1, ...months.map((m) => m.books));
  const peak = months.reduce((a, b) => (b.books > a.books ? b : a), months[0]);
  const chart = $("#statsChart");
  const tip = el("div", { class: "chart-tip", hidden: true });
  const show = (m: StatsAnswer["months"][number], bar: HTMLElement): void => {
    tip.replaceChildren(el("b", {}, monthName(m.month, true)),
      el("span", {}, plural(m.books, t("1 Buch"), t("{n} Bücher", { n: m.books }))), el("span", { class: "muted" }, fmtHours(m.minutes)));
    tip.hidden = false;
    const r = bar.getBoundingClientRect(), box = chart.getBoundingClientRect();
    const x = Math.min(Math.max(r.left - box.left + r.width / 2, 60), box.width - 60);
    tip.style.left = `${x}px`;
  };
  chart.setAttribute("aria-label", t("Angefragte Bücher je Monat, höchstens {n}", { n: peak?.books || 0 }));
  chart.replaceChildren(...months.map((m) => {
    // the peak's number sits above its bar (inside it, positioned) – it must not squeeze the bar
    const bar = el("div", { class: "mbar", style: `height:${m.books ? Math.max(2, (m.books / max) * 100) : 0}%` },
      m === peak && m.books ? el("span", { class: "peak" }, String(m.books)) : null);
    const col = el("div", { class: "col", tabindex: "0", "aria-label": `${monthName(m.month, true)}: ${m.books}` },
      el("div", { class: "plot" }, bar),
      el("span", { class: "label" }, monthName(m.month)));
    for (const ev of ["mouseenter", "focus"]) col.addEventListener(ev, () => show(m, bar));
    for (const ev of ["mouseleave", "blur"]) col.addEventListener(ev, () => { tip.hidden = true; });
    return col;
  }), tip);
  $("#statsMonthRows").replaceChildren(...months.slice().reverse().map((m) =>
    el("tr", {}, el("td", {}, monthName(m.month, true)), el("td", { class: "num" }, String(m.books)), el("td", { class: "num" }, fmtHours(m.minutes)))));
}

async function loadStats(): Promise<void> {
  const period = document.querySelector<HTMLInputElement>("#statsPeriod input:checked")?.value || "365";
  try { renderStats(await api<StatsAnswer>("GET", `/api/stats?period=${encodeURIComponent(period)}`)); }
  catch (e) { toast(errMsg(e), true); }
}

function initStats(): void {
  if (!IS_ADMIN) return;
  const saved = store.get("statsPeriod", "365");
  for (const r of document.querySelectorAll<HTMLInputElement>("#statsPeriod input")) {
    r.checked = r.value === saved;
    r.addEventListener("change", () => { store.set("statsPeriod", r.value); loadStats(); });
  }
}

initStats();
