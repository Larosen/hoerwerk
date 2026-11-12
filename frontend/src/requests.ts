"use strict";
// ------------------------------------------------------------------------- //
// Requests (like Overseerr): ask for a book, follow it until it is in the library.
// Admins approve or decline; with the right, the interactive release search per request.
// ------------------------------------------------------------------------- //
type RequestStatus = "pending" | "searching" | "downloading" | "processing" | "available" | "declined" | "failed";
interface BookRequest {
  id: number; book: Book; status: RequestStatus; created: number; updated: number; user?: string; mine: boolean;
  reason?: string; release?: string; error?: string | null; last_search?: number | null; found?: number; seen?: number;
  history?: { time: number; text: string }[]; blocked?: string[]; attempts?: number; group?: RequestGroup | null;
  live?: RequestLive | null; resting?: string[] | null;
  /** a better release for a book that is there: which one and how it sounds now */
  upgrade?: { path: string; have?: { format?: string; kbps?: number } | null } | null;
}
/** What a running request is doing right now: SABnzbd's download, then the conversion to M4B. */
interface RequestLive {
  kind: "download" | "import"; state: string; progress: number; done?: boolean; paused?: boolean;
  mb?: number | null; mbleft?: number | null; left?: string | null; speed?: number | null;
}
/** The volumes of a series asked for at once share a group (one request against the quota, decided together). */
interface RequestGroup { id: string; title: string; asin?: string | null; count: number }
interface SeriesAnswer { series: { asin: string; title: string; region: string }; books: (Book & { requested?: string | null })[] }
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
/** SABnzbd's states (queue and post-processing) in plain words. */
const SAB_STATE: Record<string, () => string> = {
  Queued: () => t("Wartet in SABnzbd"),
  Paused: () => t("Pausiert"),
  Downloading: () => t("Lädt"),
  Grabbing: () => t("Holt die NZB"),
  Fetching: () => t("Holt fehlende Teile"),
  Propagating: () => t("Wartet, bis alles beim Anbieter ist"),
  QuickCheck: () => t("Prüft"),
  Verifying: () => t("Prüft"),
  Checking: () => t("Prüft"),
  Repairing: () => t("Repariert"),
  Extracting: () => t("Entpackt"),
  Moving: () => t("Verschiebt in den Eingang"),
  Running: () => t("Nachbearbeitung"),
  Completed: () => t("Fertig – geht in den Eingang"),
  Failed: () => t("Fehlgeschlagen"),
};
const IMPORT_STATE: Record<string, () => string> = {
  inbox: () => t("Download fertig – wartet auf den Eingang"),
  review: () => IS_ADMIN ? t("Wartet auf deine Prüfung unter „Hörbücher“") : t("Wartet auf Prüfung durch einen Admin"),
  queued: () => t("Wartet auf die Umwandlung"),
  matching: () => t("Erkennt das Buch"),
  converting: () => t("Wandelt in M4B um"),
  tagging: () => t("Schreibt Tags und Kapitel"),
};

/** "M4B · 64 kbit/s" of the book that is there. */
function haveLabel(have?: { format?: string; kbps?: number } | null): string {
  if (!have) return "?";
  return [(have.format || "").toUpperCase(), have.kbps ? `${have.kbps} kbit/s` : ""].filter(Boolean).join(" · ") || "?";
}

/** SABnzbd's "1:02:03" in words: "1 Std. 2 Min.". */
function fmtLeft(s: string): string {
  const parts = s.split(":").map(Number);
  if (!parts.length || parts.some((n) => !Number.isFinite(n))) return s;
  const secs = parts.reduce((a, n) => a * 60 + n, 0);
  if (secs < 60) return t("unter 1 Min.");
  const h = Math.floor(secs / 3600), m = Math.round((secs % 3600) / 60);
  return h ? t("{h} Std. {m} Min.", { h, m }) : t("{m} Min.", { m });
}

function fmtMB(mb: number): string {
  const num = (n: number): string => n.toLocaleString(LOCALE, { maximumFractionDigits: 1 });
  return mb >= 1024 ? `${num(mb / 1024)} GB` : `${Math.round(mb)} MB`;
}

/** Progress, size, time left and speed while it loads; the step while it becomes an M4B. */
function liveInfo(r: BookRequest, prefix = ""): HTMLElement | null {
  const l = r.live;
  if (!l || !["downloading", "processing"].includes(r.status)) return null;
  const pct = Math.round(Math.min(1, Math.max(0, l.progress || 0)) * 100);
  const parts: string[] = prefix ? [prefix] : [];
  let bar = false;
  if (l.kind === "download") {
    parts.push(l.paused && !l.done ? t("SABnzbd ist pausiert") : SAB_STATE[l.state]?.() || l.state);
    if (!l.done) {
      const started = pct > 0 || l.state === "Downloading";
      bar = started;
      if (started) parts.push(`${pct} %`);
      if (l.mb && started) parts.push(t("{done} von {total}", { done: fmtMB(Math.max(0, l.mb - (l.mbleft ?? 0))), total: fmtMB(l.mb) }));
      else if (l.mb) parts.push(fmtMB(l.mb));
      if (l.left && !l.paused) parts.push(t("Noch {time}", { time: fmtLeft(l.left) }));
      if (l.speed && !l.paused) parts.push(`${(l.speed / 1024).toLocaleString(LOCALE, { maximumFractionDigits: 1 })} MB/s`);
    }
  } else {
    parts.push(IMPORT_STATE[l.state]?.() || l.state);
    if (l.state === "converting") { bar = true; parts.push(`${pct} %`); }
  }
  return el("span", { class: "req-live" },
    el("span", { class: "small" }, parts.join(" · ")),
    bar ? el("span", { class: "bar", role: "progressbar", "aria-valuenow": String(pct), "aria-valuemin": "0", "aria-valuemax": "100" },
      el("i", { style: `width:${pct}%` })) : null);
}

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
  releases: [] as JudgedRelease[],          // the open release window's list (re-sorted without searching again)
  timer: undefined as number | undefined,
  openGroups: new Set<string>(),            // series cards whose volume list is unfolded (survives the refresh)
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
  const btn = el("button", { type: "button", class: "btn small primary" }, t("Anfragen|Verb"));
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

// -- whole series ---------------------------------------------------------------
/** "de"/"en" of a book (for the series search: the volumes in the same language). */
function bookLangCode(b: Book): string {
  const lang = (b.language || "").toLowerCase();
  return lang === "english" ? "en" : lang === "german" || lang === "deutsch" ? "de" : "";
}

/** "Ganze Reihe" next to a book that belongs to a series. */
function seriesButton(b: Book): HTMLButtonElement | null {
  if (!b.series_asin) return null;
  return el("button", { type: "button", class: "btn small series-btn", title: t("Alle Bände der Reihe „{series}“ ansehen und anfragen", { series: b.series || "" }),
    onclick: (e: Event) => { e.stopPropagation(); openSeries(b); } }, icon("books"), t("Ganze Reihe"));
}

function volumeLabel(b: Book): string {
  return b.series_position ? t("Band {n}", { n: b.series_position }) : "";
}

/** The volumes of a series to tick: what is in the library or asked for already can't be ticked. */
async function openSeries(b: Book): Promise<void> {
  const d = $<HTMLDialogElement>("#seriesDialog");
  const ok = $<HTMLButtonElement>("#seriesOk");
  $("#seriesTitle").replaceChildren(el("span", {}, t("Reihe „{series}“", { series: b.series || b.title })), " ", langTag(b) || "");
  $("#seriesInfo").textContent = t("Lade die Bände …");
  $("#seriesVolumes").replaceChildren();
  ok.disabled = true;
  ok.textContent = t("Anfragen|Verb");
  d.returnValue = "";
  if (!d.open) d.showModal();
  let res: SeriesAnswer;
  try {
    res = await api<SeriesAnswer>("POST", "/api/search/series", { asin: b.series_asin, region: b.region || "", language: bookLangCode(b) });
  } catch (e) { $("#seriesInfo").textContent = errMsg(e); return; }
  $("#seriesTitle").replaceChildren(el("span", {}, t("Reihe „{series}“", { series: res.series.title || b.series || "" })), " ", langTag(b) || "");
  const rows = res.books.map((v) => {
    const state = (v.requested || requests.keys[requestKey(v)]) as RequestStatus | undefined;
    const taken = v.owned ? t("In der Bibliothek") : state && state !== "declined" ? REQ_STATUS[state]?.() || state : "";
    const cb = el("input", { type: "checkbox", checked: !taken, disabled: !!taken });
    const row = el("label", { class: "check series-vol" + (taken ? " is-taken" : "") }, cb,
      v.cover_url ? el("img", { class: "thumb", src: v.cover_url, alt: "", loading: "lazy" }) : coverArt(v.title, (v.authors || [])[0]),
      el("span", { class: "series-vol-text" },
        el("b", {}, [volumeLabel(v), v.title].filter(Boolean).join(" · ")),
        el("span", { class: "muted small" }, [v.runtime_min ? fmtDuration(v.runtime_min * 60) : "", v.format_type === "abridged" ? t("Gekürzt") : "",
          v.release_date ? v.release_date.slice(0, 4) : ""].filter(Boolean).join(" · ")),
        taken ? el("span", { class: v.owned ? "rtag ok" : `rtag req-${state}` }, taken) : editionNote(v)));
    return { v, cb, taken, row };
  });
  const free = rows.filter((r) => !r.taken).length;
  $("#seriesVolumes").replaceChildren(...rows.map((r) => r.row));
  $("#seriesInfo").textContent = !rows.length ? t("Bei Audible sind zu dieser Reihe keine einzelnen Bände zu finden.")
    : (plural(rows.length, t("1 Band bei Audible."), t("{n} Bände bei Audible.", { n: rows.length })) + " " +
      (free ? t("Alles Angehakte zählt zusammen als eine Anfrage. Hörwerk lädt die Bände nacheinander.")
        : t("Alle Bände sind schon da oder angefragt.")));
  const count = (): void => {
    const n = rows.filter((r) => r.cb.checked).length;
    ok.disabled = !n;
    ok.textContent = plural(n, t("1 Band anfragen"), t("{n} Bände anfragen", { n }));
  };
  rows.forEach((r) => r.cb.addEventListener("change", count));
  $("#seriesMissing").onclick = () => { rows.forEach((r) => { if (!r.taken) r.cb.checked = true; }); count(); };
  $("#seriesNone").onclick = () => { rows.forEach((r) => { r.cb.checked = false; }); count(); };
  count();
  d.addEventListener("close", async () => {
    if (d.returnValue !== "ok") return;
    const books = rows.filter((r) => r.cb.checked).map((r) => r.v);
    try {
      const out = await api<{ status: RequestStatus; items: BookRequest[]; skipped: string[] }>("POST", "/api/requests/series",
        { series: res.series, books });
      for (const r of out.items) requests.keys[requestKey(r.book)] = r.status;
      const n = out.items.length;
      toast((out.status === "pending" ? plural(n, t("Reihe angefragt (1 Band) – wartet auf Freigabe"), t("Reihe angefragt ({n} Bände) – wartet auf Freigabe", { n }))
        : plural(n, t("Reihe angefragt (1 Band) – Hörwerk sucht jetzt"), t("Reihe angefragt ({n} Bände) – Hörwerk sucht jetzt", { n }))) +
        (out.skipped.length ? " · " + t("übersprungen: {list}", { list: out.skipped.join(", ") }) : ""));
      loadRequests();
    } catch (e) { toast(errMsg(e), true); }
  }, { once: true });
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
    : r.status === "searching" && r.last_search ? t("Zuletzt gesucht {when} – nichts Passendes ({checked})", { when: fmtAgo(r.last_search),
      checked: plural(r.seen || 0, t("1 Release geprüft"), t("{n} Releases geprüft", { n: r.seen || 0 })) }) + (r.resting?.length ? " · " + restingText(r.resting) : "")
      : r.status === "downloading" && r.release ? r.release
        : r.error || (last ? last.text : "");
  return el("li", { class: `request req-${r.status}` },
    b.cover_url ? el("img", { class: "cover", src: b.cover_url, alt: "", loading: "lazy" }) : coverArt(b.title, author),
    el("div", { class: "s-body" },
      el("b", { class: "s-title", title: b.title }, b.title),
      el("span", { class: "s-author" }, (b.authors || []).join(", ")),
      el("span", { class: "tag-row" }, r.upgrade ? el("span", { class: "series-tag", title: r.upgrade.path }, icon("refresh"), t("Ersatz")) : null,
        langTag(b), genreTag(b)),
      r.upgrade ? el("span", { class: "muted small" }, t("Vorhanden: {have} – automatisch nur ein klar besseres Release, im Releases-Fenster zählt deine Wahl",
        { have: haveLabel(r.upgrade.have) })) : null,
      el("span", { class: "req-line" }, el("span", { class: `rtag req-${r.status}` }, REQ_STATUS[r.status]()),
        r.user && !r.mine ? el("span", { class: "muted small" }, t("von {name}", { name: r.user })) : null,
        el("span", { class: "muted small" }, fmtAgo(r.created))),
      liveInfo(r),
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

/** A series asked for at once: one card with the state of all volumes, the volumes unfold below. */
function groupCard(members: BookRequest[]): HTMLLIElement {
  const first = members[0];
  const g = first.group!;
  const b = first.book;
  const author = (b.authors || [])[0] || "";
  const counts = new Map<RequestStatus, number>();
  for (const m of members) counts.set(m.status, (counts.get(m.status) || 0) + 1);
  const order: RequestStatus[] = ["pending", "searching", "downloading", "processing", "available", "failed", "declined"];
  const pending = members.find((m) => m.status === "pending");
  const actions: (HTMLElement | null)[] = [];
  if (IS_ADMIN && pending) {
    actions.push(el("button", { type: "button", class: "btn small primary", onclick: () => decide(pending, "approve") }, icon("check"), t("Reihe freigeben")));
    actions.push(el("button", { type: "button", class: "btn small", onclick: () => decide(pending, "decline") }, t("Ablehnen")));
  }
  const removable = IS_ADMIN || members.some((m) => m.mine && ["pending", "searching", "failed", "declined"].includes(m.status));
  if (removable) {
    actions.push(el("button", { type: "button", class: "btn small ghost", title: t("Reihen-Anfrage zurückziehen / löschen"), onclick: async () => {
      const q = IS_ADMIN ? t("Reihe „{title}“ löschen – alle {n} Bände?", { title: g.title, n: members.length })
        : t("Reihe „{title}“ zurückziehen? Bände, die schon laden, bleiben.", { title: g.title });
      if (!(await confirmBox(q, t("Löschen")))) return;
      try { await api("DELETE", `/api/requests/${first.id}?group=1`); loadRequests(); } catch (e) { toast(errMsg(e), true); }
    } }, icon("x"), IS_ADMIN ? t("Reihe löschen") : t("Reihe zurückziehen")));
  }
  const vols = el("details", { class: "group-vols" },
    el("summary", {}, plural(members.length, t("1 Band anzeigen"), t("{n} Bände anzeigen", { n: members.length }))),
    el("ul", {}, ...members.map(volumeRow)));
  vols.open = requests.openGroups.has(g.id);
  vols.addEventListener("toggle", () => { if (vols.open) requests.openGroups.add(g.id); else requests.openGroups.delete(g.id); });
  return el("li", { class: `request request-group req-${pending ? "pending" : first.status}` },
    b.cover_url ? el("img", { class: "cover", src: b.cover_url, alt: "", loading: "lazy" }) : coverArt(g.title, author),
    el("div", { class: "s-body" },
      el("b", { class: "s-title", title: g.title }, g.title),
      el("span", { class: "s-author" }, (b.authors || []).join(", ")),
      el("span", { class: "tag-row" }, el("span", { class: "series-tag" }, icon("books"), t("Reihe")), langTag(b), genreTag(b)),
      el("span", { class: "req-line" }, ...order.filter((s) => counts.get(s)).map((s) =>
        el("span", { class: `rtag req-${s}` }, `${REQ_STATUS[s]()}: ${counts.get(s)}`)),
        first.user && !first.mine ? el("span", { class: "muted small" }, t("von {name}", { name: first.user })) : null,
        el("span", { class: "muted small" }, fmtAgo(first.created))),
      ...members.filter((m) => m.live).slice(0, 3).map((m) => liveInfo(m, volumeLabel(m.book) || m.book.title))),
    vols,
    actions.length ? el("div", { class: "s-actions" }, ...actions) : null);
}

/** One volume inside a series card: number, title, state – and what can be done with it alone. */
function volumeRow(r: BookRequest): HTMLLIElement {
  const b = r.book;
  const last = (r.history || [])[(r.history || []).length - 1];
  const detail = r.status === "declined" && r.reason ? r.reason : r.status === "downloading" && r.release ? r.release
    : r.error || (r.status === "searching" && r.resting?.length ? restingText(r.resting)
      : !r.live && ["processing", "searching"].includes(r.status) && last ? last.text : "");
  return el("li", { class: "group-vol" },
    el("span", { class: "group-vol-text" },
      el("span", { title: b.title }, [volumeLabel(b), b.title].filter(Boolean).join(" · ")),
      detail ? el("span", { class: "muted small", title: detail }, detail) : null,
      liveInfo(r)),
    el("span", { class: `rtag req-${r.status}` }, REQ_STATUS[r.status]()),
    requests.data?.may_search && ["searching", "failed", "downloading", "processing"].includes(r.status)
      ? el("button", { type: "button", class: "icon-btn", title: t("Releases"), onclick: () => openReleases(r) }, icon("search")) : null,
    IS_ADMIN && ["failed", "searching"].includes(r.status)
      ? el("button", { type: "button", class: "icon-btn", title: t("Jetzt suchen"), onclick: () => decide(r, "retry") }, icon("refresh")) : null,
    (r.mine && ["pending", "searching", "failed", "declined"].includes(r.status)) || IS_ADMIN
      ? el("button", { type: "button", class: "icon-btn", title: t("Diesen Band löschen"), onclick: async () => {
        if (!(await confirmBox(t("Anfrage „{title}“ löschen?", { title: b.title }), t("Löschen")))) return;
        try { await api("DELETE", `/api/requests/${r.id}`); loadRequests(); } catch (e) { toast(errMsg(e), true); }
      } }, icon("x")) : null);
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
  // a series is one card – shown when one of its volumes fits the filter
  const groups = new Map<string, BookRequest[]>();
  for (const r of d.items) if (r.group?.id) groups.set(r.group.id, [...(groups.get(r.group.id) || []), r]);
  const seen = new Set<string>();
  const cards: HTMLLIElement[] = [];
  for (const r of d.items) {
    const gid = r.group?.id;
    if (gid && seen.has(gid)) continue;
    const members = gid ? groups.get(gid)! : [r];
    if (states.length && !members.some((m) => states.includes(m.status))) continue;
    if (gid) seen.add(gid);
    cards.push(gid ? groupCard([...members].sort((a, b) => parseFloat(a.book.series_position || "0") - parseFloat(b.book.series_position || "0"))) : requestCard(r));
  }
  const items = cards;
  $("#requestList").replaceChildren(...(items.length ? items
    : [el("li", { class: "empty" }, d.items.length ? t("Nichts in dieser Ansicht.")
      : t("Noch keine Anfragen. Such ein Hörbuch unter „Suchen“ oder stöbere in „Entdecken“."))]));
}

async function loadRequests(): Promise<void> {
  clearTimeout(requests.timer);
  await loadList($("#requestList"), async () => {
    requests.data = await api<RequestList>("GET", "/api/requests");
    for (const r of requests.data.items) requests.keys[requestKey(r.book)] = r.status;
    renderRequests();
  }, { count: 4, quiet: true });                    // polled: a failed round keeps the last state
  const busy = requests.data?.items.some((r) => ["searching", "downloading", "processing"].includes(r.status));
  const live = requests.data?.items.some((r) => r.live);
  if (!$("#viewRequests").hidden) requests.timer = setTimeout(loadRequests, live ? 5000 : busy ? 8000 : 30000);
}

// -- interactive search -------------------------------------------------------
async function openReleases(r: BookRequest, query = ""): Promise<void> {
  requests.current = r;
  $("#releasesTitle").textContent = r.book.title;
  const d = $<HTMLDialogElement>("#releasesDialog");
  if (!d.open) d.showModal();
  const rows = $("#releasesRows");
  requests.releases = [];
  rows.replaceChildren(el("tr", {}, el("td", { colspan: "6", class: "empty-row" }, t("Suche …"))));
  try {
    const res = await api<{ query: string; queries: string[]; errors: string[]; resting?: string[]; results: JudgedRelease[] }>(
      "POST", `/api/requests/${r.id}/releases`, { query });
    $<HTMLInputElement>("#releasesQuery").value = res.query;
    const ok = res.results.filter((x) => x.verdict.ok).length;
    $("#releasesNote").textContent = t("{n} Releases, davon {ok} passend", { n: res.results.length, ok }) +
      (res.queries.length > 1 ? " · " + t("gesucht als: {queries}", { queries: res.queries.map((x) => t("„{title}“", { title: x })).join(", ") }) : "") +
      (res.errors.length ? " · " + t("Fehler: {errors}", { errors: res.errors.join("; ") }) : "") +
      (res.resting?.length ? " · " + restingText(res.resting) : "");
    requests.releases = res.results;
    renderReleases();
  } catch (e) {
    rows.replaceChildren(el("tr", {}, el("td", { colspan: "6", class: "empty-row" }, errMsg(e))));
  }
}

function renderReleases(): void {
  const r = requests.current;
  if (!r) return;
  const list = sortReleases(requests.releases, $<HTMLSelectElement>("#releasesSort").value);
  $("#releasesRows").replaceChildren(...(list.length ? list.map((x) => releaseRow(r, x))
    : [el("tr", {}, el("td", { colspan: "6", class: "empty-row" }, t("Keine Releases – anderen Suchbegriff versuchen (z. B. nur Titel).")))]));
}

function releaseRow(r: BookRequest, x: JudgedRelease): HTMLTableRowElement {
  const v = x.verdict;
  const grab = el("button", { type: "button", class: "btn small" + (v.ok ? " primary" : "") }, t("Laden"));
  grab.title = v.ok ? t("Dieses Release herunterladen") : t("Trotz Bedenken herunterladen");
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
    el("td", { class: "c-num", "data-label": t("Alter"), title: x.published ? new Date(x.published * 1000).toLocaleString(LOCALE) : "" }, fmtAge(x.published)),
    el("td", { class: "c-num", "data-label": "kbit/s" }, v.kbps === null ? "–" : String(v.kbps)),
    el("td", { class: "c-verdict" }, v.ok ? el("span", { class: "verdict ok" }, icon("check"), v.notes.join(" · ") || t("Passt"))
      : el("span", { class: "verdict no" }, v.reason || "")),
    el("td", { class: "c-act" }, grab, block));
}

function initRequests(): void {
  requests.filter = store.get("requestFilter", "open");
  if (!(requests.filter in REQ_FILTERS)) requests.filter = "open";
  initReleaseSort($<HTMLSelectElement>("#releasesSort"), renderReleases);
  $("#releasesGo").addEventListener("click", () => { if (requests.current) openReleases(requests.current, $<HTMLInputElement>("#releasesQuery").value); });
  // Enter in the search field searches again – it must not submit (and so close) the window
  $("#releasesQuery").addEventListener("keydown", (e) => {
    if (e.key !== "Enter") return;
    e.preventDefault();
    $("#releasesGo").click();
  });
  document.addEventListener("viewchange", (e) => {
    if ((e as CustomEvent<string>).detail === "requests") loadRequests(); else clearTimeout(requests.timer);
  });
  loadRequestKeys();
  loadRequests();            // the badge with pending approvals, on every view
}

initRequests();
