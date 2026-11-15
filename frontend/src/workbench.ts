"use strict";
// ------------------------------------------------------------------------- //
// Werkbank: what is being worked on right now – downloads, unpacking, recognising, converting, tagging – with how far
// it is. In the sidebar on wide screens (the free space under the menu); on the phone a slim bar above the tab bar,
// only while something is on the bench. Admins see every job, family accounts their own requests.
// ------------------------------------------------------------------------- //
interface BenchItem { key: string; title: string; step: string; pct: number | null; href: string }

const bench = { timer: undefined as number | undefined, items: [] as BenchItem[], requests: [] as BookRequest[] };

const BENCH_MAX = 4;

function benchFromRequests(list: BookRequest[]): BenchItem[] {
  return list.filter((r) => ["downloading", "processing"].includes(r.status)).map((r) => {
    const l = r.live;
    let step = REQ_STATUS[r.status] ? REQ_STATUS[r.status]() : r.status;
    let pct: number | null = null;
    if (l?.kind === "download") {
      step = l.paused && !l.done ? t("SABnzbd ist pausiert") : SAB_STATE[l.state]?.() || l.state;
      if (!l.done && (l.progress > 0 || l.state === "Downloading")) pct = Math.round(Math.min(1, l.progress || 0) * 100);
    } else if (l) {
      step = IMPORT_STATE[l.state]?.() || l.state;
      if (l.state === "converting") pct = Math.round(Math.min(1, l.progress || 0) * 100);
    }
    return { key: `r${r.id}`, title: r.book.title, step, pct, href: "/requests" };
  });
}

function benchFromJobs(jobs: AudiobookJob[], fromRequests: BenchItem[]): BenchItem[] {
  const titles = new Set(fromRequests.map((b) => b.title.toLowerCase()));
  return jobs.filter((j) => ACTIVE_STATES.includes(j.status))
    .map((j) => ({ key: `j${j.id}`, title: j.match?.title || j.hints?.title || j.name, step: BOOK_STATUS[j.status] || j.status,
      pct: j.status === "converting" ? Math.round((j.progress || 0) * 100) : null, href: "/books" }))
    .filter((b) => !titles.has(b.title.toLowerCase()));       // a request being imported is shown once, as the request
}

function renderBench(): void {
  const fromRequests = benchFromRequests(bench.requests);
  const items = [...fromRequests, ...(IS_ADMIN && books.data ? benchFromJobs(books.data.jobs, fromRequests) : [])];
  bench.items = items;
  const box = $("#workbench");
  box.hidden = !items.length && !window.matchMedia("(min-width: 780px)").matches;
  box.classList.toggle("idle", !items.length);
  $("#workbenchCount").textContent = items.length ? String(items.length) : "";
  $("#workbenchHead").setAttribute("href", items[0]?.href || (IS_ADMIN ? "/books" : "/requests"));
  if (!items.length) {
    $("#workbenchList").replaceChildren(el("li", { class: "wb-empty muted small" }, t("Gerade nichts in Arbeit")));
    return;
  }
  const shown = items.slice(0, BENCH_MAX);
  $("#workbenchList").replaceChildren(...shown.map((b, i) => el("li", {},
    el("a", { href: b.href, class: "wb-item", title: `${b.title} – ${b.step}` },
      el("span", { class: "wb-title" }, b.title),
      el("span", { class: "wb-step" }, b.pct !== null ? `${b.step} · ${b.pct} %` : b.step,
        i === 0 && items.length > 1 ? el("span", { class: "wb-plus" }, ` · +${items.length - 1}`) : null),
      el("span", { class: "wb-bar" + (b.pct === null ? " busy" : "") }, el("i", { style: b.pct !== null ? `width:${b.pct}%` : "" }))))),
  ...(items.length > shown.length ? [el("li", { class: "wb-more muted small" }, t("und {n} weitere", { n: items.length - shown.length }))] : []));
}

async function pollBench(): Promise<void> {
  clearTimeout(bench.timer);
  if (typeof ME === "undefined" || !ME) return;
  if (!document.hidden) {
    try {
      bench.requests = (await api<{ items: BookRequest[] }>("GET", "/api/requests", undefined, { key: "workbench" })).items || [];
      if (IS_ADMIN && !books.data) books.data = await api<AudiobooksStatus>("GET", "/api/audiobooks", undefined, { key: "workbench-books" });
    } catch { /* away for a moment: the last state stays */ }
    renderBench();
  }
  // something on the bench: look again soon; otherwise now and then (live updates bring the jobs anyway)
  bench.timer = setTimeout(pollBench, bench.items.length ? 6000 : 45_000);
}

onLive("audiobooks", () => { queueMicrotask(renderBench); });
document.addEventListener("visibilitychange", () => { if (!document.hidden) void pollBench(); });
window.matchMedia("(min-width: 780px)").addEventListener("change", renderBench);
void pollBench();
