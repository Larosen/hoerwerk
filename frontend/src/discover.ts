"use strict";

// ------------------------------------------------------------------------- //
// Entdecken: suggested audiobooks – for the library, by genre, from reviews
// ------------------------------------------------------------------------- //
interface Suggestion extends Book {
  requested?: string | null;
  rating?: number; ratings?: number; because?: string[]; loose?: boolean;
  review?: { source: string; link: string; date: string; notes?: number; month?: boolean; list?: string; rank?: number };
  later?: boolean; announced?: string | null;
}
interface DiscoverAnswer { items: Suggestion[]; updated: number | null; error: string | null; running: boolean; personal?: boolean; linked?: boolean; waiting?: number }
interface SubGenre { id: string; name: string }
interface Genre extends SubGenre { nonfiction: boolean; children: SubGenre[] }
const discover = {
  section: "foryou",
  genre: "",
  genres: [] as Genre[],
  nonfiction: new Set<string>(),    // ids of non-fiction genres and their subcategories
  timer: undefined as number | undefined,
  length: "",                       // "", short, mid, long
  year: "",                         // "", 2, 5, old
  last: null as DiscoverAnswer | null,
  panel: false,                     // the genre panel is open
};

const DISCOVER_TITLES: Record<string, () => string> = {
  foryou: () => t("Für dich"),
  genre: () => t("Nach Genre"),
  critics: () => t("Aus der Kritik"),
  follows: () => t("Gefolgt"),
};

function fmtCount(n: number): string {
  return new Intl.NumberFormat(LANG).format(n);
}

/** "Hörspiel" / "gekürzt" – what tells six versions of the same title apart. */
function productionTag(b: Suggestion): HTMLElement | null {
  const text = `${b.title} ${(b as { subtitle?: string | null }).subtitle || ""} ${b.series || ""}`;
  if (/h(?:ö|oe)rspiel|full[\s-]?cast|volle[rn]?\s+besetzung|dramati[sz]/i.test(text)) {
    return el("span", { class: "tag", title: t("Mit Sprecherensemble und Geräuschen, nicht von einer Person gelesen") }, t("Hörspiel"));
  }
  return (b.format_type || "").toLowerCase() === "abridged" ? el("span", { class: "tag", title: t("Gekürzte Fassung") }, t("gekürzt")) : null;
}

/** "Nicht interessiert": the book is not suggested to this account again (kept with the account, not the browser). */
async function hideSuggestion(b: Suggestion): Promise<void> {
  if (!b.asin || typeof ME === "undefined" || !ME) return;
  const list = [...(ME.hidden || []), b.asin.toUpperCase()];
  try {
    const res = await api<{ hidden: string[] }>("PUT", "/api/auth/prefs", { hidden: list });
    ME.hidden = res.hidden;
    if (discover.last) renderDiscover(discover.last);
    toast(t("Wird dir nicht mehr vorgeschlagen"));
  } catch (e) { toast(errMsg(e), true); }
}

function suggestionCard(b: Suggestion): HTMLLIElement {
  const author = (b.authors || [])[0] || "";
  const facts = [
    b.series ? `${b.series} ${b.series_position || ""}`.trim() : "",
    b.runtime_min ? fmtDuration(b.runtime_min * 60) : "",
    (b.release_date || "").slice(0, 4),
    b.source && !["audible", "review"].includes(b.source) ? t("via {source}", { source: SOURCE_NAMES[b.source] || b.source }) : "",
  ].filter(Boolean).join(" · ");
  let reason: Node | null = null;
  if (b.because?.length) {
    reason = el("p", { class: "reason" }, t("Passt zu: {titles}", { titles: b.because.map((x) => t("„{title}“", { title: x })).join(", ") }));
  } else if (b.review) {
    reason = el("p", { class: "reason" },
      b.review.list ? t("Platz {rank}: {list} ({source})", { rank: b.review.rank || "?", list: b.review.list, source: b.review.source })
      : b.review.month ? t("Hörbuch des Monats bei {source}", { source: b.review.source }) : t("Besprochen bei {source}", { source: b.review.source }),
      b.review.notes ? " · " + plural(b.review.notes, t("1 Kritik in der Presse"), t("{n} Kritiken in der Presse", { n: b.review.notes })) : null,
      b.review.link ? el("span", {}, " · ", el("a", { href: b.review.link, target: "_blank", rel: "noopener noreferrer" }, b.review.list ? t("Liste ansehen") : t("Kritik lesen"))) : null);
  }
  const card = el("li", { class: "suggestion tint-card", "data-asin": b.asin || "" },
    coverFor(b.cover_url, b.title, author, "cover"),
    el("div", { class: "s-body" },
      el("button", { type: "button", class: "s-title", title: t("Details"), onclick: () => showDetail(b) }, b.title),
      el("span", { class: "s-author" }, ...personLinks(b.authors, "author", b)),
      el("span", { class: "tag-row" }, genreTag(b), langTag(b), productionTag(b),
        b.announced ? el("span", { class: "tag ok", title: t("Das Hörbuch ist angekündigt, aber noch nicht erschienen") },
          t("erscheint {date}", { date: new Date(b.announced + "T12:00:00").toLocaleDateString(LANG, { dateStyle: "medium" }) }))
        : b.later ? el("span", { class: "tag ok", title: t("Das Hörbuch wurde früher besprochen – das Hörbuch ist erst jetzt erschienen") }, t("jetzt als Hörbuch")) : null),
      b.narrators?.length ? el("span", { class: "muted small" }, t("gelesen von"), " ", ...personLinks(b.narrators, "narrator", b, 2)) : null,
      b.rating ? el("span", { class: "rating", title: plural(b.ratings || 0, t("1 Bewertung bei Audible"), t("{n} Bewertungen bei Audible", { n: fmtCount(b.ratings || 0) })) },
        `★ ${b.rating.toFixed(1)}`, el("span", { class: "muted" }, ` (${fmtCount(b.ratings || 0)})`)) : null,
      facts ? el("span", { class: "muted small" }, facts) : null,
      reason, editionNote(b)),
    // own row across the whole card (under cover and text), so the buttons never squeeze into the text column
    el("div", { class: "s-actions" },
      requestButton(b),
      b.url ? el("a", { class: "btn small ghost", href: b.url, target: "_blank", rel: "noopener noreferrer" }, "Audible") : null,
      b.asin && discover.section !== "follows" ? el("button", { type: "button", class: "btn small ghost", title: t("Dieses Hörbuch nicht mehr vorschlagen"),
        onclick: () => { void hideSuggestion(b); } }, t("Nicht interessiert")) : null));
  return card;
}

function renderDiscover(res: DiscoverAnswer): void {
  const list = $("#discoverList");
  // only what needs a look: a running update or an error – no time stamps, no counts
  $("#discoverState").textContent = res.running ? t("Wird zusammengestellt …") : res.error ? t("Fehler: {errors}", { errors: res.error }) : "";
  $("#discoverNote").hidden = !(discover.section === "genre" && discover.nonfiction.has(discover.genre));
  // "Für dich": personal when the account's listening is known – otherwise say how it gets personal
  const personal = $("#discoverPersonal");
  personal.hidden = discover.section !== "foryou";
  if (discover.section === "foryou") {
    personal.replaceChildren(res.personal ? t("Passend zu dem, was du zuletzt gehört hast.")
      : res.linked ? t("Noch nichts gehört, das Hörwerk zuordnen kann – bis dahin Vorschläge zur ganzen Bibliothek.")
        : el("span", {}, t("Vorschläge zur ganzen Bibliothek. Persönlich werden sie, wenn du unter "),
          el("a", { href: "/account" }, t("Konto")), t(" dein Audiobookshelf verbindest.")));
  }
  discover.last = res;
  const items = filtered(res.items);
  if (res.items.length && !items.length) {
    list.replaceChildren(el("li", { class: "empty" }, t("Nichts passt zu den Filtern."), " ",
      el("button", { type: "button", class: "linkish", onclick: resetFilters }, t("Filter zurücksetzen"))));
    return;
  }
  if (items.length) {
    list.replaceChildren(...items.map(suggestionCard));
    // wide screens have the column: it starts with the first one, so it is never an empty strip
    if (window.matchMedia(WIDE).matches && !detail.current && !$("#viewDiscover").hidden) showDetail(items[0], false);
    else if (detail.current) for (const c of list.querySelectorAll<HTMLElement>("[data-asin]")) c.classList.toggle("selected", c.dataset.asin === detail.current.asin);
    return;
  }
  let text = discover.section === "genre"
    ? t("Hier gibt es gerade keine passenden Hörbücher, die du noch nicht hast.")
    : t("Keine Vorschläge, die alle Qualitätsregeln erfüllen.");
  if (res.running) text = t("Vorschläge werden zusammengestellt – das dauert beim ersten Mal etwas.");
  else if (discover.section === "genre" && !discover.genre) text = t("Genre wählen.");
  else if (discover.section === "foryou" && !res.updated) text = t("Noch keine Hörbücher mit ASIN in der Bibliothek.");
  list.replaceChildren(el("li", { class: "empty" }, text));
}

async function loadDiscover(): Promise<void> {
  clearTimeout(discover.timer);
  for (const b of document.querySelectorAll<HTMLElement>("#discoverSections button")) {
    b.setAttribute("aria-selected", String(b.dataset.section === discover.section));
  }
  renderGenreChips();
  const follows = discover.section === "follows";
  $("#discoverRefresh").hidden = follows;
  if (follows) {
    $("#discoverState").textContent = "";
    $("#discoverNote").hidden = $("#discoverPersonal").hidden = true;
    $("#discoverList").dataset.shows = "follows";
    await renderFollows();
    return;
  }
  if (discover.section === "genre" && !discover.genre) {
    $("#discoverState").textContent = "";
    $("#discoverNote").hidden = $("#discoverPersonal").hidden = true;
    $("#discoverList").dataset.shows = "";
    discover.last = null;
    $("#discoverList").replaceChildren(el("li", { class: "empty" }, t("Genre wählen.")));
    if (!discover.panel) { discover.panel = true; renderGenreChips(); }
    return;
  }
  const params = new URLSearchParams({ section: discover.section, genre: discover.section === "genre" ? discover.genre : "" });
  const list = $("#discoverList");
  const fresh = list.dataset.shows !== params.toString();     // another section or genre: no stale cards meanwhile
  await loadList(list, async () => {
    const res = await api<DiscoverAnswer>("GET", `/api/discover?${params}`, undefined, { key: "discover" });
    renderDiscover(res);
    list.dataset.shows = params.toString();
    if (res.running && !$("#viewDiscover").hidden) discover.timer = setTimeout(loadDiscover, 3000);
  }, { count: 8, kind: "card", fresh, quiet: !fresh });
}

async function loadGenres(): Promise<void> {
  if (discover.genres.length) return;
  try {
    const [g, s] = await Promise.all([api<{ genres: Genre[] }>("GET", "/api/discover/genres"),
      api<Settings>("GET", "/api/settings").catch(() => ({} as Settings))]);
    discover.genres = g.genres;
    discover.nonfiction = new Set(g.genres.filter((x) => x.nonfiction).flatMap((x) => [x.id, ...x.children.map((c) => c.id)]));
    const ids = new Set(g.genres.flatMap((x) => [x.id, ...x.children.map((c) => c.id)]));
    // the genre picked last (per browser); never picked: the default an admin set for "Nach Genre", else none
    const saved = store.get("discoverGenre", String(s.discover_genre || ""));
    discover.genre = ids.has(saved) ? saved : "";
  } catch { /* the store is down: the genre list stays empty */ }
}

/** The genre that is picked, and its main category. */
function pickedGenre(): { top: Genre | undefined; sub: SubGenre | undefined } {
  const top = discover.genres.find((x) => x.id === discover.genre || x.children.some((c) => c.id === discover.genre));
  return { top, sub: top?.children.find((c) => c.id === discover.genre) };
}

/** The list as the filters want it. On "Nach Genre" the server already chose the genre; elsewhere it is a filter. */
function filtered(items: Suggestion[]): Suggestion[] {
  const { top, sub } = pickedGenre();
  const name = sub?.name || top?.name || "";
  const nowYear = new Date().getFullYear();
  const hidden = new Set((typeof ME !== "undefined" && ME?.hidden) || []);
  return items.filter((b) => {
    if (b.asin && hidden.has(b.asin.toUpperCase())) return false;
    if (name && discover.section !== "genre" && !(b.genres || []).includes(name)) return false;
    const min = b.runtime_min || 0;
    if (discover.length === "short" && !(min > 0 && min <= 300)) return false;
    if (discover.length === "mid" && !(min > 300 && min <= 720)) return false;
    if (discover.length === "long" && !(min > 720)) return false;
    const year = Number((b.release_date || "").slice(0, 4)) || 0;
    if (discover.year === "2" && !(year >= nowYear - 1)) return false;
    if (discover.year === "5" && !(year >= nowYear - 4)) return false;
    if (discover.year === "old" && !(year > 0 && year < nowYear - 4)) return false;
    return true;
  });
}

/** One quiet row: genre (folded away until wanted), length, year. The genre panel holds the main categories and,
 *  for the one picked, its subcategories. A second tap on what is picked takes it back. */
function renderGenreChips(): void {
  const bar = $("#discoverFilters"), panel = $("#genrePanel"), row = $("#discoverGenres"), sub = $("#discoverSubGenres");
  const show = discover.section !== "follows";
  bar.hidden = !show;
  const canGenre = discover.genres.length > 0;
  $("#genreToggle").hidden = !canGenre;
  panel.hidden = !(show && canGenre && discover.panel);
  $("#genreToggle").setAttribute("aria-expanded", String(!panel.hidden));
  const { top, sub: picked } = pickedGenre();
  $("#genreLabel").textContent = picked ? `${top?.name} › ${picked.name}` : top?.name || t("Alle");
  $("#genreToggle").classList.toggle("active", !!top);
  $("#genreClear").hidden = !top || !show;
  $<HTMLSelectElement>("#lengthFilter").value = discover.length;
  $<HTMLSelectElement>("#yearFilter").value = discover.year;
  $("#lengthFilter").classList.toggle("active", !!discover.length);
  $("#yearFilter").classList.toggle("active", !!discover.year);
  if (panel.hidden) return;
  row.replaceChildren(
    el("button", { type: "button", class: "chip", "aria-pressed": String(!top && discover.section !== "genre"), onclick: () => { void selectGenre("", true); } }, t("Alle Genres")),
    ...discover.genres.map((x) => el("button", {
      type: "button", class: "chip", "aria-pressed": String(x === top),
      onclick: () => { void selectGenre(x === top ? "" : x.id, !x.children.length); },
    }, x.name)));
  sub.hidden = !(top && top.children.length);
  if (top && top.children.length) {
    sub.replaceChildren(el("button", { type: "button", class: "chip", "aria-pressed": String(!picked),
      onclick: () => { void selectGenre(top.id, true); } }, t("Alle in {genre}", { genre: top.name })),
    ...top.children.map((c) => el("button", { type: "button", class: "chip", "aria-pressed": String(c === picked),
      onclick: () => { void selectGenre(c === picked ? top.id : c.id, c !== picked); } }, c.name)));
  }
}

async function selectGenre(id: string, close: boolean): Promise<void> {
  if (discover.genre !== id) closeDetail();
  discover.genre = id;
  store.set("discoverGenre", id);       // per browser, also for family accounts
  if (close && id) discover.panel = false;
  if (discover.section === "genre") await loadDiscover();
  else { renderGenreChips(); if (discover.last) renderDiscover(discover.last); }
}

function resetFilters(): void {
  discover.length = discover.year = "";
  store.set("discoverLength", ""); store.set("discoverYear", "");
  void selectGenre("", false);
}

async function refreshDiscover(): Promise<void> {
  if (discover.section === "genre" && !discover.genre) return;
  try {
    await api("POST", "/api/discover/refresh", { section: discover.section, genre: discover.genre });
    await loadDiscover();
  } catch (err) { toast(errMsg(err), true); }
}

async function showDiscover(): Promise<void> {
  await loadRequestKeys();
  if (discover.section !== "follows") await loadGenres();
  await loadDiscover();
}

/** A short "how it works" for family accounts – until it was read once (per browser). */
function initWelcome(): void {
  if (IS_ADMIN || store.get("welcomeRead", false)) return;
  $("#welcome").hidden = false;
  $("#welcomeOk").addEventListener("click", () => { store.set("welcomeRead", true); $("#welcome").hidden = true; });
}

function initDiscover(): void {
  initWelcome();
  discover.section = store.get("discoverSection", "foryou");
  if (!(discover.section in DISCOVER_TITLES)) discover.section = "foryou";
  for (const b of document.querySelectorAll<HTMLElement>("#discoverSections button")) {
    b.addEventListener("click", () => {
      // another list: the column does not keep a book that is not in it any more (on wide screens it starts with
      // the first book of the new list)
      if (discover.section !== (b.dataset.section || "foryou")) closeDetail();
      discover.section = b.dataset.section || "foryou";
      store.set("discoverSection", discover.section);
      showDiscover();
    });
  }
  $("#discoverRefresh").addEventListener("click", refreshDiscover);
  discover.length = store.get("discoverLength", "");
  discover.year = store.get("discoverYear", "");
  $("#genreToggle").addEventListener("click", () => { discover.panel = !discover.panel; renderGenreChips(); });
  $("#genreClear").addEventListener("click", () => { void selectGenre("", false); });
  for (const [sel, key] of [["#lengthFilter", "length"], ["#yearFilter", "year"]] as const) {
    $<HTMLSelectElement>(sel).addEventListener("change", (e) => {
      discover[key] = (e.target as HTMLSelectElement).value;
      store.set(key === "length" ? "discoverLength" : "discoverYear", discover[key]);
      closeDetail();
      renderGenreChips();
      if (discover.last) renderDiscover(discover.last);
    });
  }
  document.addEventListener("viewchange", (e) => {
    if ((e as CustomEvent<string>).detail === "discover") showDiscover(); else clearTimeout(discover.timer);
  });
  if (!$("#viewDiscover").hidden) showDiscover();
}

initDiscover();
