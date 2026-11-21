"use strict";

// ------------------------------------------------------------------------- //
// Audiobook pipeline view (Eingang -> M4B -> Audible -> Bibliothek)
// ------------------------------------------------------------------------- //
const BOOK_STATUS: Record<BookStatus, string> = {
  queued: t("Wartet"), matching: t("Sucht"), converting: t("Konvertiert"), review: t("Zu prüfen"),
  tagging: t("Schreibt Tags"), done: t("Fertig"), failed: t("Fehler"), cancelled: t("Abgebrochen"),
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
  ucShown: 200,                               // cards shown in that list (thousands would make the page sluggish)
  tracks: null as Track[] | null,             // tracks of the book in the "Kapitel & Namen" dialog
  libQuery: "",                               // search in the library view
  trickle: null as Trickle | null,            // the gentle conversion of the whole library
  libSort: store.get("librarySort", "author"),  // order of the library view
  libView: store.get("libraryView", "regal"),   // "regal" (covers on shelves) or "list"
  libShown: 100,                              // rows loaded so far
  libTotal: null as number | null,            // books in the library (for the filter chip)
  libTimer: undefined as number | undefined,
  libIssue: "",                              // library filter: only books with this finding
  drawSort: null as (() => void) | null,     // redraws the sort keys of the library
  check: null as LibraryCheck | null,
  ratings: null as LibraryRatings | null,    // fetching Audible's ratings of the library        // the library check (progress and findings)
  checkTimer: undefined as number | undefined,
  libMatch: 0,                                // how many books match the search and the filter
  libPicked: new Set<string>(),               // library books ticked for "Kapitel neu benennen"
  hideDone: store.get("booksHideDone", false),   // "Aufträge" without the finished ones (only hidden, never deleted)
};

function isActive(status: BookStatus): boolean {
  return ACTIVE_STATES.includes(status);
}

// Jobs started from this page: a click only means "accepted" – when the work itself ends (filed, needs a
// decision, failed), a message says so, also when the list is not on screen.
const followed = new Map<string, { name: string; ran: boolean }>();      // job id -> name, seen running

function follow(id: string | undefined, name: string): void {
  if (id) followed.set(id, { name, ran: false });
}

function reportFollowed(data: AudiobooksStatus): void {
  for (const j of data.jobs) {
    const f = followed.get(j.id);
    if (!f) continue;
    if (isActive(j.status)) { f.ran = true; continue; }
    // "review"/"failed" right after the click is still the old state – only what comes after running counts
    if (!f.ran && j.status !== "done") continue;
    const name = f.name;
    if (j.status === "done") toast(t("Fertig abgelegt: {name}", { name }));
    else if (j.status === "failed") toast(t("Fehlgeschlagen: {name} – {error}", { name, error: j.error || j.message }), "error");
    else if (j.status === "review") toast(t("Braucht deine Entscheidung: {name}", { name }), "warning");
    else if (j.status === "cancelled") toast(t("Abgebrochen: {name}", { name }), "info");
    followed.delete(j.id);
  }
}

function applyBooks(data: AudiobooksStatus, force = false): void {
  reportFollowed(data);
  const json = JSON.stringify(data);
  if (force || json !== books.lastJson) {
    books.lastJson = json;
    books.data = data;
    renderBooks();
  }
}

async function pollBooks(force = false): Promise<void> {
  clearTimeout(books.timer);
  if (!books.data) {                         // the first load: placeholders, and "Erneut versuchen" if it fails
    await loadList($("#bookList"), async () => applyBooks(await api<AudiobooksStatus>("GET", "/api/audiobooks"), force), { count: 5 });
  } else {
    try { applyBooks(await api<AudiobooksStatus>("GET", "/api/audiobooks"), force); } catch { /* keep the last state, try again */ }
  }
  if (live.ok) return;                       // live updates bring every change
  const busy = books.data?.jobs.some((j) => isActive(j.status)) || !!books.data?.waiting.length;
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
  if (active && active.closest("#bookList") && /^(INPUT|SELECT)$/.test(active.tagName)
    && !(active instanceof HTMLInputElement && ["checkbox", "radio", "button"].includes(active.type))) {
    books.lastJson = "";   // render on the next update instead
    return;
  }
  renderInbox(d);
  if (!live.ok) loadLibraryStats();   // otherwise the live updates bring it
  renderFilter(d.jobs);
  const list = $("#bookList");
  // the library does not depend on the jobs: a live update (a conversion's progress, every few seconds) must not
  // rebuild and reload it – that threw a reader scrolling through it back to the top
  if (books.filter === "library" && list.dataset.view === "library") return;
  const y = window.scrollY;                          // a rebuild (live update) keeps the place on the page
  queueMicrotask(() => { if (window.scrollY !== y) window.scrollTo(0, y); });
  list.replaceChildren();
  list.dataset.view = books.filter;
  if (books.filter === "unconverted") { renderUnconverted(list); return; }
  if (books.filter === "library") { renderLibrary(list); return; }
  const clear = $("#clearBooksBtn");
  clear.hidden = books.filter !== "all" || !d.jobs.some((j) => j.status === "done");
  clear.textContent = books.hideDone ? t("Fertige einblenden") : t("Fertige ausblenden");
  const jobs = d.jobs.filter((j) => books.filter === "all" ? !(books.hideDone && j.status === "done")
    : (books.filter === "active" ? isActive(j.status)
      : books.filter === "failed" ? isTrouble(j) : j.status === books.filter));
  // new in the inbox, not started yet: visible right away, so nobody wonders whether the download arrived
  if (["all", "active"].includes(books.filter)) {
    for (const w of d.waiting) {
      list.append(el("li", { class: "book waiting-card" },
        el("div", { class: "thumb none" }, icon("refresh")),
        el("div", { class: "book-main" },
          el("b", {}, w.name),
          el("span", { class: "muted small" }, w.ready_in > 0
            ? t("Im Eingang – startet in ca. {n} s, sobald der Download fertig kopiert ist", { n: w.ready_in })
            : t("Im Eingang – startet gleich")))));
    }
  }
  if (!jobs.length && !(d.waiting.length && ["all", "active"].includes(books.filter))) {
    list.append(el("li", { class: "empty" }, d.jobs.length ? t("Nichts in dieser Ansicht.")
      : d.inbox_ok ? t("Noch keine Hörbücher. Lege einen Ordner in den Eingang.")
        : t("Der Eingangsordner ist nicht eingebunden. In Docker/Unraid einen Pfad auf /inbox mappen.")));
  }
  // errors that are dealt with: all of them off the list at once (the books and files stay as they are)
  if (books.filter === "failed" && jobs.length > 1) {
    list.append(el("li", { class: "uc-bar" },
      el("span", { class: "muted small" }, plural(jobs.length, t("1 Eintrag"), t("{n} Einträge", { n: jobs.length }))),
      el("button", { type: "button", class: "btn small ghost", title: t("Nimmt alle Einträge aus der Liste – Dateien und Bibliothek bleiben unberührt"),
        onclick: () => { void clearTrouble(jobs); } }, icon("check"), t("Alle als erledigt abhaken"))));
  }
  for (const j of jobs) list.append(bookCard(j));
}

/** All shown errors off the list (the jobs only: files in the inbox and books in the library stay). */
async function clearTrouble(jobs: AudiobookJob[]): Promise<void> {
  if (!(await confirmBox(plural(jobs.length, t("1 Eintrag aus der Liste nehmen? Dateien und Bibliothek bleiben unberührt."),
    t("{n} Einträge aus der Liste nehmen? Dateien und Bibliothek bleiben unberührt.", { n: jobs.length })), t("Abhaken")))) return;
  let done = 0;
  for (const j of jobs) {
    try { await api("DELETE", `/api/audiobooks/${j.id}`); books.open.delete(j.id); done++; } catch { /* a running job cannot be removed: it stays */ }
  }
  toast(plural(done, t("1 Eintrag abgehakt"), t("{n} Einträge abgehakt", { n: done })));
  pollBooks(true);
}

// ------------------------------------------------------------------------- //
// The whole library: browse and search (shows how every book was read)
// ------------------------------------------------------------------------- //
interface Trickle { on: boolean; left: number }
interface LibraryBook {
  path: string; title: string; authors: string[]; names?: string[]; series: string | null;
  series_position: string | null; asin: string | null; format: string; files: number; mtime?: number; cover?: boolean;
  rating?: number; ratings?: number;
}
interface LibraryRatings { running: boolean; done: number; total: number; with_asin: number; rated: number }
interface LibraryCheck {
  running: boolean; done: number; total: number; checked: number; books: number;
  tiles: { key: string; count: number }[];
  autofix: { running: boolean; done: number; total: number; settled: number };
}

/** The sort keys of the library, short, each with the sort word for ascending / descending (a "-" in front reverses).
 *  Ascending is the arrow up: A→Z, oldest first, lowest condition or rating first (what needs cleaning up). */
const LIBRARY_SORTS: { key: string; label: () => string; name?: () => string; admin?: boolean; asc: string; desc: string; first: "asc" | "desc" }[] = [
  { key: "author", label: () => t("Autor"), asc: "author", desc: "-author", first: "asc" },
  { key: "title", label: () => t("Titel"), asc: "title", desc: "-title", first: "asc" },
  { key: "date", label: () => t("Datum"), asc: "old", desc: "new", first: "desc" },
  { key: "health", label: () => t("Zustand"), asc: "health", desc: "-health", first: "asc", admin: true },
  { key: "rating", label: () => "★", name: () => t("Audible-Bewertung"), asc: "rating", desc: "-rating", first: "asc" },
];

/** Which key and direction a stored sort word means. */
function sortState(word: string): { key: string; dir: "asc" | "desc" } {
  const w = word === "rating_best" ? "-rating" : word;
  for (const s of LIBRARY_SORTS) {
    if (w === s.asc) return { key: s.key, dir: "asc" };
    if (w === s.desc) return { key: s.key, dir: "desc" };
  }
  return { key: "author", dir: "asc" };
}

function renderLibrary(list: HTMLElement): void {
  const q = el("input", { type: "search", value: books.libQuery, placeholder: t("Titel, Autor, Serie oder Ordner"),
    "aria-label": t("Bibliothek durchsuchen") });
  const rows = el("ul", { class: "lib-rows" + (books.libView === "regal" ? " regal" : ""), id: "libRows" }, el("li", { class: "empty" }, t("Lade …")));
  q.addEventListener("input", () => {
    books.libQuery = q.value;
    books.libShown = 100;
    clearTimeout(books.libTimer);
    books.libTimer = setTimeout(loadLibraryBooks, 250);
  });
  const sort = el("div", { class: "sort-bar", role: "group", "aria-label": t("Bibliothek sortieren") });
  const drawSort = (): void => {
    const cur = sortState(books.libSort);
    sort.replaceChildren(...LIBRARY_SORTS.filter((s) => !s.admin || IS_ADMIN).map((s) => {
      const on = cur.key === s.key;
      return el("button", { type: "button", class: "chip" + (on ? " on" : ""), "aria-pressed": String(on),
        "aria-label": (s.name || s.label)(),
        title: on ? (cur.dir === "asc" ? t("Aufsteigend – antippen für absteigend") : t("Absteigend – antippen für aufsteigend")) : (s.name || s.label)(),
        onclick: () => {
          const dir = on ? (cur.dir === "asc" ? "desc" : "asc") : s.first;
          books.libSort = dir === "asc" ? s.asc : s.desc;
          store.set("librarySort", books.libSort); books.libShown = 100; drawSort(); loadLibraryBooks();
        } }, s.label(), on ? icon(cur.dir === "asc" ? "arrow-up" : "arrow-down") : null);
    }));
  };
  drawSort();
  books.drawSort = drawSort;
  const view = el("div", { class: "seg seg-buttons view-toggle", role: "group", "aria-label": t("Ansicht") });
  for (const [key, label, ico] of [["regal", t("Regal"), "grid"], ["list", t("Liste"), "list"]]) {
    view.append(el("button", { type: "button", title: label, "aria-label": label, "aria-pressed": String(books.libView === key), onclick: () => {
      books.libView = key; store.set("libraryView", key);
      rows.classList.toggle("regal", key === "regal");
      for (const b of view.querySelectorAll("button")) b.setAttribute("aria-pressed", String(b === view.children[key === "regal" ? 0 : 1]));
    } }, icon(ico)));
  }
  list.append(el("li", { class: "lib-search" }, icon("search"), q, view), el("li", { class: "lib-sort" }, sort),
    ...(IS_ADMIN ? [el("li", { class: "lib-tiles", id: "libTiles" }), el("li", { class: "lib-bar", id: "libBar", hidden: true })] : []), el("li", { class: "lib-wrap" }, rows));
  updateLibBar();
  if (IS_ADMIN) loadCheck();
  loadLibraryBooks();
}

async function loadLibraryBooks(): Promise<void> {
  const target = document.getElementById("libRows");
  if (target && !target.children.length) skeleton(target, 6);
  try {
    const res = await api<{ total: number; books: LibraryBook[] }>("GET",
      libraryUrl(), undefined, { key: "library" });
    if (!books.libQuery) books.libTotal = res.total;
    books.libMatch = res.total;
    updateLibBar();
    const rows = document.getElementById("libRows");
    if (!rows) return;
    const y = window.scrollY;                       // a reload (new books filed) keeps the place in the list
    rows.replaceChildren(...res.books.map(libraryRow));
    if (!res.books.length) rows.append(el("li", { class: "empty" }, books.libQuery ? t("Nichts gefunden.") : t("Die Bibliothek ist leer.")));
    if (res.total > res.books.length) {
      rows.append(el("li", { class: "lib-more" },
        el("span", { class: "muted small" }, t("{shown} von {total}", { shown: res.books.length, total: res.total })),
        el("button", { class: "btn small", onclick: () => { books.libShown += 200; loadLibraryBooks(); } }, t("Mehr anzeigen"))));
    }
    if (window.scrollY !== y) window.scrollTo(0, y);
  } catch (e) { toast(errMsg(e), true); }
}

/** The one "⋯" of a book: everything that can be done with it, in words (instead of four small icons). */
function rowMenu(b: LibraryBook): HTMLElement {
  const m4b = b.format === "m4b";
  const one = [{ path: b.path, title: b.title || b.path }];
  const items: [string, string, () => void][] = [
    [t("Details"), "info", () => showDetail(libraryAsBook(b))],
    ...(m4b ? [
      [t("Kapitel neu benennen …"), "chapters", () => { void previewChapters(one); }] as [string, string, () => void],
      [t("Daten ergänzen …"), "check", () => { void previewEnrich(one); }] as [string, string, () => void],
      [t("Eigenes Cover …"), "image", () => pickCover(b)] as [string, string, () => void],
    ] : []),
    [t("Besser suchen"), "refresh", () => { void upgradeLibraryBook(b); }],
    [t("Neu zuordnen"), "pencil", () => { void correctLibraryBook(b); }],
  ];
  const menu = el("div", { class: "row-menu", role: "menu", hidden: true }, items.map(([label, ico, run]) =>
    el("button", { type: "button", role: "menuitem", class: "row-menu-item", onclick: () => { closeRowMenus(); run(); } }, icon(ico), label)));
  const btn = el("button", { type: "button", class: "btn small ghost more", "aria-haspopup": "menu", "aria-expanded": "false",
    "aria-label": t("Weitere Aktionen"), title: t("Weitere Aktionen"), onclick: (e: Event) => {
      e.stopPropagation();
      const open = menu.hidden;
      closeRowMenus();
      menu.hidden = !open;
      btn.setAttribute("aria-expanded", String(open));
      // open towards the side that has room (the right column of a shelf, a narrow phone)
      menu.classList.remove("flip", "up");
      const r = menu.getBoundingClientRect();
      if (open && r.right > window.innerWidth - 8) menu.classList.add("flip");
      if (open && r.bottom > window.innerHeight - 76) menu.classList.add("up");   // not behind the tab bar
    } }, icon("more"));
  return el("div", { class: "row-menu-wrap" }, btn, menu);
}

function closeRowMenus(): void {
  for (const m of document.querySelectorAll<HTMLElement>(".row-menu")) m.hidden = true;
  for (const b of document.querySelectorAll<HTMLElement>(".row-menu-wrap .more")) b.setAttribute("aria-expanded", "false");
}

function libraryRow(b: LibraryBook): HTMLLIElement {
  const author = (b.authors || [])[0] || "";
  // further names, without the pieces of longer ones ("Kleist" of "Kleist, Heinrich von")
  const all = [...(b.authors || []), ...(b.names || [])];
  const other = (b.names || []).filter((n, i, arr) => arr.indexOf(n) === i && !(b.authors || []).includes(n)
    && !all.some((m) => m !== n && m.includes(n)));
  const tick = IS_ADMIN && b.format === "m4b" ? el("input", { type: "checkbox", checked: books.libPicked.has(b.path),
    "aria-label": t("Auswählen: {title}", { title: b.title || b.path }) }) : null;
  tick?.addEventListener("change", () => { if (tick.checked) books.libPicked.add(b.path); else books.libPicked.delete(b.path); updateLibBar(); });
  const row = el("li", { class: "lib-row", "data-asin": b.asin || "" },
    tick ? el("label", { class: "lib-check" }, tick) : null,
    libraryCover(b.path, b.title || b.path, author, b.format !== "m4b" ? (b.format || "").toUpperCase() : "", b.mtime || "", !!b.cover),
    el("div", { class: "lib-main" },
      el("div", { class: "lib-title" }, el("button", { type: "button", class: "lib-open", title: t("Details"), onclick: () => showDetail(libraryAsBook(b)) }, b.title || b.path),
        b.series ? el("span", { class: "muted" }, ` · ${b.series}${b.series_position ? " " + b.series_position : ""}`) : null),
      el("div", { class: "book-sub" }, author || t("Autor unbekannt"),
 b.rating ? el("span", { class: "rating", title: t("Bewertung bei Audible: {n} Stimmen", { n: b.ratings || 0 }) },
          `★ ${b.rating.toFixed(1).replace(".", ",")} (${b.ratings})`) : null,
        other.length ? el("span", { class: "faint", title: t("Weitere Namen aus Tags und Ordnern – zählen beim Abgleich mit") },
          " · " + other.slice(0, 4).join(" / ")) : null),
      el("code", { class: "lib-path" }, b.path)),
    el("div", { class: "lib-side" },
      b.format === "m4b" ? el("span", { class: "tag" }, "M4B") : el("span", { class: "tag soft" }, plural(b.files, t("1 Datei"), t("{n} Dateien", { n: b.files }))),
      b.asin ? el("span", { class: "muted small" }, b.asin) : null),
    IS_ADMIN ? el("div", { class: "lib-actions" }, rowMenu(b)) : null);
  // the whole row opens the details (the buttons keep their own job)
  row.addEventListener("click", (e) => { if (!(e.target as Element).closest("button, a, label, input")) showDetail(libraryAsBook(b)); });
  return row;
}

/** A library book as the detail column knows books: title, author, series, the cover from its folder – and, with an ASIN, the shop's details. */
function libraryAsBook(b: LibraryBook): Book {
  return {
    asin: b.asin, title: b.title || b.path, authors: b.authors || [], series: b.series, series_position: b.series_position,
    cover_url: b.cover ? `/api/library/cover?path=${encodeURIComponent(b.path)}${b.mtime ? "&v=" + b.mtime : ""}` : null,
    source: b.asin ? "audible" : "library", owned: { path: b.path, title: b.title, authors: b.authors || [], why: "", count: 1 },
  };
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
      ? t("Die Auswahl wird zu M4B mit Kapiteln. Alte Dateien werden erst entfernt, wenn die neue M4B in der Bibliothek liegt.")
      : t("Alle Hörbücher der Bibliothek liegen schon als M4B vor.")),
    settingSelect("ab_library_originals", t("Alte Dateien danach:"), [
      ["archive", t("Nach _ersetzt im Eingang verschieben")], ["delete", t("Löschen")]]),
    el("button", {
      class: "btn small primary", disabled: !books.picked.size,
      onclick: () => convertLibrary([...books.picked].map((path) => ({ path }))),
    }, books.picked.size ? t("{n} zu M4B", { n: books.picked.size }) : t("Zu M4B")));
  list.append(bar);
  // the whole library in the background: one book after the other, one core, lowest priority
  const tr = books.trickle;
  const toggle = el("button", { type: "button", class: "btn small" + (tr?.on ? "" : " primary"), disabled: !tr?.on && tr?.left === 0,
    onclick: async () => {
      try {
        books.trickle = await api<Trickle>("POST", "/api/library/trickle", { on: !tr?.on });
        toast(books.trickle.on ? t("Schonende Umwandlung läuft – ein Hörbuch nach dem anderen") : t("Schonende Umwandlung angehalten"));
        renderBooks();
      } catch (e) { toast(errMsg(e), true); }
    } }, tr?.on ? t("Anhalten") : t("Alle nach und nach umwandeln"));
  list.append(el("li", { class: "uc-bar trickle" },
    el("span", { class: "muted small" }, tr?.on
      ? plural(tr.left, t("Schonende Umwandlung läuft: noch 1 Hörbuch."), t("Schonende Umwandlung läuft: noch {n} Hörbücher.", { n: tr.left }))
        + " " + t("Ein Hörbuch nach dem anderen, mit einem CPU-Kern und niedrigster Priorität; neue Hörbücher und Downloads gehen vor.")
      : t("Schonend im Hintergrund: ein Hörbuch nach dem anderen, mit einem CPU-Kern und niedrigster Priorität – der Server bleibt nutzbar, auch wenn es Tage dauert. Was unklar ist, bleibt unter „Zu prüfen“ liegen.")),
    toggle));
  for (const b of items.slice(0, books.ucShown)) {
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
            plural(b.files, t("1 Datei"), t("{n} Dateien", { n: b.files }))].filter(Boolean).join(" · ")),
          el("div", { class: "book-dest" }, el("code", {}, b.path)))),
      b.job ? null : el("div", { class: "book-tools" },
        el("button", { class: "btn small", onclick: () => openTracks(b) }, t("Kapitel & Namen …")),
        el("button", { class: "btn small ghost", onclick: () => convertLibrary([{ path: b.path }]) }, t("Direkt zu M4B")))));
  }
  if (items.length > books.ucShown) {
    list.append(el("li", { class: "lib-more" },
      el("span", { class: "muted small" }, t("{shown} von {total}", { shown: books.ucShown, total: items.length })),
      el("button", { class: "btn small", onclick: () => { books.ucShown += 200; books.lastJson = ""; renderBooks(); } }, t("Mehr anzeigen"))));
  }
}

interface ConvertItem { path: string; chapter_names?: string[]; rename_files?: boolean }

async function convertLibrary(items: ConvertItem[]): Promise<void> {
  try {
    const res = await api<{ jobs: AudiobookJob[] }>("POST", "/api/library/convert", { items });
    for (const job of res.jobs) follow(job.id, job.name);
    toast(plural(res.jobs.length, t("1 Hörbuch in Arbeit – Fortschritt unter „Laufend“"),
      t("{n} Hörbücher in Arbeit – Fortschritt unter „Laufend“", { n: res.jobs.length })), "info");
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
  const before = JSON.stringify(books.trickle);
  books.trickle = st.trickle || null;
  if (books.filter === "unconverted" && before !== JSON.stringify(books.trickle)) { books.lastJson = ""; renderBooks(); }
  if (books.libTotal !== st.books) {
    books.libTotal = st.books;
    if (books.filter === "library" && !$("#viewBooks").hidden && !st.scanning) loadLibraryBooks();
  }
  $("#libraryInfo").replaceChildren(
    el("div", { class: "kv" }, el("span", {}, t("Hörbücher")),
      el("span", {}, st.scanning ? t("Wird eingelesen …") : `${st.books}` + (st.other ? " " + t("(davon {n} als MP3 & Co.)", { n: st.other }) : "")), el("span", {})),
    el("div", { class: "kv" }, el("span", {}, t("Eingelesen")), el("span", {}, fmtAgo(st.last_scan)), el("span", {})),
    el("div", { class: "muted small" }, t("Neue Funde werden mit der Bibliothek abgeglichen – in der Suche und vor dem Ablegen.")));
}

/** Delete desc.txt/reader.txt where metadata.json has everything (after a preview). */
async function cleanLibrary(): Promise<void> {
  try {
    const pre = await api<{ count: number; folders: number; examples: string[] }>("GET", "/api/library/cleanup");
    if (!pre.count) { toast(t("Nichts aufzuräumen.")); return; }
    const ok = await confirmBox(t("{n} überflüssige Dateien (desc.txt, reader.txt) in {folders} Buchordnern löschen? Hörbücher und Cover bleiben.",
      { n: pre.count, folders: pre.folders }), t("Löschen"));
    if (!ok) return;
    const res = await api<{ removed: number; errors: string[] }>("POST", "/api/library/cleanup");
    toast(plural(res.removed, t("1 Datei gelöscht"), t("{n} Dateien gelöscht", { n: res.removed })) + (res.errors.length ? " · " + t("Fehler: {errors}", { errors: res.errors.length }) : ""), res.errors.length > 0);
  } catch (e) { toast(errMsg(e), true); }
}

/** The inbox in one line under the heading; paths, queues and problems only when they matter. */
function renderInbox(d: AudiobooksStatus): void {
  const box = $("#inboxInfo");
  const parts: (HTMLElement | string)[] = [];
  const bad = (what: string, path: string): HTMLElement =>
    el("span", { class: "err", title: path }, icon("alert"), " ", t("{what} nicht erreichbar: {path}", { what, path: path || "–" }));
  if (!d.inbox_ok) parts.push(bad(t("Eingang"), d.inbox));
  if (!d.library_ok) parts.push(bad(t("Bibliothek"), d.library));
  parts.push(el("span", {}, d.enabled
    ? t("Eingang wird überwacht · zuletzt geprüft {when}", { when: fmtAgo(d.last_scan) })
    : t("Eingang wird nicht überwacht (Einstellungen)")));
  if (d.queues) {
    const waiting = (n: number): string => plural(n, t("1 wartet"), t("{n} warten", { n }));
    const q = (x: QueueStat): string => (x.running ? plural(x.running, t("1 läuft"), t("{n} laufen", { n: x.running })) : "")
      + (x.running && x.waiting ? ", " : "") + (x.waiting ? waiting(x.waiting) : "");
    if (d.queues.encode.running || d.queues.encode.waiting) parts.push(el("span", {}, t("Kodieren: {state}", { state: q(d.queues.encode) })));
    if (d.queues.io.running || d.queues.io.waiting) parts.push(el("span", {}, t("Ablegen: {state}", { state: q(d.queues.io) })));
  }
  if (d.waiting.length) {
    parts.push(el("span", { title: d.waiting.map((w) => w.name).join("\n") },
      t("Wird noch kopiert: {names}", { names: d.waiting.map((w) => w.name).join(", ") })));
  }
  if (d.scan_note) parts.push(el("span", {}, d.scan_note));
  if (d.ignored?.length) {
    const back = el("button", { type: "button", class: "linkish" }, t("Wieder aufnehmen"));
    back.addEventListener("click", async () => {
      try { await api("POST", "/api/inbox/unignore", {}); toast(t("Wird beim nächsten Prüfen wieder aufgenommen")); pollBooks(true); }
      catch (e) { toast(errMsg(e), true); }
    });
    parts.push(el("span", { title: d.ignored.join("\n") },
      plural(d.ignored.length, t("1 Ordner ignoriert"), t("{n} Ordner ignoriert", { n: d.ignored.length })), " · ", back));
  }
  box.replaceChildren(...parts);
}

/** An error to look at: failed, or stopped by hand – not a double that was dropped on purpose. */
function isTrouble(j: AudiobookJob): boolean {
  return j.status === "failed" || (j.status === "cancelled" && !j.duplicate);
}

function renderFilter(jobs: AudiobookJob[]): void {
  // null = not loaded yet: no number (a "0" would claim an empty library)
  const counts: Record<BookFilter, number | null> = {
    all: jobs.length,
    review: jobs.filter((j) => j.status === "review").length,
    active: jobs.filter((j) => isActive(j.status)).length,
    done: jobs.filter((j) => j.status === "done").length,
    failed: jobs.filter(isTrouble).length,
    unconverted: books.unconverted ? books.unconverted.length : null,
    library: books.libTotal ?? null,
  };
  $("#bookFilter").replaceChildren(...BOOK_FILTERS.map(([k, label]) => el("button", {
    class: "chip" + (books.filter === k ? " on" : "") + (k === "review" && counts.review ? " attention" : ""),
    onclick: () => { books.filter = k; store.set("bookFilter", k); if (k === "unconverted") loadUnconverted(); renderBooks(); },
  }, `${label} ${counts[k] ?? ""}`.trim())));
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
    m?.cover_url ? coverFor(m.cover_url, ...jobCoverText(j)) : coverArt(...jobCoverText(j)),
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
      j.status === "done" && j.dest && !(j.message || "").includes(j.dest) ? el("div", { class: "book-dest" }, "→ ", el("code", {}, j.dest)) : null,
      j.status === "failed" && j.dest ? el("div", { class: "book-dest" }, "Originale: ", el("code", {}, j.dest)) : null,
    ),
  );
  li.append(head);

  const tools = el("div", { class: "book-tools" });
  if (isActive(j.status) || j.status === "review") {
    tools.append(el("button", { class: "btn small ghost", onclick: () => bookAction(j, "cancel") }, t("Abbrechen")));
  }
  if (isTrouble(j)) {
    tools.append(el("button", { class: "btn small", onclick: () => bookAction(j, "retry") }, t("Erneut versuchen")));
  }
  if (j.hints?.request_conflict && j.status === "review") {
    // downloaded for a request, but it does not fit: throw it away (deleted, blocked for this book) or keep it
    li.append(el("div", { class: "conflict-box" },
      el("p", {}, icon("alert"), " ", t("Passt nicht zur Anfrage: {why}", { why: j.hints.request_conflict })),
      el("div", { class: "row gap wrap" },
        el("button", { class: "btn small primary", onclick: () => decideConflict(j, "wrong-release") }, t("Verwerfen – löschen und sperren")),
        el("button", { class: "btn small", onclick: () => decideConflict(j, "keep") }, t("Behalten – als das Hörbuch einordnen, das es ist")))));
  } else if (j.hints?.request_id && j.status !== "done" && j.origin === "inbox") {
    // downloaded for a request, but not the book that was asked for: away with it, the request looks further
    tools.append(el("button", {
      class: "btn small ghost danger", title: t("Nicht das angefragte Hörbuch: Import stoppen, Download löschen, die Anfrage sucht weiter"),
      onclick: async () => {
        if (!(await confirmBox(t("„{name}“ ist das falsche Release? Der Import stoppt, der Download wird gelöscht und die Anfrage sucht weiter.", { name: j.name }), t("Falsches Release")))) return;
        try { await api("POST", `/api/audiobooks/${encodeURIComponent(j.id)}/wrong-release`); toast(t("Gelöscht – die Anfrage sucht weiter")); pollBooks(true); }
        catch (e) { toast(errMsg(e), true); }
      },
    }, t("Falsches Release")));
  }
  if (j.status === "done" && j.dest) {
    tools.append(el("button", {
      class: "btn small", title: t("Falsch erkannt? Hörbuch aus der Bibliothek zurückholen und neu zuordnen"),
      onclick: () => reopenBook(j),
    }, t("Korrigieren")),
    el("button", {
      class: "btn small ghost", title: t("Eine andere Aufnahme als gewünscht (anderer Sprecher, Lesung statt Hörspiel …): neu zuordnen, die Anfrage sucht weiter"),
      onclick: () => wrongEdition(j),
    }, t("Falsche Fassung")));
  }
  if (!isActive(j.status)) {
    if (j.origin === "inbox" && j.status !== "done") {
      // still in the inbox: really delete it – or leave it there and never pick it up again
      tools.append(el("button", { class: "btn small ghost danger", onclick: () => deleteFromInbox(j) }, t("Aus Eingang löschen")),
        el("button", { class: "btn small ghost", onclick: () => ignoreInInbox(j) }, t("Ignorieren")));
    } else {
      tools.append(el("button", { class: "btn small ghost", onclick: () => removeBook(j) }, t("Aus Liste entfernen")));
    }
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
      box.append(el("p", { class: "muted" }, t("Keine Treffer. Anders suchen, einen Link einfügen oder selbst eintragen.")));
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

/** Which version sounds better? Clearly more kbit/s (by a quarter or more) – otherwise they count as equal. */
function betterVersion(q: Duplicate["quality"]): "old" | "new" | null {
  if (!q?.old || !q?.new) return null;
  if (q.new.kbps >= q.old.kbps * 1.25) return "new";
  if (q.old.kbps >= q.new.kbps * 1.25) return "old";
  return null;
}

function qualityCard(label: string, q: Quality | null, best: boolean): HTMLElement {
  return el("div", { class: "dup-version" + (best ? " best" : "") },
    el("span", { class: "dup-label" }, label, best ? el("span", { class: "rtag ok" }, icon("check"), t("Besser")) : null),
    q ? el("b", {}, `${q.kbps} kbit/s`) : el("b", { class: "muted" }, "–"),
    q ? el("span", { class: "muted small" }, [fmtDuration(q.duration), fmtSize(q.size),
      plural(q.files, t("1 Datei"), t("{n} Dateien", { n: q.files }))].join(" · ")) : null);
}

function duplicateBox(j: AudiobookJob, d: Duplicate): HTMLDivElement {
  const better = betterVersion(d.quality);
  const fromLibrary = j.origin === "library";     // the gentle conversion of a book that is in the library twice
  const act = async (action: string, label: string): Promise<void> => {
    if (action === "discard" && fromLibrary
      && !(await confirmBox(t("„{path}“ bleibt. Das Doppel „{old}“ wird entfernt – wie unter „Alte Dateien danach“ eingestellt nach _ersetzt verschoben oder gelöscht.", { old: j.rel, path: d.path }),
        t("Doppel entfernen")))) return;
    if (action === "replace") {
      const worse = better === "old" ? " " + t("Achtung: Die neue Fassung klingt deutlich schlechter ({new} statt {old} kbit/s).",
        { new: d.quality?.new?.kbps ?? "?", old: d.quality?.old?.kbps ?? "?" }) : "";
      if (!(await confirmBox(t("Vorhandene Fassung „{path}“ durch die neue ersetzen? Sie wird nach _ersetzt im Eingang verschoben.", { path: d.path }) + worse,
        t("Ersetzen")))) return;
    }
    try { await api("POST", `/api/audiobooks/${j.id}/duplicate`, { action }); toast(label); pollBooks(true); }
    catch (e) { toast(errMsg(e), true); }
  };
  // the better version is the suggestion: keep the old one (discard the new) – or replace it
  const discard = el("button", { class: "btn small" + (better !== "new" ? " primary" : ""), onclick: () => act("discard", t("Verworfen")) },
    fromLibrary ? t("Neues nicht übernehmen") : better === "old" ? t("Vorhandene behalten") : t("Verwerfen"));
  const replace = el("button", { class: "btn small" + (better === "new" ? " primary" : ""), onclick: () => act("replace", t("Wird ersetzt")) },
    better === "new" ? t("Durch die bessere ersetzen") : t("Vorhandenes ersetzen"));
  const keep = el("button", { class: "btn small ghost", onclick: () => act("keep", t("Wird zusätzlich abgelegt")) }, t("Trotzdem zusätzlich ablegen"));
  // probably another work under the same short title: filing it as a book of its own is the suggestion
  const own = d.own_title ? el("button", { class: "btn small primary", onclick: () => act("keep_named", t("Wird als eigenes Hörbuch abgelegt")) },
    t("Als „{title}“ ablegen", { title: d.own_title })) : null;
  if (d.other_work) { discard.classList.remove("primary"); replace.classList.remove("primary"); if (!own) keep.classList.replace("ghost", "primary"); }
  // what exactly is in the library – so nobody replaces a book without knowing which one it is
  const existing: LibraryBook = { path: d.path, title: d.title, authors: d.authors || [], series: d.series ?? null,
    series_position: d.series_position ?? null, asin: d.asin, format: d.format, files: d.quality?.old?.files || 0, cover: !!d.cover };
  const narrators = d.narrators || [];
  const buttons = d.other_work ? [own || keep, discard, replace, own ? keep : null] : [...(better === "new" ? [replace, discard] : [discard, replace]), keep];
  return el("div", { class: "dup-box" },
    el("div", {}, el("b", {}, t("Schon in der Bibliothek:"), " "),
      el("span", { class: "muted" }, `${DUP_WHY[d.why] || t("ähnlich")}${d.format && d.format !== "m4b" ? ", " + t("dort als {format}", { format: d.format.toUpperCase() }) : ""}`)),
    el("div", { class: "dup-existing" },
      libraryCover(d.path, d.title || d.path, (d.authors || [])[0], (d.format || "").toUpperCase(), "", !!d.cover),
      el("div", { class: "dup-existing-main" },
        el("b", {}, d.title || d.path),
        el("span", { class: "muted small" }, [(d.authors || []).join(", "),
          narrators.length ? t("gelesen von {names}", { names: narrators.slice(0, 3).join(", ") }) : "",
          d.series ? `${d.series}${d.series_position ? " " + d.series_position : ""}` : ""].filter(Boolean).join(" · ")),
        el("code", {}, d.path),
        d.origin ? el("span", { class: "muted small" }, t("Abgelegt aus: {name}", { name: d.origin })) : null,
        el("button", { type: "button", class: "btn small ghost", onclick: () => showDetail(libraryAsBook(existing)) }, t("Ansehen")))),
    // the book came from the library itself: the second copy there is where the new M4B was made from
    fromLibrary ? el("div", { class: "muted small" }, t("Doppel in der Bibliothek:"), " ", el("code", {}, j.rel),
      " – " + t("daraus wurde die neue M4B umgewandelt")) : null,
    d.other_work ? el("div", { class: "dup-other" }, icon("alert"), t("Wahrscheinlich ein anderes Werk: {why}", { why: d.other_work })) : null,
    d.quality ? el("div", { class: "dup-compare" },
      qualityCard(fromLibrary ? t("Vorhanden") : t("In der Bibliothek"), d.quality.old, better === "old"),
      qualityCard(fromLibrary ? t("Neu aus dem Doppel") : t("Neu"), d.quality.new, better === "new")) : null,
    el("div", { class: "row gap wrap" }, ...buttons),
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
    coverFor(c.cover_url, c.title, (c.authors || [])[0]),
    el("div", { class: "cand-main" },
      el("div", { class: "cand-title" }, c.title, c.subtitle ? el("span", { class: "muted" }, " – " + c.subtitle) : null),
      el("div", { class: "cand-sub" }, [
        (c.authors || []).join(", "),
        narrators.length ? t("Sprecher: {names}", { names: narrators.slice(0, 3).join(", ") + (narrators.length > 3 ? " …" : "") }) : "",
      ].filter(Boolean).join(" · ")),
      c.mismatch ? el("div", { class: "cand-warn" }, icon("alert"), t("Passt vermutlich nicht: {why}", { why: c.mismatch })) : null,
      el("div", { class: "cand-meta" },
        langTag(c),
        c.request_id ? el("span", { class: "rtag req-pending", title: t("Dieses Hörbuch wurde angefragt – Übernehmen erfüllt die Anfrage") }, t("Anfrage von {name}", { name: c.request_user || "–" })) : null,
        c.series ? el("span", { class: "tag" }, `${c.series}${c.series_position ? " #" + c.series_position : ""}`) : null,
        c.format_type && c.format_type !== "unabridged" ? el("span", { class: "tag soft" }, c.format_type === "abridged" ? t("Gekürzt") : c.format_type) : null,
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
        el("button", { class: best && !c.mismatch ? "btn small primary" : "btn small accent", onclick: () => chooseBook(j, c) }, t("Übernehmen")))));
}

async function decideConflict(j: AudiobookJob, what: "wrong-release" | "keep"): Promise<void> {
  try {
    await api("POST", `/api/audiobooks/${encodeURIComponent(j.id)}/${what}`);
    toast(what === "keep" ? t("Behalten – die Anfrage sucht weiter") : t("Gelöscht – die Anfrage sucht weiter"));
    pollBooks(true);
  } catch (e) { toast(errMsg(e), true); }
}

async function chooseBook(j: AudiobookJob, c: Book, keep = false): Promise<void> {
  try {
    await api("POST", `/api/audiobooks/${j.id}/choose`, { id: c.id || c.asin, asin: c.asin, region: c.region, keep });
    follow(j.id, c.title);
    toast(t("„{title}“ wird abgelegt", { title: c.title }), "info");
    pollBooks(true);
  } catch (e) {
    if (!(e instanceof ApiError && e.status === 409)) { toast(errMsg(e), true); return; }
    // downloaded for a request, but it is another book: normally it goes and the request looks further
    if (await confirmBox(`${errMsg(e)} ${t("Download löschen und die Anfrage weitersuchen lassen?")}`, t("Löschen und weitersuchen"))) {
      try { await api("POST", `/api/audiobooks/${encodeURIComponent(j.id)}/wrong-release`); toast(t("Gelöscht – die Anfrage sucht weiter")); pollBooks(true); }
      catch (e2) { toast(errMsg(e2), true); }
    } else if (await confirmBox(t("Trotzdem als „{title}“ in die Bibliothek legen?", { title: c.title }), t("Trotzdem ablegen"))) {
      await chooseBook(j, c, true);
    }
  }
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
    ...(c.cover_url ? [coverFor(c.cover_url, c.title, (c.authors || [])[0])] : []),
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
    follow(d.dataset.jobId, String(data.title || ""));
    toast(t("Wird mit deinen Angaben abgelegt"), "info");
    pollBooks(true);
  } catch (e) { toast(errMsg(e), true); }
}

async function reopenBook(j: AudiobookJob): Promise<void> {
  if (!(await confirmBox(t("Hörbuch aus der Bibliothek zurückholen und neu zuordnen? Die M4B kommt zurück unter „Zu prüfen“, desc.txt, reader.txt und cover.jpg werden entfernt."), t("Korrigieren")))) return;
  try {
    await api("POST", `/api/audiobooks/${j.id}/reopen`);
    books.filter = "review"; store.set("bookFilter", "review");
    pollBooks(true);
  } catch (e) { toast(errMsg(e), true); }
}

async function wrongEdition(j: AudiobookJob): Promise<void> {
  if (!(await confirmBox(t("„{name}“ ist eine andere Fassung als gewünscht? Das Hörbuch kommt zurück unter „Zu prüfen“ und wird dort richtig zugeordnet. Gehört es zu einer Anfrage, wird dieses Release gesperrt und die Anfrage sucht weiter.", { name: j.match?.title || j.name }), t("Falsche Fassung")))) return;
  try {
    await api("POST", `/api/audiobooks/${j.id}/wrong-edition`);
    books.filter = "review"; store.set("bookFilter", "review");
    pollBooks(true);
  } catch (e) { toast(errMsg(e), true); }
}

/** Look again for releases of a book that is there: a better one (or the one picked by hand) replaces it. */
async function upgradeLibraryBook(b: LibraryBook): Promise<void> {
  if (!(await confirmBox(t("Für „{title}“ erneut nach Releases suchen? Automatisch wird nur ein klar besseres Release geladen (besseres Format oder deutlich mehr Bitrate); unter „Anfragen“ kannst du auch selbst eins wählen. Das neue ersetzt dann dieses Hörbuch, die alte Datei wird archiviert.", { title: b.title || b.path }), t("Suchen")))) return;
  try {
    await api("POST", "/api/library/upgrade", { path: b.path });
    toast(t("Sucht nach einem besseren Release – zu sehen unter „Anfragen“"));
    loadRequests();
  } catch (e) { toast(errMsg(e), true); }
}

/** The tidy-up tasks as tiles: a number and a word each; a tap shows only those books (a second tap: all again). */
const TILES: { key: string; label: () => string; hint: () => string }[] = [
  { key: "chapters", label: () => t("Kapitel"), hint: () => t("Kapitelnamen wiederholen den Titel, sagen nichts oder fehlen") },
  { key: "details", label: () => t("Angaben"), hint: () => t("Was Hörwerk nicht selbst ergänzen konnte: Cover, Jahr oder Sprecher fehlen") },
  { key: "match", label: () => t("Zuordnung"), hint: () => t("Was Hörwerk nicht sicher zuordnen konnte (keine Audible-Verknüpfung) oder eine andere Fassung (Laufzeit weicht ab)") },
  { key: "lowrated", label: () => t("Schlecht bewertet"), hint: () => t("Audible-Bewertung unter 3,4 bei mindestens 20 Stimmen") },
];

/** The check and Audible's data run by themselves in the background; this only shows how far they are. */
async function loadCheck(): Promise<void> {
  try {
    books.check = await api<LibraryCheck>("GET", "/api/library/check", undefined, { key: "libcheck" });
    books.ratings = await api<LibraryRatings>("GET", "/api/library/ratings", undefined, { key: "libratings" });
    renderTiles();
    clearTimeout(books.checkTimer);
    const going = (): boolean => !!books.check?.running || !!books.ratings?.running || !!books.check?.autofix.running;
    if (going()) {
      books.checkTimer = setTimeout(async () => {
        await loadCheck();
        loadLibraryBooks();                                      // the numbers (and the books) change as it goes
      }, going() && books.check?.autofix.running ? 8000 : 4000);
    }
  } catch (e) { toast(errMsg(e), true); }
}

function renderTiles(): void {
  const box = document.getElementById("libTiles");
  const c = books.check;
  if (!box || !c) return;
  const busy = c.running || !!books.ratings?.running || c.autofix.running;
  const status = c.running ? t("Prüft {done}/{total}", { done: c.done, total: c.total })
    : c.autofix.running ? t("Ordnet zu {done}/{total}", { done: c.autofix.done, total: c.autofix.total })
    : books.ratings?.running ? t("Audible {done}/{total}", { done: books.ratings.done, total: books.ratings.total }) : "";
  // one button: look at everything again (check and Audible's data); while it runs it stops
  const again = el("button", { type: "button", class: "icon-btn", "aria-label": busy ? t("Anhalten") : t("Neu prüfen"),
    title: busy ? t("Anhalten") : t("Alles neu prüfen und Audible-Daten aktualisieren"), onclick: async () => {
    try {
      const on = !busy;
      books.check = await api<LibraryCheck>("POST", "/api/library/check", { on });
      books.ratings = await api<LibraryRatings>("POST", "/api/library/ratings", { on });
      if (on) toast(t("Läuft im Hintergrund – ein Buch nach dem anderen"), "info");
      loadCheck();
    } catch (e) { toast(errMsg(e), true); }
  } }, icon(busy ? "x" : "refresh"));
  const tiles = TILES.map((tile) => {
    const n = c.tiles.find((x) => x.key === tile.key)?.count ?? 0;
    const on = books.libIssue === tile.key;
    return el("button", { type: "button", class: "tile" + (on ? " on" : "") + (n || on ? "" : " none"), "aria-pressed": String(on), title: tile.hint(),
      onclick: () => {
        books.libIssue = on ? "" : tile.key;
        // badly rated: the worst first
        if (!on && tile.key === "lowrated") { books.libSort = "rating"; store.set("librarySort", books.libSort); books.drawSort?.(); }
        books.libShown = 100;
        renderTiles();
        loadLibraryBooks();
      } }, el("b", { class: "n" }, String(n)), el("span", { class: "l" }, tile.label()));
  });
  box.replaceChildren(
    el("div", { class: "tiles-head" }, el("span", { class: "tiles-title" }, t("Zum Aufräumen")),
      el("span", { class: "muted small" }, status), again),
    el("div", { class: "tiles" }, tiles));
}

/** The bar over the library: how many books are ticked, and the way to the chapter preview. */
function updateLibBar(): void {
  const bar = document.getElementById("libBar");
  if (!bar) return;
  const n = books.libPicked.size;
  const items = (): { path: string; title: string }[] => [...books.libPicked].map((path) => ({ path, title: path.split("/").pop() || path }));
  const tick = (on: boolean): void => {
    for (const cb of document.querySelectorAll<HTMLInputElement>("#libRows .lib-check input")) cb.checked = on;
  };
  // everything that matches the search and the filter – not only the books loaded so far
  const all = el("button", { type: "button", class: "btn small ghost", onclick: async () => {
    if (!(await confirmBox(t("Alle {n} Treffer auswählen? Damit lassen sich danach alle auf einmal löschen.", { n: books.libMatch }), t("Alle auswählen")))) return;
    try {
      const res = await api<{ paths: string[] }>("GET", libraryUrl({ ids: true }));
      for (const p of res.paths) books.libPicked.add(p);
      tick(true);
      updateLibBar();
    } catch (e) { toast(errMsg(e), true); }
  } }, icon("check"), t("Alle ({n})", { n: books.libMatch }));
  bar.hidden = !n && !books.libMatch;
  if (!n) { bar.replaceChildren(all); return; }
  bar.replaceChildren(
    el("span", { class: "sel-n" }, String(n)),
    el("button", { type: "button", class: "btn small primary", title: t("Kapitelnamen neu benennen – erst eine Vorschau"), onclick: () => previewChapters(items()) },
      icon("chapters"), t("Kapitel")),
    el("button", { type: "button", class: "btn small", title: t("Fehlendes (Jahr, Sprecher, Cover …) ergänzen, wo die Fassung sicher ist"),
      onclick: () => previewEnrich(items()) }, icon("check"), t("Ergänzen")),
    el("button", { type: "button", class: "btn small ghost danger", title: t("Ausgewählte Hörbücher löschen"), onclick: () => deleteSelected() },
      icon("x"), t("Löschen")),
    ...(n < books.libMatch ? [all] : []),
    el("button", { type: "button", class: "btn small ghost", title: t("Auswahl aufheben"), "aria-label": t("Auswahl aufheben"),
      onclick: () => { books.libPicked.clear(); tick(false); updateLibBar(); } }, icon("x")));
}

/** The books of the library as the list asks for them (search, filter, order) – `ids` only the paths of all of them. */
function libraryUrl(o: { ids?: boolean } = {}): string {
  const q = `q=${encodeURIComponent(books.libQuery)}&sort=${books.libSort}&issue=${encodeURIComponent(books.libIssue)}`;
  return o.ids ? `/api/library/books?${q}&ids=1` : `/api/library/books?${q}&limit=${books.libShown}`;
}

/** Delete the ticked books (moved to `_ersetzt` or deleted, as set for replaced originals) – after asking once. */
async function deleteSelected(): Promise<void> {
  const paths = [...books.libPicked];
  if (!paths.length) return;
  if (!(await confirmBox(plural(paths.length,
    t("1 Hörbuch aus der Bibliothek löschen? Je nach Einstellung landet es unter _ersetzt im Eingang oder wird gelöscht."),
    t("{n} Hörbücher aus der Bibliothek löschen? Je nach Einstellung landen sie unter _ersetzt im Eingang oder werden gelöscht.", { n: paths.length })),
    t("Löschen")))) return;
  try {
    const res = await api<{ deleted: number; failed: { path: string; error: string }[] }>("POST", "/api/library/delete", { paths });
    toast(plural(res.deleted, t("1 Hörbuch gelöscht"), t("{n} Hörbücher gelöscht", { n: res.deleted })), res.failed.length ? true : undefined);
    if (res.failed.length) toast(`${res.failed[0].path}: ${res.failed[0].error}`, true);
    books.libPicked.clear();
    updateLibBar();
    loadLibraryBooks();
    loadCheck();
  } catch (e) { toast(errMsg(e), true); }
}

/** Choose a picture for a book that has none (or a wrong one): it becomes the cover in its folder and inside the M4B. */
function pickCover(b: LibraryBook): void {
  const input = el("input", { type: "file", accept: "image/jpeg,image/png" });
  input.addEventListener("change", async () => {
    const file = input.files?.[0];
    if (!file) return;
    if (!/^image\/(jpeg|png)$/.test(file.type)) { toast(t("Das ist kein JPEG- oder PNG-Bild"), true); return; }
    try {
      const res = await fetch(`/api/library/cover?path=${encodeURIComponent(b.path)}`, {
        method: "PUT", headers: { "X-Requested-With": "Hoerwerk", "Content-Type": "application/octet-stream" }, body: file });
      if (!res.ok) throw new Error((await res.json().catch(() => ({}))).error || `HTTP ${res.status}`);
      toast(t("Cover gespeichert: {title}", { title: b.title || b.path }));
      loadLibraryBooks();
    } catch (e) { toast(errMsg(e), true); }
  });
  input.click();
}

interface EnrichChange { field: string; label: string; to: string; image?: boolean }
interface EnrichPlan { status: string; match?: { title: string; source: string }; changes: EnrichChange[] }

/** What Hörwerk can add to the chosen books where it is sure about the edition – and one click to add it to all. */
async function previewEnrich(items: { path: string; title: string }[]): Promise<void> {
  const d = $<HTMLDialogElement>("#enrichDialog");
  const body = $("#enrichBody");
  const apply = $<HTMLButtonElement>("#enrichApply");
  apply.disabled = true;
  body.replaceChildren(el("p", { class: "muted" }, t("Suche die richtige Fassung …")));
  if (!d.open) d.showModal();
  let cancelled = false;
  d.addEventListener("close", () => { cancelled = true; }, { once: true });
  const todo: string[] = [];
  const sections: HTMLElement[] = [];
  for (const it of items) {
    if (cancelled) return;
    const head = el("h3", {}, it.title);
    let list: HTMLElement | null = null;
    try {
      const plan = await api<EnrichPlan>("POST", "/api/library/enrich", { path: it.path, apply: false });
      if (cancelled) return;
      const note: Record<string, string> = {
        complete: t("Nichts zu ergänzen"), unsure: t("Nicht sicher genug – über „Besser“ oder „Korrigieren“ prüfen"),
        no_m4b: t("Noch keine M4B") };
      if (plan.changes.length) {
        head.append(" ", el("span", { class: "muted small" }, t("Fassung: {title}", { title: plan.match?.title || "" })));
        list = el("ul", { class: "enrich-list" }, plan.changes.map((c) => el("li", {},
          c.image ? el("img", { class: "enrich-cover", alt: "", src: `/api/cover?u=${encodeURIComponent(c.to)}&w=300` }) : null,
          el("span", { class: "enrich-label" }, c.label),
          c.image ? el("span", { class: "plus" }, t("Cover aus dem Shop")) : el("span", { class: "plus" }, c.to))));
        todo.push(it.path);
      } else head.append(" ", el("span", { class: "muted small" }, note[plan.status] || ""));
    } catch (e) { head.append(" ", el("span", { class: "muted small" }, errMsg(e))); }
    sections.push(el("section", { class: "chap-book" }, head, list));
  }
  body.replaceChildren(...sections);
  apply.disabled = !todo.length;
  apply.onclick = async () => {
    apply.disabled = true;
    let added = 0;
    for (const [i, path] of todo.entries()) {
      apply.textContent = t("Ergänze … ({n} von {total})", { n: i + 1, total: todo.length });
      try { added += (await api<{ added: number }>("POST", "/api/library/enrich", { path })).added; } catch (e) { toast(errMsg(e), true); }
    }
    apply.textContent = t("Ergänzen");
    toast(t("{n} Angaben ergänzt", { n: added }));
    for (const p of todo) books.libPicked.delete(p);
    d.close();
    updateLibBar();
    loadLibraryBooks();
  };
}

interface ChapterRow { from: string; to: string; why: string }
interface ChapterPlan { changed: number; total: number; rows: ChapterRow[] }

/** Chapter names of the chosen M4B books by the rules for new books: first "now → new" with the reason, then the
 *  user decides – nothing is written before "Umbenennung anwenden". */
async function previewChapters(items: { path: string; title: string }[]): Promise<void> {
  const d = $<HTMLDialogElement>("#chaptersDialog");
  const body = $("#chaptersBody");
  const apply = $<HTMLButtonElement>("#chaptersApply");
  apply.disabled = true;
  body.replaceChildren(el("p", { class: "muted" }, t("Lade Vorschau …")));
  if (!d.open) d.showModal();
  let cancelled = false;
  d.addEventListener("close", () => { cancelled = true; }, { once: true });
  const todo: { path: string; changed: number }[] = [];
  const sections: HTMLElement[] = [];
  for (const it of items) {
    if (cancelled) return;
    const head = el("h3", {}, it.title);
    try {
      const plan = await api<ChapterPlan>("POST", "/api/library/chapters", { path: it.path, apply: false });
      if (cancelled) return;
      head.append(" ", el("span", { class: "muted small" }, plan.changed
        ? t("{changed} von {total} Kapiteln werden geändert", { changed: plan.changed, total: plan.total })
        : t("schon sauber")));
      // what was above, what it will be below: the same on a phone and on a wide screen
      const change = (r: ChapterRow): HTMLElement => el("li", { class: "chap-change" },
        el("div", { class: "was" }, r.from), el("div", { class: "will" }, r.to));
      const kept = (r: ChapterRow): HTMLElement => el("li", { class: "chap-kept" }, r.from);
      const changedRows = plan.rows.filter((r) => r.from !== r.to);
      const keptRows = plan.rows.filter((r) => r.from === r.to);
      sections.push(el("section", { class: "chap-book" }, head,
        changedRows.length ? el("ul", { class: "chap-rows" }, changedRows.map(change)) : null,
        keptRows.length ? el("details", {}, el("summary", { class: "muted small" }, t("{n} bleiben unverändert", { n: keptRows.length })),
          el("ul", { class: "chap-rows" }, keptRows.map(kept))) : null));
      if (plan.changed) todo.push({ path: it.path, changed: plan.changed });
    } catch (e) {
      head.append(" ", el("span", { class: "muted small" }, errMsg(e)));
      sections.push(el("section", { class: "chap-book" }, head));
    }
  }
  body.replaceChildren(...sections);
  apply.disabled = !todo.length;
  apply.onclick = async () => {
    apply.disabled = true;
    let changed = 0;
    for (const [i, x] of todo.entries()) {
      apply.textContent = t("Benenne um … ({n} von {total})", { n: i + 1, total: todo.length });
      try {
        const r = await api<{ changed: number }>("POST", "/api/library/chapters", { path: x.path });
        changed += r.changed;
      } catch (e) { toast(errMsg(e), true); }
    }
    apply.textContent = t("Umbenennung anwenden");
    toast(t("{n} Kapitelnamen umbenannt", { n: changed }));
    for (const x of todo) books.libPicked.delete(x.path);
    d.close();
    updateLibBar();
    loadLibraryBooks();
  };
}

/** Any book of the library back for a correction – also when its import is long gone from the list. */
async function correctLibraryBook(b: LibraryBook): Promise<void> {
  if (!(await confirmBox(t("„{title}“ neu zuordnen? Das Hörbuch wird neu erkannt und wartet unter „Zu prüfen“ auf deine Wahl. Die alte Ablage bleibt, bis das korrigierte Hörbuch abgelegt ist.", { title: b.title || b.path }), t("Korrigieren")))) return;
  try {
    await api("POST", "/api/library/correct", { path: b.path });
    books.filter = "review"; store.set("bookFilter", "review");
    pollBooks(true);
  } catch (e) { toast(errMsg(e), true); }
}

async function bookAction(j: AudiobookJob, action: "cancel" | "retry"): Promise<void> {
  try {
    await api("POST", `/api/audiobooks/${j.id}/${action}`);
    if (action === "retry") {
      follow(j.id, j.name);
      toast(t("Wieder eingereiht: {name} – startet, sobald ein Platz frei ist", { name: j.name }), "info");
    } else toast(t("Wird abgebrochen: {name}", { name: j.name }), "info");
    pollBooks(true);
  } catch (e) { toast(errMsg(e), true); }
}

async function removeBook(j: AudiobookJob): Promise<void> {
  try {
    await api("DELETE", `/api/audiobooks/${j.id}`);
    books.open.delete(j.id);
    toast(t("Aus der Liste entfernt: {name}", { name: j.name }));
    pollBooks(true);
  } catch (e) { toast(errMsg(e), true); }
}

/** Delete the download itself (folder and files) from the inbox – it can't come back. */
async function deleteFromInbox(j: AudiobookJob): Promise<void> {
  if (!(await confirmBox(t("„{name}“ endgültig aus dem Eingang löschen? Die Dateien sind danach weg.", { name: j.name }), t("Löschen")))) return;
  try {
    await api("POST", `/api/audiobooks/${j.id}/delete-source`);
    books.open.delete(j.id);
    toast(t("Gelöscht: {name}", { name: j.name }));
    pollBooks(true);
  } catch (e) { toast(errMsg(e), true); }
}

/** Leave the download in the inbox, but Hörwerk won't pick it up again. */
async function ignoreInInbox(j: AudiobookJob): Promise<void> {
  try {
    await api("POST", `/api/audiobooks/${j.id}/ignore`);
    books.open.delete(j.id);
    toast(t("Wird ab jetzt ignoriert: {name}", { name: j.name }));
    pollBooks(true);
  } catch (e) { toast(errMsg(e), true); }
}

let pathTimer: number | undefined;
/** Settings page "Eingang & Bibliothek": the docker paths and a preview of the folder template. */
function showLibrarySettings(): void {
  updatePathPreview();
  loadLibraryStats();
  const d = books.data;
  $("#abPaths").textContent = d ? t("Eingang: {inbox} · Bibliothek: {library} (über Docker-Pfade einstellen)", { inbox: d.inbox || "–", library: d.library || "–" }) : "";
}

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
  $("#chaptersCancel").addEventListener("click", () => $<HTMLDialogElement>("#chaptersDialog").close());
  $("#enrichCancel").addEventListener("click", () => $<HTMLDialogElement>("#enrichDialog").close());
  if (!IS_ADMIN) return;   // family accounts: search, discover, requests
  document.addEventListener("click", closeRowMenus);
  document.addEventListener("keydown", (e) => { if (e.key === "Escape") closeRowMenus(); });
  $("#scanBtn").addEventListener("click", async () => {
    try { await api("POST", "/api/audiobooks/scan"); toast(t("Eingang wird geprüft")); setTimeout(() => pollBooks(true), 800); }
    catch (e) { toast(errMsg(e), true); }
  });
  for (const btn of document.querySelectorAll<HTMLElement>("[data-preset]")) btn.addEventListener("click", () => fillTracks(btn.dataset.preset || ""));
  $("#tracksGo").addEventListener("click", convertTracks);
  $("#libraryScanBtn").addEventListener("click", async () => {
    try {
      await api("POST", "/api/library/scan"); toast(t("Bibliothek wird eingelesen"));
      setTimeout(loadLibraryStats, 600);
      if (!live.ok) setTimeout(() => { loadLibraryStats(); loadUnconverted(); }, 2500);
    }
    catch (e) { toast(errMsg(e), true); }
  });
  $("#libraryCleanBtn").addEventListener("click", cleanLibrary);
  $("#clearBooksBtn").addEventListener("click", () => {
    books.hideDone = !books.hideDone;                 // only a view: "Fertig" still lists them, a second click brings them back
    store.set("booksHideDone", books.hideDone);
    books.lastJson = "";
    renderBooks();
  });
  $("#manualLinkBtn").addEventListener("click", fetchLinkIntoManual);
  $("#manualLink").addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); fetchLinkIntoManual(); } });
  const manualDialog = $<HTMLDialogElement>("#manualDialog");
  manualDialog.addEventListener("close", () => { if (manualDialog.returnValue === "save") saveManual(); });
  const f = $<HTMLFormElement>("#settingsForm");
  field(f, "ab_folder_template").addEventListener("input", updatePathPreview);
  field(f, "ab_file_template").addEventListener("input", updatePathPreview);
  loadUnconverted();
}

initBooks();
