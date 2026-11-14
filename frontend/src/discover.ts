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

function suggestionCard(b: Suggestion): HTMLLIElement {
  const author = (b.authors || [])[0] || "";
  const facts = [
    b.series ? `${b.series} ${b.series_position || ""}`.trim() : "",
    b.runtime_min ? fmtDuration(b.runtime_min * 60) : "",
    (b.release_date || "").slice(0, 4),
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
  return el("li", { class: "suggestion" },
    b.cover_url ? el("img", { class: "cover", src: b.cover_url, alt: "", loading: "lazy" }) : coverArt(b.title, author),
    el("div", { class: "s-body" },
      el("b", { class: "s-title", title: b.title }, b.title),
      el("span", { class: "s-author" }, ...personLinks(b.authors, "author", b)),
      el("span", { class: "tag-row" }, genreTag(b), langTag(b),
        b.announced ? el("span", { class: "tag ok", title: t("Das Hörbuch ist angekündigt, aber noch nicht erschienen") },
          t("erscheint {date}", { date: new Date(b.announced + "T12:00:00").toLocaleDateString(LANG, { dateStyle: "medium" }) }))
        : b.later ? el("span", { class: "tag ok", title: t("Das Hörbuch wurde früher besprochen – das Hörbuch ist erst jetzt erschienen") }, t("jetzt als Hörbuch")) : null),
      b.narrators?.length ? el("span", { class: "muted small" }, t("gelesen von"), " ", ...personLinks(b.narrators, "narrator", b, 2)) : null,
      b.rating ? el("span", { class: "rating", title: plural(b.ratings || 0, t("1 Bewertung bei Audible"), t("{n} Bewertungen bei Audible", { n: fmtCount(b.ratings || 0) })) },
        `★ ${b.rating.toFixed(1)}`, el("span", { class: "muted" }, ` (${fmtCount(b.ratings || 0)})`)) : null,
      facts ? el("span", { class: "muted small" }, facts) : null,
      reason),
    // own row across the whole card (under cover and text), so the buttons never squeeze into the text column
    el("div", { class: "s-actions" },
      requestButton(b),
      el("button", { type: "button", class: "btn small", onclick: () => searchSuggestion(b) }, icon("search"), t("Suchen")),
      b.url ? el("a", { class: "btn small ghost", href: b.url, target: "_blank", rel: "noopener noreferrer" }, "Audible") : null));
}

/** Into the release search with this book picked (title, author and ASIN help later when tagging). */
function searchSuggestion(b: Suggestion): void {
  setView("search");
  pickBook(b);
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
  if (res.items.length) {
    list.replaceChildren(...res.items.map(suggestionCard));
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
  const params = new URLSearchParams({ section: discover.section, genre: discover.genre });
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
    const saved = store.get("discoverGenre", String(s.discover_genre || ""));
    // nothing picked yet: start with fiction, so the list is never empty
    const first = g.genres.find((x) => /Literatur|Fiction/.test(x.name)) || g.genres.find((x) => !x.nonfiction) || g.genres[0];
    discover.genre = ids.has(saved) ? saved : first?.id || "";
  } catch { /* the store is down: the genre list stays empty */ }
}

/** Genres as chips: on "Für dich" they lead to the genre page, there a second row offers the subcategories. */
function renderGenreChips(): void {
  const row = $("#discoverGenres"), sub = $("#discoverSubGenres");
  const show = (discover.section === "foryou" || discover.section === "genre") && discover.genres.length > 0;
  row.hidden = !show;
  sub.hidden = true;
  if (!show) return;
  const top = discover.genres.find((x) => x.id === discover.genre || x.children.some((c) => c.id === discover.genre));
  const here = discover.section === "genre";
  row.replaceChildren(...discover.genres.map((x) => el("button", {
    type: "button", class: "chip", "aria-pressed": String(here && x === top),
    onclick: () => { void selectGenre(x.id, true); },
  }, x.name + (x.nonfiction ? " · " + t("Sachbuch") : ""))));
  if (here && top && top.children.length) {
    sub.hidden = false;
    sub.replaceChildren(el("button", { type: "button", class: "chip", "aria-pressed": String(discover.genre === top.id),
      onclick: () => { void selectGenre(top.id, false); } }, t("Alle")),
    ...top.children.map((c) => el("button", { type: "button", class: "chip", "aria-pressed": String(discover.genre === c.id),
      onclick: () => { void selectGenre(c.id, false); } }, c.name)));
  }
}

async function selectGenre(id: string, fromChip: boolean): Promise<void> {
  discover.genre = id;
  store.set("discoverGenre", id);       // per browser, also for family accounts
  if (fromChip && discover.section !== "genre") { discover.section = "genre"; store.set("discoverSection", "genre"); }
  await loadDiscover();
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
  if (discover.section === "genre" || discover.section === "foryou") await loadGenres();
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
      discover.section = b.dataset.section || "foryou";
      store.set("discoverSection", discover.section);
      showDiscover();
    });
  }
  $("#discoverRefresh").addEventListener("click", refreshDiscover);
  document.addEventListener("viewchange", (e) => {
    if ((e as CustomEvent<string>).detail === "discover") showDiscover(); else clearTimeout(discover.timer);
  });
  if (!$("#viewDiscover").hidden) showDiscover();
}

initDiscover();
