"use strict";

// ------------------------------------------------------------------------- //
// Audiobook pipeline view (Eingang -> M4B -> Audible -> Bibliothek)
// ------------------------------------------------------------------------- //
const BOOK_STATUS: Record<BookStatus, string> = {
  queued: t("wartet"), matching: t("sucht"), converting: t("konvertiert"), review: t("zu prüfen"),
  tagging: t("taggt"), done: t("fertig"), failed: t("Fehler"), cancelled: t("abgebrochen"),
};
type BookFilter = "library" | "all" | "review" | "active" | "done" | "failed" | "unconverted";
const BOOK_FILTERS: [BookFilter, string][] = [["library", t("Bibliothek")], ["all", t("Aufträge")], ["review", t("Zu prüfen")], ["active", t("Laufend")],
  ["done", t("Fertig")], ["failed", t("Fehler")], ["unconverted", t("Nicht als M4B")]];
const ACTIVE_STATES: BookStatus[] = ["queued", "matching", "converting", "tagging"];
const REGIONS = ["de", "us", "uk", "fr", "it", "es", "ca", "au", "in", "jp"];

const books = {
  data: null as AudiobooksStatus | null,
  filter: store.get<BookFilter>("bookFilter", "all"),
  open: new Set<string>(),                   // expanded cards
  queries: {} as Record<string, string>,     // search box contents per job (survive re-renders)
  timer: undefined as number | undefined,
  lastJson: "",
  unconverted: null as UnconvertedBook[] | null,   // library books that are not one M4B yet
  picked: new Set<string>(),                  // selected paths in that list
  tracks: null as Track[] | null,             // tracks of the book in the "Kapitel & Namen" dialog
  libQuery: "",                               // search in the library view
  libShown: 100,                              // rows loaded so far
  libTotal: null as number | null,            // books in the library (for the filter chip)
  libTimer: undefined as number | undefined,
};

function isActive(status: BookStatus): boolean {
  return ACTIVE_STATES.includes(status);
}

function setView(view: string): void {
  if (!["books", "search", "discover"].includes(view)) view = "books";
  $("#viewBooks").hidden = view !== "books";
  $("#viewSearch").hidden = view !== "search";
  $("#viewDiscover").hidden = view !== "discover";
  for (const t of document.querySelectorAll<HTMLElement>(".tab")) t.setAttribute("aria-selected", String(t.dataset.view === view));
  store.set("view", view);
  if (view === "books") pollBooks(true);
  document.dispatchEvent(new CustomEvent<string>("viewchange", { detail: view }));
}

function applyBooks(data: AudiobooksStatus, force = false): void {
  const json = JSON.stringify(data);
  if (force || json !== books.lastJson) {
    books.lastJson = json;
    books.data = data;
    renderBooks();
  }
}

async function pollBooks(force = false): Promise<void> {
  clearTimeout(books.timer);
  try { applyBooks(await api<AudiobooksStatus>("GET", "/api/audiobooks"), force); } catch { /* keep the last state, try again */ }
  if (live.ok) return;                       // live updates bring every change
  const busy = books.data?.jobs.some((j) => isActive(j.status));
  const visible = !$("#viewBooks").hidden;
  books.timer = setTimeout(pollBooks, busy && visible ? 1500 : visible ? 5000 : 15000);
}
onLive("audiobooks", (data) => applyBooks(data));
onLive("library", (st) => {
  renderLibraryStats(st);
  if (books.filter === "unconverted" && !st.scanning) loadUnconverted();
});
document.addEventListener("livechange", (e) => {
  if ((e as CustomEvent<boolean>).detail) clearTimeout(books.timer); else { pollBooks(); loadLibraryStats(); }
});

function renderBooks(): void {
  const d = books.data;
  if (!d) return;
  const badge = $("#reviewBadge");
  badge.hidden = !d.review;
  badge.textContent = String(d.review);
  if ($("#viewBooks").hidden) return;
  // Don't rebuild while the user is typing in a search box
  const active = document.activeElement;
  if (active && active.closest("#bookList") && /^(INPUT|SELECT)$/.test(active.tagName)) {
    books.lastJson = "";   // render on the next update instead
    return;
  }
  renderInbox(d);
  if (!live.ok) loadLibraryStats();   // otherwise the live updates bring it
  renderFilter(d.jobs);
  const list = $("#bookList");
  list.replaceChildren();
  if (books.filter === "unconverted") { renderUnconverted(list); return; }
  if (books.filter === "library") { renderLibrary(list); return; }
  const jobs = d.jobs.filter((j) => books.filter === "all"
    || (books.filter === "active" ? isActive(j.status)
      : books.filter === "failed" ? ["failed", "cancelled"].includes(j.status) : j.status === books.filter));
  if (!jobs.length) {
    list.append(el("li", { class: "empty" }, d.jobs.length ? t("Nichts in dieser Ansicht.")
      : d.inbox_ok ? t("Noch keine Hörbücher. Lege einen Ordner in den Eingang – er wird automatisch erkannt.")
        : t("Der Eingangsordner ist nicht eingebunden. In Docker/Unraid einen Pfad auf /inbox mappen.")));
  }
  for (const j of jobs) list.append(bookCard(j));
}

// ------------------------------------------------------------------------- //
// The whole library: browse and search (shows how every book was read)
// ------------------------------------------------------------------------- //
interface LibraryBook {
  path: string; title: string; authors: string[]; names?: string[]; series: string | null;
  series_position: string | null; asin: string | null; format: string; files: number; mtime?: number; cover?: boolean;
}

function renderLibrary(list: HTMLElement): void {
  const q = el("input", { type: "search", value: books.libQuery, placeholder: t("Titel, Autor, Serie oder Ordner"),
    "aria-label": t("Bibliothek durchsuchen") });
  const rows = el("ul", { class: "lib-rows", id: "libRows" }, el("li", { class: "empty" }, t("Lade …")));
  q.addEventListener("input", () => {
    books.libQuery = q.value;
    books.libShown = 100;
    clearTimeout(books.libTimer);
    books.libTimer = setTimeout(loadLibraryBooks, 250);
  });
  list.append(el("li", { class: "lib-search" }, icon("search"), q), el("li", { class: "lib-wrap" }, rows));
  loadLibraryBooks();
}

async function loadLibraryBooks(): Promise<void> {
  try {
    const res = await api<{ total: number; books: LibraryBook[] }>("GET",
      `/api/library/books?q=${encodeURIComponent(books.libQuery)}&limit=${books.libShown}`);
    if (!books.libQuery) books.libTotal = res.total;
    const rows = document.getElementById("libRows");
    if (!rows) return;
    rows.replaceChildren(...res.books.map(libraryRow));
    if (!res.books.length) rows.append(el("li", { class: "empty" }, books.libQuery ? t("Nichts gefunden.") : t("Die Bibliothek ist leer.")));
    if (res.total > res.books.length) {
      rows.append(el("li", { class: "lib-more" },
        el("span", { class: "muted small" }, t("{shown} von {total}", { shown: res.books.length, total: res.total })),
        el("button", { class: "btn small", onclick: () => { books.libShown += 200; loadLibraryBooks(); } }, t("Mehr anzeigen"))));
    }
  } catch (e) { toast(errMsg(e), true); }
}

function libraryRow(b: LibraryBook): HTMLLIElement {
  const author = (b.authors || [])[0] || "";
  // further names, without the pieces of longer ones ("Kleist" of "Kleist, Heinrich von")
  const all = [...(b.authors || []), ...(b.names || [])];
  const other = (b.names || []).filter((n, i, arr) => arr.indexOf(n) === i && !(b.authors || []).includes(n)
    && !all.some((m) => m !== n && m.includes(n)));
  return el("li", { class: "lib-row" },
    libraryCover(b.path, b.title || b.path, author, b.format !== "m4b" ? (b.format || "").toUpperCase() : "", b.mtime || "", !!b.cover),
    el("div", { class: "lib-main" },
      el("div", { class: "lib-title" }, b.title || b.path,
        b.series ? el("span", { class: "muted" }, ` · ${b.series}${b.series_position ? " " + b.series_position : ""}`) : null),
      el("div", { class: "book-sub" }, author || t("Autor unbekannt"),
        other.length ? el("span", { class: "faint", title: t("Weitere Namen aus Tags und Ordnern – zählen beim Abgleich mit") },
          " · " + other.slice(0, 4).join(" / ")) : null),
      el("code", { class: "lib-path" }, b.path)),
    el("div", { class: "lib-side" },
      b.format === "m4b" ? el("span", { class: "tag" }, "M4B") : el("span", { class: "tag soft" }, t("{n} Datei(en)", { n: b.files })),
      b.asin ? el("span", { class: "muted small" }, b.asin) : null));
}

// ------------------------------------------------------------------------- //
// Library books that are not one M4B yet
// ------------------------------------------------------------------------- //
async function loadUnconverted(): Promise<void> {
  try {
    const res = await api<{ books: UnconvertedBook[] }>("GET", "/api/library/unconverted");
    books.unconverted = res.books;
    const paths = new Set(res.books.map((b) => b.path));
    for (const p of [...books.picked]) if (!paths.has(p)) books.picked.delete(p);
    books.lastJson = "";
    renderBooks();
  } catch (e) { toast(errMsg(e), true); }
}

function pick(path: string, on: boolean): void {
  if (on) books.picked.add(path); else books.picked.delete(path);
}

function renderUnconverted(list: HTMLElement): void {
  const items = books.unconverted;
  if (!items) { list.append(el("li", { class: "empty" }, t("Lade …"))); return; }
  const free = items.filter((b) => !b.job);
  const all = el("input", { type: "checkbox", checked: free.length > 0 && free.every((b) => books.picked.has(b.path)) });
  all.addEventListener("change", () => { for (const b of free) pick(b.path, all.checked); renderBooks(); });
  const bar = el("li", { class: "uc-bar" },
    el("label", { class: "check" }, all, t("Alle ({n})", { n: free.length })),
    el("span", { class: "muted small" }, items.length
      ? t("Ausgewählte werden zu M4B mit Kapiteln, bei Audible erkannt und getaggt. Die alten Dateien werden erst weggeräumt, wenn die neue M4B fertig in der Bibliothek liegt.")
      : t("Alle Bücher der Bibliothek liegen schon als M4B vor.")),
    settingSelect("ab_library_originals", t("Alte Dateien danach:"), [
      ["archive", t("nach _ersetzt im Eingang verschieben")], ["delete", t("löschen")]]),
    el("button", {
      class: "btn small primary", disabled: !books.picked.size,
      onclick: () => convertLibrary([...books.picked].map((path) => ({ path }))),
    }, books.picked.size ? t("{n} zu M4B", { n: books.picked.size }) : t("zu M4B")));
  list.append(bar);
  for (const b of items) {
    const cb = el("input", { type: "checkbox", checked: books.picked.has(b.path), disabled: !!b.job });
    cb.addEventListener("change", () => { pick(b.path, cb.checked); renderBooks(); });
    list.append(el("li", { class: "book uc" },
      el("div", { class: "book-head" },
        el("label", { class: "uc-check" }, cb),
        libraryCover(b.path, b.title || b.path, (b.authors || [])[0], (b.format || "").toUpperCase(), "", !!b.cover),
        el("div", { class: "book-main" },
          el("div", { class: "book-title" }, el("span", { class: "name" }, b.title || b.path),
            b.job ? el("span", { class: "badge " + b.job.status }, BOOK_STATUS[b.job.status] || b.job.status) : null),
          el("div", { class: "book-sub" }, [(b.authors || []).join(", "),
            b.series ? `${b.series}${b.series_position ? " " + b.series_position : ""}` : "",
            t("{n} Datei(en)", { n: b.files })].filter(Boolean).join(" · ")),
          el("div", { class: "book-dest" }, el("code", {}, b.path)))),
      b.job ? null : el("div", { class: "book-tools" },
        el("button", { class: "btn small", onclick: () => openTracks(b) }, t("Kapitel & Namen …")),
        el("button", { class: "btn small ghost", onclick: () => convertLibrary([{ path: b.path }]) }, t("Direkt zu M4B")))));
  }
}

interface ConvertItem { path: string; chapter_names?: string[]; rename_files?: boolean }

async function convertLibrary(items: ConvertItem[]): Promise<void> {
  try {
    const res = await api<{ jobs: AudiobookJob[] }>("POST", "/api/library/convert", { items });
    toast(t("{n} Buch/Bücher in Arbeit – Fortschritt unter „Laufend“", { n: res.jobs.length }));
    for (const it of items) books.picked.delete(it.path);
    await loadUnconverted();
    pollBooks(true);
  } catch (e) { toast(errMsg(e), true); }
}

const TRACK_PRESETS: Record<string, (t: Track, i: number) => string> = {
  auto: (t) => t.auto,
  filename: (t) => t.filename,
  tag: (t) => t.tag || t.auto,
  numbered: (_t, i) => t("Kapitel {n}", { n: i + 1 }),
  strip: (t) => t.auto.replace(/^(?:cd|disc|teil|part)?\s*\d+\s*[-_.)]?\s*/i, "") || t.auto,
};

async function openTracks(b: UnconvertedBook): Promise<void> {
  const d = $<HTMLDialogElement>("#tracksDialog");
  d.dataset.path = b.path;
  books.tracks = null;          // never show the tracks of the previously opened book
  $("#tracksTitle").textContent = b.title || b.path;
  $("#tracksPath").textContent = b.path;
  const body = $("#tracksBody");
  body.replaceChildren(el("tr", {}, el("td", { colspan: "4", class: "empty-row" }, t("Lade Tracks …"))));
  $<HTMLInputElement>("#tracksRename").checked = false;
  d.showModal();
  try {
    const res = await api<{ tracks: Track[] }>("GET", "/api/library/tracks?" + new URLSearchParams({ path: b.path }));
    books.tracks = res.tracks;
    fillTracks("auto");
  } catch (e) { body.replaceChildren(el("tr", {}, el("td", { colspan: "4", class: "err" }, errMsg(e)))); }
}

function fillTracks(preset: string): void {
  const tracks = books.tracks;
  const fn = TRACK_PRESETS[preset];
  if (!tracks || !fn) return;       // still loading
  $("#tracksBody").replaceChildren(...tracks.map((tr, i) => el("tr", {},
    el("td", { class: "c-num" }, String(i + 1)),
    el("td", { class: "trk-file", title: tr.file }, tr.file),
    el("td", { class: "c-num" }, fmtDuration(tr.duration)),
    el("td", {}, el("input", { type: "text", value: fn(tr, i), "aria-label": t("Kapitel {n}", { n: i + 1 }) })))));
}

async function convertTracks(): Promise<void> {
  const d = $<HTMLDialogElement>("#tracksDialog");
  if (!books.tracks) return;
  const names = [...document.querySelectorAll<HTMLInputElement>("#tracksBody input")].map((i) => i.value);
  if (names.some((n) => !n.trim())) { toast(t("Jedes Kapitel braucht einen Namen"), true); return; }
  d.close();
  await convertLibrary([{ path: d.dataset.path || "", chapter_names: names, rename_files: $<HTMLInputElement>("#tracksRename").checked }]);
}

async function loadLibraryStats(): Promise<void> {
  try { renderLibraryStats(await api<LibraryStats>("GET", "/api/library")); } catch { /* ignore */ }
}

function renderLibraryStats(st: LibraryStats): void {
  if (books.libTotal !== st.books) {
    books.libTotal = st.books;
    if (books.filter === "library" && !$("#viewBooks").hidden && !st.scanning) loadLibraryBooks();
  }
  $("#libraryInfo").replaceChildren(
    el("div", { class: "kv" }, el("span", {}, t("Bücher")),
      el("span", {}, st.scanning ? t("wird eingelesen …") : `${st.books}` + (st.other ? " " + t("(davon {n} als MP3 & Co.)", { n: st.other }) : "")), el("span", {})),
    el("div", { class: "kv" }, el("span", {}, t("Eingelesen")), el("span", {}, fmtAgo(st.last_scan)), el("span", {})),
    el("div", { class: "muted small" }, t("Neue Funde werden mit der Bibliothek abgeglichen – in der Suche und vor dem Ablegen.")));
}

function renderInbox(d: AudiobooksStatus): void {
  const ok = (flag: boolean): HTMLSpanElement => el("span", { class: flag ? "ok" : "err" }, icon(flag ? "check" : "alert", flag ? "OK" : t("Fehler")));
  const box = $("#inboxInfo");
  box.replaceChildren(
    el("div", { class: "kv" }, el("span", {}, t("Eingang")), el("code", { title: d.inbox }, d.inbox || "–"), ok(d.inbox_ok)),
    el("div", { class: "kv" }, el("span", {}, t("Bibliothek")), el("code", { title: d.library }, d.library || "–"), ok(d.library_ok)),
    el("div", { class: "kv" }, el("span", {}, t("Überwachung")),
      el("span", {}, d.enabled ? t("an · zuletzt geprüft {when}", { when: fmtAgo(d.last_scan) }) : t("aus (Einstellungen)"))),
  );
  if (d.queues) {
    const q = (x: QueueStat): string => x.running ? t("{n} läuft", { n: x.running }) + (x.waiting ? ", " + t("{n} warten", { n: x.waiting }) : "")
      : x.waiting ? t("{n} warten", { n: x.waiting }) : t("frei");
    box.append(el("div", { class: "kv" }, el("span", {}, t("Kodieren")), el("span", {}, q(d.queues.encode))),
      el("div", { class: "kv" }, el("span", {}, t("Ablegen")), el("span", {}, q(d.queues.io))));
  }
  if (d.scan_note) box.append(el("div", { class: "muted small" }, d.scan_note));
  if (d.waiting.length) {
    box.append(el("div", { class: "small" }, t("Wird noch kopiert / wartet auf Ruhe:")),
      el("ul", { class: "waiting" }, d.waiting.map((w) => el("li", {}, w.name))));
  }
}

function renderFilter(jobs: AudiobookJob[]): void {
  const counts: Record<BookFilter, number> = {
    all: jobs.length,
    review: jobs.filter((j) => j.status === "review").length,
    active: jobs.filter((j) => isActive(j.status)).length,
    done: jobs.filter((j) => j.status === "done").length,
    failed: jobs.filter((j) => ["failed", "cancelled"].includes(j.status)).length,
    unconverted: books.unconverted ? books.unconverted.length : 0,
    library: books.libTotal ?? 0,
  };
  $("#bookFilter").replaceChildren(...BOOK_FILTERS.map(([k, label]) => el("button", {
    class: "chip" + (books.filter === k ? " on" : "") + (k === "review" && counts.review ? " attention" : ""),
    onclick: () => { books.filter = k; store.set("bookFilter", k); if (k === "unconverted") loadUnconverted(); renderBooks(); },
  }, `${label} ${counts[k] || ""}`.trim())));
}

/** Title and author for the stand-in cover: the match, else "Author - Title" from the folder name. */
function jobCoverText(j: AudiobookJob): [string, string] {
  if (j.match?.title) return [j.match.title, (j.match.authors || [])[0] || ""];
  const [first, ...rest] = (j.name || j.rel).split(" - ");
  return rest.length ? [rest.join(" - "), first] : [first, ""];
}

function bookCard(j: AudiobookJob): HTMLLIElement {
  const pct = Math.round((j.progress || 0) * 100);
  const isOpen = books.open.has(j.id) || j.status === "review";
  const li = el("li", { class: "book " + j.status });
  const m = j.match;
  const head = el("div", { class: "book-head" },
    m?.cover_url ? el("img", { class: "thumb", src: m.cover_url, alt: "", loading: "lazy" }) : coverArt(...jobCoverText(j)),
    el("div", { class: "book-main" },
      el("div", { class: "book-title" },
        el("span", { class: "name", title: j.source }, m?.title ? m.title : j.rel),
        el("span", { class: "badge " + j.status }, BOOK_STATUS[j.status] + (j.status === "converting" ? ` ${pct}%` : ""))),
      m?.title ? el("div", { class: "book-sub" }, [
        (m.authors || []).join(", "),
        m.series ? `${m.series}${m.series_position ? " " + m.series_position : ""}` : "",
        m.narrators?.length ? t("gelesen von {names}", { names: m.narrators.join(", ") }) : "",
      ].filter(Boolean).join(" · ")) : null,
      el("div", { class: "book-msg" }, j.status === "failed" ? j.error || j.message : j.message),
      isActive(j.status) ? el("div", { class: "bar" }, el("i", { style: `width:${pct}%` })) : null,
      j.status === "done" && j.dest ? el("div", { class: "book-dest" }, "→ ", el("code", {}, j.dest)) : null,
      j.status === "failed" && j.dest ? el("div", { class: "book-dest" }, "Originale: ", el("code", {}, j.dest)) : null,
    ),
  );
  li.append(head);

  const tools = el("div", { class: "book-tools" });
  if (isActive(j.status) || j.status === "review") {
    tools.append(el("button", { class: "btn small ghost", onclick: () => bookAction(j, "cancel") }, t("Abbrechen")));
  }
  if (["failed", "cancelled"].includes(j.status)) {
    tools.append(el("button", { class: "btn small", onclick: () => bookAction(j, "retry") }, t("Erneut versuchen")));
  }
  if (j.status === "done" && j.dest) {
    tools.append(el("button", {
      class: "btn small", title: t("Falsch erkannt? Buch aus der Bibliothek zurückholen und neu zuordnen"),
      onclick: () => reopenBook(j),
    }, t("Korrigieren")));
  }
  if (!isActive(j.status)) {
    tools.append(el("button", {
      class: "btn small ghost", title: t("Aus der Liste entfernen (Dateien bleiben, wie sie sind)"),
      onclick: () => removeBook(j),
    }, t("Entfernen")));
  }
  if (j.status !== "review") {
    tools.append(el("button", {
      class: "btn small ghost",
      onclick: () => { if (books.open.has(j.id)) books.open.delete(j.id); else books.open.add(j.id); renderBooks(); },
    }, isOpen ? t("Weniger") : t("Details")));
  }
  li.append(tools);
  if (isOpen) li.append(bookDetails(j));
  return li;
}

function bookDetails(j: AudiobookJob): HTMLDivElement {
  const box = el("div", { class: "book-details" });
  const facts: [string, string][] = [
    [t("Quelle"), j.source], [t("Dateien"), String(j.files?.length || 0)], [t("Laufzeit"), fmtDuration(j.duration)],
    [t("Suchbegriff"), j.hints?.query || "–"],
  ];
  if (j.converted_info?.bitrate) facts.push([t("Konvertierung"), t("{bitrate}, {n} Kapitel", { bitrate: j.converted_info.bitrate, n: j.converted_info.chapters ?? 0 })]);
  box.append(el("dl", { class: "facts" }, facts.flatMap(([k, v]) => [el("dt", {}, k), el("dd", {}, v)])));
  if (j.status === "review" && j.duplicate) box.append(duplicateBox(j, j.duplicate));

  if (j.status === "review" || (j.candidates?.length && j.status !== "done")) {
    box.append(searchForm(j));
    if (j.candidates?.length) {
      box.append(el("ul", { class: "cands" }, j.candidates.map((c, i) => candidate(j, c, i === 0))));
    } else {
      box.append(el("p", { class: "muted" }, t("Keine Treffer. Anders suchen, einen Link von Audible, BookBeat oder Thalia einfügen – oder die Angaben selbst eintragen.")));
    }
    box.append(el("div", { class: "row gap wrap" },
      el("button", { class: "btn small", onclick: () => manualBook(j) }, t("Selbst eintragen …"))));
  }
  if (j.status !== "review") {
    const pre = el("pre", { "data-log": j.id }, "…");
    const log = el("details", { class: "log" }, el("summary", {}, t("Protokoll")), pre);
    log.addEventListener("toggle", async () => {
      if (!log.open) return;
      try {
        const full = await api<AudiobookJob>("GET", `/api/audiobooks/${j.id}`);
        pre.textContent = (full.log || []).join("\n") || "–";
      } catch (e) { pre.textContent = errMsg(e); }
    });
    box.append(log);
  }
  return box;
}

const DUP_WHY: Record<string, string> = { asin: t("gleiche ASIN"), title: t("gleicher Autor und Titel"), series: t("gleiche Serie und Folge") };

function duplicateBox(j: AudiobookJob, d: Duplicate): HTMLDivElement {
  const act = async (action: string, label: string): Promise<void> => {
    if (action === "replace" && !(await confirmBox(t("Die vorhandene Fassung „{path}“ aus der Bibliothek nehmen (sie wird nach _ersetzt im Eingang verschoben, nicht gelöscht) und durch die neue ersetzen?", { path: d.path }), t("Ersetzen")))) return;
    try { await api("POST", `/api/audiobooks/${j.id}/duplicate`, { action }); toast(label); pollBooks(true); }
    catch (e) { toast(errMsg(e), true); }
  };
  return el("div", { class: "dup-box" },
    el("div", {}, el("b", {}, t("Schon in der Bibliothek:"), " "), el("code", {}, d.path),
      el("span", { class: "muted" }, ` – ${DUP_WHY[d.why] || t("ähnlich")}${d.format && d.format !== "m4b" ? ", " + t("dort als {format}", { format: d.format.toUpperCase() }) : ""}`)),
    el("div", { class: "row gap wrap" },
      el("button", { class: "btn small", onclick: () => act("discard", t("Verworfen")) }, t("Verwerfen")),
      el("button", { class: "btn small", onclick: () => act("replace", t("Wird ersetzt")) }, t("Altes ersetzen")),
      el("button", { class: "btn small ghost", onclick: () => act("keep", t("Wird zusätzlich abgelegt")) }, t("Trotzdem zusätzlich ablegen"))),
    el("div", { class: "muted small" }, t("Falscher Treffer? Unten einen anderen wählen – dann wird erneut geprüft.")));
}

function searchForm(j: AudiobookJob): HTMLFormElement {
  const q = el("input", {
    type: "search", value: books.queries[j.id] ?? j.hints?.query ?? "", placeholder: t("Titel Autor, ASIN – oder Link (Audible, BookBeat, Thalia)"),
    "aria-label": t("Bei Audible suchen oder Link einfügen"),
  });
  q.addEventListener("input", () => { books.queries[j.id] = q.value; });
  const region = el("select", { "aria-label": t("Audible-Shop") }, REGIONS.map((r) => el("option", { value: r }, r)));
  region.value = j.region || String(state.settings.ab_region || "de");
  const go = el("button", { class: "btn small", type: "submit" }, t("Suchen"));
  const form = el("form", { class: "search-row" }, q, region, go);
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    go.disabled = true;
    go.textContent = /^\s*(https?:\/\/|www\.)/i.test(q.value) ? t("Lade …") : t("Suche …");
    try {
      await api("POST", `/api/audiobooks/${j.id}/search`, { query: q.value, region: region.value });
      (document.activeElement as HTMLElement | null)?.blur();
      await pollBooks(true);
    } catch (err) { toast(errMsg(err), true); } finally { go.disabled = false; go.textContent = t("Suchen"); }
  });
  return form;
}

function candidate(j: AudiobookJob, c: Book, best = false): HTMLLIElement {
  const runtime = c.runtime_min ? c.runtime_min * 60 : 0;
  const diff = runtime && j.duration ? Math.abs(j.duration - runtime) / runtime : null;
  const score = Math.round((c.match?.score || 0) * 100);
  const narrators = c.narrators || [];
  return el("li", { class: "cand" },
    c.cover_url ? el("img", { class: "thumb", src: c.cover_url, alt: "", loading: "lazy" }) : coverArt(c.title, (c.authors || [])[0]),
    el("div", { class: "cand-main" },
      el("div", { class: "cand-title" }, c.title, c.subtitle ? el("span", { class: "muted" }, " – " + c.subtitle) : null),
      el("div", { class: "cand-sub" }, [
        (c.authors || []).join(", "),
        narrators.length ? t("Sprecher: {names}", { names: narrators.slice(0, 3).join(", ") + (narrators.length > 3 ? " …" : "") }) : "",
      ].filter(Boolean).join(" · ")),
      el("div", { class: "cand-meta" },
        c.series ? el("span", { class: "tag" }, `${c.series}${c.series_position ? " #" + c.series_position : ""}`) : null,
        c.format_type && c.format_type !== "unabridged" ? el("span", { class: "tag soft" }, c.format_type === "abridged" ? t("gekürzt") : c.format_type) : null,
        el("span", { class: diff === null ? "" : diff <= 0.03 ? "ok" : diff > 0.15 ? "err" : "" },
          `${fmtDuration(runtime)}${diff === null ? "" : diff <= 0.03 ? " ✓" : " " + t("(Dateien: {duration})", { duration: fmtDuration(j.duration) })}`),
        c.release_date ? el("span", {}, c.release_date.slice(0, 4)) : null,
        c.publisher ? el("span", { class: "muted" }, c.publisher) : null,
        el("span", { class: "tag soft" }, SOURCE_NAMES[c.source || "audible"] || "Audible"),
        c.url ? el("a", { href: c.url, target: "_blank", rel: "noopener" }, c.asin || t("Seite")) : null,
      ),
    ),
    el("div", { class: "cand-side" },
      el("span", { class: "score " + (score >= 80 ? "hi" : score >= 60 ? "mid" : "lo"), style: `--p:${score}%`, title: t("Übereinstimmung mit den Dateien") }, score + " %"),
      el("div", { class: "row gap" },
        el("button", { class: "btn small ghost", title: t("Angaben vor dem Übernehmen ändern"), onclick: () => manualBook(j, c) }, t("Bearbeiten …")),
        el("button", { class: best ? "btn small primary" : "btn small accent", onclick: () => chooseBook(j, c) }, t("Übernehmen")))));
}

async function chooseBook(j: AudiobookJob, c: Book): Promise<void> {
  try {
    await api("POST", `/api/audiobooks/${j.id}/choose`, { id: c.id || c.asin, asin: c.asin, region: c.region });
    toast(t("„{title}“ wird getaggt und abgelegt", { title: c.title }));
    pollBooks(true);
  } catch (e) { toast(errMsg(e), true); }
}

const MANUAL_FIELDS = ["title", "subtitle", "authors", "narrators", "series", "series_position", "release_date", "publisher", "genres", "description"] as const;

function fillManual(c: Book): void {
  const f = $<HTMLFormElement>("#manualForm");
  for (const k of MANUAL_FIELDS) {
    const v = c[k];
    field<HTMLInputElement | HTMLTextAreaElement>(f, k).value = Array.isArray(v) ? v.join(", ") : String(v ?? "");
  }
  $("#manualDialog").dataset.baseId = c.id || c.asin || "";
  const source = $("#manualSource");
  source.replaceChildren(
    ...(c.cover_url ? [el("img", { class: "thumb", src: c.cover_url, alt: "" })] : []),
    el("span", {}, t("Vorlage: {source}", { source: SOURCE_NAMES[c.source || "audible"] || "Audible" })
      + (c.runtime_min ? " · " + fmtDuration(c.runtime_min * 60) : "") + " – "
      + (c.asin ? t("Cover, ASIN und Kapitel kommen von dort, alles hier Eingetragene gilt.")
        : t("Das Cover kommt von dort, alles hier Eingetragene gilt."))));
  source.hidden = false;
}

function manualBook(j: AudiobookJob, c: Book | null = null): void {
  const d = $<HTMLDialogElement>("#manualDialog");
  const f = $<HTMLFormElement>("#manualForm");
  f.reset();
  d.dataset.jobId = j.id;
  d.dataset.baseId = "";
  $("#manualSource").hidden = true;
  $("#manualTitle").textContent = c ? t("Angaben bearbeiten") : t("Selbst eintragen");
  if (c) fillManual(c);
  else {
    field(f, "title").value = j.hints?.title || j.hints?.folder || j.name || "";
    field(f, "authors").value = j.hints?.author || "";
    field(f, "narrators").value = j.hints?.narrator || "";
  }
  d.showModal();
}

async function fetchLinkIntoManual(): Promise<void> {
  const d = $("#manualDialog");
  const input = $<HTMLInputElement>("#manualLink");
  const btn = $<HTMLButtonElement>("#manualLinkBtn");
  if (!input.value.trim()) { input.focus(); return; }
  btn.disabled = true;
  btn.textContent = t("Lade …");
  try {
    const res = await api<{ job: AudiobookJob; candidate: Book }>("POST", `/api/audiobooks/${d.dataset.jobId}/link`, { url: input.value });
    fillManual(res.candidate);
    toast(t("Daten von {source} übernommen – bitte prüfen", { source: SOURCE_NAMES[res.candidate.source || "audible"] || "Audible" }));
    pollBooks(true);
  } catch (e) { toast(errMsg(e), true); } finally { btn.disabled = false; btn.textContent = t("Daten holen"); }
}

async function saveManual(): Promise<void> {
  const d = $("#manualDialog");
  const data: Record<string, FormDataEntryValue> = Object.fromEntries(new FormData($<HTMLFormElement>("#manualForm")).entries());
  delete data.link;
  data.base_id = d.dataset.baseId || "";
  try {
    await api("POST", `/api/audiobooks/${d.dataset.jobId}/manual`, data);
    toast(t("Wird mit deinen Angaben getaggt und abgelegt"));
    pollBooks(true);
  } catch (e) { toast(errMsg(e), true); }
}

async function reopenBook(j: AudiobookJob): Promise<void> {
  if (!(await confirmBox(t("Buch aus der Bibliothek zurückholen und neu zuordnen? Die M4B wandert zurück in die Prüf-Liste, desc.txt/reader.txt/cover.jpg werden entfernt."), t("Korrigieren")))) return;
  try {
    await api("POST", `/api/audiobooks/${j.id}/reopen`);
    books.filter = "review"; store.set("bookFilter", "review");
    pollBooks(true);
  } catch (e) { toast(errMsg(e), true); }
}

async function bookAction(j: AudiobookJob, action: "cancel" | "retry"): Promise<void> {
  try { await api("POST", `/api/audiobooks/${j.id}/${action}`); pollBooks(true); } catch (e) { toast(errMsg(e), true); }
}

async function removeBook(j: AudiobookJob): Promise<void> {
  const inInbox = j.origin === "inbox" && ["review", "cancelled"].includes(j.status);
  if (inInbox && !(await confirmBox(t("Aus der Liste entfernen? Liegt der Ordner noch im Eingang, wird er beim nächsten Prüfen erneut aufgenommen."), t("Entfernen")))) return;
  try { await api("DELETE", `/api/audiobooks/${j.id}`); books.open.delete(j.id); pollBooks(true); } catch (e) { toast(errMsg(e), true); }
}

let pathTimer: number | undefined;
function updatePathPreview(): void {
  clearTimeout(pathTimer);
  pathTimer = setTimeout(async () => {
    const f = $<HTMLFormElement>("#settingsForm");
    try {
      const res = await api<{ examples: string[] }>("GET", "/api/audiobooks/paths?" + new URLSearchParams({
        folder: field(f, "ab_folder_template").value, file: field(f, "ab_file_template").value,
      }));
      $("#pathPreview").replaceChildren(...res.examples.map((p) => el("code", {}, p)));
    } catch { /* ignore */ }
  }, 200);
}

function initBooks(): void {
  for (const t of document.querySelectorAll<HTMLElement>(".tab")) t.addEventListener("click", () => setView(t.dataset.view || ""));
  $("#scanBtn").addEventListener("click", async () => {
    try { await api("POST", "/api/audiobooks/scan"); toast(t("Eingang wird geprüft")); setTimeout(() => pollBooks(true), 800); }
    catch (e) { toast(errMsg(e), true); }
  });
  for (const btn of document.querySelectorAll<HTMLElement>("[data-preset]")) btn.addEventListener("click", () => fillTracks(btn.dataset.preset || ""));
  $("#tracksGo").addEventListener("click", convertTracks);
  $("#libraryScanBtn").addEventListener("click", async () => {
    try {
      await api("POST", "/api/library/scan"); toast(t("Bibliothek wird eingelesen"));
      if (!live.ok) setTimeout(() => { loadLibraryStats(); loadUnconverted(); }, 2500);
    }
    catch (e) { toast(errMsg(e), true); }
  });
  $("#clearBooksBtn").addEventListener("click", async () => {
    try { await api("DELETE", "/api/audiobooks"); pollBooks(true); } catch (e) { toast(errMsg(e), true); }
  });
  $("#manualLinkBtn").addEventListener("click", fetchLinkIntoManual);
  $("#manualLink").addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); fetchLinkIntoManual(); } });
  const manualDialog = $<HTMLDialogElement>("#manualDialog");
  manualDialog.addEventListener("close", () => { if (manualDialog.returnValue === "save") saveManual(); });
  const f = $<HTMLFormElement>("#settingsForm");
  field(f, "ab_folder_template").addEventListener("input", updatePathPreview);
  field(f, "ab_file_template").addEventListener("input", updatePathPreview);
  $("#settingsBtn").addEventListener("click", () => {
    updatePathPreview();
    const d = books.data;
    $("#abPaths").textContent = d ? t("Eingang: {inbox} · Bibliothek: {library} (über Docker-Pfade einstellen)", { inbox: d.inbox || "–", library: d.library || "–" }) : "";
  });
  loadUnconverted();
  setView(store.get("view", "books"));
  pollBooks(true);
}

initBooks();
