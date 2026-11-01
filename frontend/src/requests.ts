"use strict";
// ------------------------------------------------------------------------- //
// Requests (like Overseerr): ask for a book, follow it until it is in the library.
// Admins approve or decline; with the right, the interactive release search per request.
// ------------------------------------------------------------------------- //
type RequestStatus = "pending" | "searching" | "downloading" | "processing" | "available" | "declined" | "failed";
interface BookRequest {
  id: number; book: Book; status: RequestStatus; created: number; updated: number; user?: string; mine: boolean;
  reason?: string; release?: string; error?: string | null; last_search?: number | null; found?: number; seen?: number;
  history?: { time: number; text: string }[]; blocked?: string[]; attempts?: number;
}
interface Quota { limit: number | null; used: number; left: number | null }
interface RequestList { items: BookRequest[]; quota: Quota; sees_all: boolean; may_search: boolean; pending: number | null; interval: number }
interface Verdict { ok: boolean; score: number; reason: string | null; notes: string[]; kbps: number | null }
interface JudgedRelease extends Release { verdict: Verdict }

const REQ_STATUS: Record<RequestStatus, () => string> = {
  pending: () => t("Wartet auf Freigabe"),
  searching: () => t("Wird gesucht"),
  downloading: () => t("Lädt"),
  processing: () => t("Wird verarbeitet"),
  available: () => t("Verfügbar"),
  declined: () => t("Abgelehnt"),
  failed: () => t("Fehlgeschlagen"),
};
const REQ_FILTERS: Record<string, [() => string, RequestStatus[]]> = {
  open: [() => t("Offen"), ["pending", "searching", "downloading", "processing"]],
  available: [() => t("Verfügbar"), ["available"]],
  problems: [() => t("Abgelehnt / Fehlgeschlagen"), ["declined", "failed"]],
  all: [() => t("Alle"), []],
};

const requests = {
  filter: "open",
  data: null as RequestList | null,
  keys: {} as Record<string, string>,
  current: null as BookRequest | null,
  timer: undefined as number | undefined,
};

/** Same key as the server (bookrequests.book_key): ASIN, else title words + author surname. */
function requestKey(b: Book): string {
  if (b.asin) return "asin:" + b.asin.toUpperCase();
  const plain = (s: string): string[] => s.toLowerCase().replace(/ß/g, "ss").replace(/([aou])e/g, "$1")
    .normalize("NFKD").replace(/[̀-ͯ]/g, "").match(/[a-z0-9]+/g) || [];
  const author = plain((b.authors || [""])[0]);
  return "book:" + plain(b.title).join(" ") + "|" + (author[author.length - 1] || "");
}

async function loadRequestKeys(): Promise<void> {
  try { requests.keys = (await api<{ keys: Record<string, string> }>("GET", "/api/requests/keys")).keys; } catch { /* marks only */ }
}

/** "Anfragen" on a book card / in the search – or the state of the existing request. */
function requestButton(b: Book & { requested?: string | null }): HTMLElement {
  const state = (b.requested || requests.keys[requestKey(b)]) as RequestStatus | undefined;
  if (b.owned) return el("span", { class: "rtag ok" }, icon("check"), t("In der Bibliothek"));
  if (state && state !== "declined") {
    return el("span", { class: `rtag req-${state}`, title: t("Schon angefragt") }, REQ_STATUS[state]?.() || state);
  }
  const btn = el("button", { type: "button", class: "btn small primary" }, t("Anfragen"));
  btn.addEventListener("click", async () => {
    btn.disabled = true;
    try {
      const r = await api<BookRequest>("POST", "/api/requests", { book: b });
      requests.keys[requestKey(b)] = r.status;
      btn.replaceWith(el("span", { class: `rtag req-${r.status}` }, REQ_STATUS[r.status]()));
      toast(r.status === "pending" ? t("Angefragt – wartet auf Freigabe") : t("Angefragt – Hörwerk sucht es jetzt"));
      loadRequests();
    } catch (e) { toast(errMsg(e), true); btn.disabled = false; }
  });
  return btn;
}

function requestCard(r: BookRequest): HTMLLIElement {
  const b = r.book;
  const author = (b.authors || [])[0] || "";
  const last = (r.history || [])[(r.history || []).length - 1];
  const actions: (HTMLElement | null)[] = [];
  if (IS_ADMIN && r.status === "pending") {
    actions.push(el("button", { type: "button", class: "btn small primary", onclick: () => decide(r, "approve") }, icon("check"), t("Freigeben")));
    actions.push(el("button", { type: "button", class: "btn small", onclick: () => decide(r, "decline") }, t("Ablehnen")));
  }
  if (requests.data?.may_search && ["searching", "failed", "downloading", "processing"].includes(r.status)) {
    actions.push(el("button", { type: "button", class: "btn small", onclick: () => openReleases(r) }, icon("search"), t("Releases")));
  }
  if (IS_ADMIN && ["failed", "searching"].includes(r.status)) {
    actions.push(el("button", { type: "button", class: "btn small ghost", onclick: () => decide(r, "retry") }, icon("refresh"), t("Jetzt suchen")));
  }
  if ((r.mine && ["pending", "searching", "failed", "declined"].includes(r.status)) || IS_ADMIN) {
    actions.push(el("button", { type: "button", class: "icon-btn", title: t("Anfrage zurückziehen / löschen"), onclick: async () => {
      if (!(await confirmBox(t("Anfrage „{title}“ löschen?", { title: b.title }), t("Löschen")))) return;
      try { await api("DELETE", `/api/requests/${r.id}`); loadRequests(); } catch (e) { toast(errMsg(e), true); }
    } }, icon("x")));
  }
  const detail = r.status === "declined" && r.reason ? t("Grund: {reason}", { reason: r.reason })
    : r.status === "searching" && r.last_search ? t("Zuletzt gesucht {when} – nichts Passendes ({n} Releases geprüft)", { when: fmtAgo(r.last_search), n: r.seen || 0 })
      : r.status === "downloading" && r.release ? r.release
        : r.error || (last ? last.text : "");
  return el("li", { class: `request req-${r.status}` },
    b.cover_url ? el("img", { class: "cover", src: b.cover_url, alt: "", loading: "lazy" }) : coverArt(b.title, author),
    el("div", { class: "s-body" },
      el("b", { class: "s-title", title: b.title }, b.title),
      el("span", { class: "s-author" }, (b.authors || []).join(", ")),
      genreTag(b),
      el("span", { class: "req-line" }, el("span", { class: `rtag req-${r.status}` }, REQ_STATUS[r.status]()),
        r.user && !r.mine ? el("span", { class: "muted small" }, t("von {name}", { name: r.user })) : null,
        el("span", { class: "muted small" }, fmtAgo(r.created))),
      detail ? el("span", { class: "muted small req-detail", title: detail }, detail) : null),
    actions.length ? el("div", { class: "s-actions" }, ...actions) : null);
}

async function decide(r: BookRequest, what: "approve" | "decline" | "retry"): Promise<void> {
  let body: Record<string, string> | undefined;
  if (what === "decline") {
    const reason = prompt(t("Grund für die Ablehnung (optional, sieht der Anfragende):")) ;
    if (reason === null) return;
    body = { reason };
  }
  try { await api("POST", `/api/requests/${r.id}/${what}`, body); loadRequests(); } catch (e) { toast(errMsg(e), true); }
}

function renderRequests(): void {
  const d = requests.data;
  if (!d) return;
  const badge = $("#requestBadge");
  badge.hidden = !d.pending;
  badge.textContent = String(d.pending || "");
  const chips = $("#requestFilter");
  chips.replaceChildren(...Object.entries(REQ_FILTERS).map(([key, [label, states]]) => {
    const n = states.length ? d.items.filter((r) => states.includes(r.status)).length : d.items.length;
    return el("button", { type: "button", class: "chip" + (requests.filter === key ? " on" : ""), onclick: () => {
      requests.filter = key; store.set("requestFilter", key); renderRequests();
    } }, label(), " ", el("span", { class: "n" }, String(n)));
  }));
  const q = d.quota;
  $("#requestQuota").textContent = q.limit === null ? t("Du kannst unbegrenzt anfragen.")
    : t("Diesen Monat noch {left} von {limit} Anfragen frei.", { left: q.left ?? 0, limit: q.limit });
  const states = REQ_FILTERS[requests.filter]?.[1] || [];
  const items = d.items.filter((r) => !states.length || states.includes(r.status));
  $("#requestList").replaceChildren(...(items.length ? items.map(requestCard)
    : [el("li", { class: "empty" }, d.items.length ? t("Nichts in dieser Ansicht.")
      : t("Noch keine Anfragen. Such ein Hörbuch unter „Suchen“ oder stöbere in „Entdecken“ und klick auf „Anfragen“."))]));
}

async function loadRequests(): Promise<void> {
  clearTimeout(requests.timer);
  try {
    requests.data = await api<RequestList>("GET", "/api/requests");
    for (const r of requests.data.items) requests.keys[requestKey(r.book)] = r.status;
    renderRequests();
  } catch { /* keep the last state */ }
  const busy = requests.data?.items.some((r) => ["searching", "downloading", "processing"].includes(r.status));
  if (!$("#viewRequests").hidden) requests.timer = setTimeout(loadRequests, busy ? 8000 : 30000);
}

// -- interactive search -------------------------------------------------------
async function openReleases(r: BookRequest, query = ""): Promise<void> {
  requests.current = r;
  $("#releasesTitle").textContent = r.book.title;
  const d = $<HTMLDialogElement>("#releasesDialog");
  if (!d.open) d.showModal();
  const rows = $("#releasesRows");
  rows.replaceChildren(el("tr", {}, el("td", { colspan: "5", class: "empty-row" }, t("Suche …"))));
  try {
    const res = await api<{ query: string; queries: string[]; errors: string[]; results: JudgedRelease[] }>(
      "POST", `/api/requests/${r.id}/releases`, { query });
    $<HTMLInputElement>("#releasesQuery").value = res.query;
    const ok = res.results.filter((x) => x.verdict.ok).length;
    $("#releasesNote").textContent = t("{n} Releases, davon {ok} passend", { n: res.results.length, ok }) +
      (res.errors.length ? " · " + t("Fehler: {errors}", { errors: res.errors.join("; ") }) : "");
    rows.replaceChildren(...(res.results.length ? res.results.map((x) => releaseRow(r, x))
      : [el("tr", {}, el("td", { colspan: "5", class: "empty-row" }, t("Keine Releases – anderen Suchbegriff versuchen (z. B. nur Titel).")))]));
  } catch (e) {
    rows.replaceChildren(el("tr", {}, el("td", { colspan: "5", class: "empty-row" }, errMsg(e))));
  }
}

function releaseRow(r: BookRequest, x: JudgedRelease): HTMLTableRowElement {
  const v = x.verdict;
  const grab = el("button", { type: "button", class: "btn small" + (v.ok ? " primary" : "") }, t("Laden"));
  grab.title = v.ok ? t("Dieses Release laden") : t("Trotz Bedenken laden");
  grab.addEventListener("click", async () => {
    grab.disabled = true;
    try {
      await api("POST", `/api/requests/${r.id}/grab`, { release_id: x.id, title: x.title });
      toast(t("An SABnzbd geschickt"));
      $<HTMLDialogElement>("#releasesDialog").close();
      loadRequests();
    } catch (e) { toast(errMsg(e), true); grab.disabled = false; }
  });
  const block = el("button", { type: "button", class: "icon-btn", title: t("Release sperren (wird nie automatisch geladen)") }, icon("x"));
  block.addEventListener("click", async () => {
    try { await api("POST", `/api/requests/${r.id}/block`, { title: x.title }); openReleases(r, $<HTMLInputElement>("#releasesQuery").value); }
    catch (e) { toast(errMsg(e), true); }
  });
  return el("tr", { class: v.ok ? "" : "rejected" },
    el("td", { class: "c-title" }, el("span", { title: x.title }, x.title),
      el("span", { class: "muted small" }, [x.indexer, x.format, x.language].filter(Boolean).join(" · "))),
    el("td", { class: "c-num" }, fmtSize(x.size)),
    el("td", { class: "c-num" }, v.kbps === null ? "–" : String(v.kbps)),
    el("td", {}, v.ok ? el("span", { class: "verdict ok" }, icon("check"), v.notes.join(" · ") || t("passt"))
      : el("span", { class: "verdict no" }, v.reason || "")),
    el("td", { class: "c-act" }, grab, block));
}

function initRequests(): void {
  requests.filter = store.get("requestFilter", "open");
  if (!(requests.filter in REQ_FILTERS)) requests.filter = "open";
  $("#releasesGo").addEventListener("click", () => { if (requests.current) openReleases(requests.current, $<HTMLInputElement>("#releasesQuery").value); });
  document.addEventListener("viewchange", (e) => {
    if ((e as CustomEvent<string>).detail === "requests") loadRequests(); else clearTimeout(requests.timer);
  });
  loadRequestKeys();
  loadRequests();            // the badge with pending approvals, on every view
}

initRequests();
