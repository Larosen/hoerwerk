"use strict";

// ------------------------------------------------------------------------- //
// Entdecken: suggested audiobooks – for the library, by genre, from reviews
// ------------------------------------------------------------------------- //
interface Suggestion extends Book {
  rating?: number; ratings?: number; because?: string[];
  review?: { source: string; link: string; date: string; notes?: number };
}
interface DiscoverAnswer { items: Suggestion[]; updated: number | null; error: string | null; running: boolean }
interface Genre { id: string; name: string; nonfiction: boolean }
interface WantedFind { title: string; size: number; indexer: string; format?: string; abridged?: boolean }
interface WantedItem extends Book {
  id: string; added: number; last_search: number | null; found: WantedFind[]; error: string | null;
  status: "missing" | "found" | "loading";
}
interface WantedAnswer { items: WantedItem[]; interval: number; auto: boolean; indexers: number }

const discover = {
  section: "foryou",
  genre: "",
  genres: [] as Genre[],
  timer: undefined as number | undefined,
  wanted: new Set<string>(),        // ids on the wanted list (ASIN or title key)
};

const DISCOVER_TITLES: Record<string, () => string> = {
  foryou: () => t("Für dich"),
  genre: () => t("Nach Genre"),
  critics: () => t("Aus der Kritik"),
  wanted: () => t("Gesucht"),
};

/** Put a book on the wanted list (searched on the indexers every few hours). */
async function rememberBook(b: Book, btn?: HTMLButtonElement): Promise<void> {
  if (btn) btn.disabled = true;
  try {
    await api("POST", "/api/wanted", b);
    if (b.asin) discover.wanted.add(b.asin.toUpperCase());
    toast(t("Gemerkt – wird regelmäßig gesucht"));
    if (btn) btn.replaceChildren(icon("check"), t("Gemerkt"));
    updateWantedCount();
  } catch (err) { toast(errMsg(err), true); if (btn) btn.disabled = false; }
}

function rememberButton(b: Book): HTMLButtonElement | null {
  if (!IS_ADMIN) return null;                // requests for everyone come with the next step
  const known = !!b.asin && discover.wanted.has(b.asin.toUpperCase());
  const btn = el("button", { type: "button", class: "btn small ghost", title: t("Auf die Liste „Gesucht“ – wird regelmäßig bei den Indexern gesucht") },
    known ? icon("check") : null, known ? t("Gemerkt") : t("Merken"));
  btn.disabled = known;
  btn.addEventListener("click", () => rememberBook(b, btn));
  return btn;
}

async function updateWantedCount(): Promise<WantedAnswer | null> {
  if (!IS_ADMIN) return null;
  try {
    const res = await api<WantedAnswer>("GET", "/api/wanted");
    discover.wanted = new Set(res.items.flatMap((i) => [i.id, (i.asin || "").toUpperCase()]).filter(Boolean));
    const badge = $("#wantedCount");
    badge.textContent = String(res.items.length);
    badge.hidden = !res.items.length;
    return res;
  } catch { return null; }
}

function wantedCard(w: WantedItem): HTMLLIElement {
  const author = (w.authors || [])[0] || "";
  const best = w.found[0];
  const status = w.status === "loading" ? el("span", { class: "w-status ok" }, t("Download läuft"))
    : w.status === "found" ? el("span", { class: "w-status found" }, t("{n} Release(s) gefunden", { n: w.found.length }))
      : el("span", { class: "w-status" }, t("Noch nicht gefunden"));
  const searchBtn = el("button", { type: "button", class: "btn small ghost" }, icon("refresh"), t("Jetzt suchen"));
  searchBtn.addEventListener("click", async () => {
    searchBtn.disabled = true;
    try { await api("POST", "/api/wanted/search", { id: w.id }); await loadWanted(); } catch (err) { toast(errMsg(err), true); searchBtn.disabled = false; }
  });
  return el("li", { class: "suggestion" },
    w.cover_url ? el("img", { class: "cover", src: w.cover_url, alt: "", loading: "lazy" }) : coverArt(w.title, author),
    el("div", { class: "s-body" },
      el("b", { class: "s-title", title: w.title }, w.title),
      el("span", { class: "s-author" }, (w.authors || []).join(", ")),
      status,
      best ? el("span", { class: "muted small", title: best.title }, `${best.title} · ${best.indexer}`) : null,
      el("span", { class: "muted small" }, w.last_search ? t("Zuletzt gesucht: {when}", { when: fmtAgo(w.last_search) }) : t("Noch nicht gesucht")),
      w.error ? el("span", { class: "muted small err-text" }, w.error) : null,
      el("div", { class: "s-actions" },
        el("button", { type: "button", class: "btn small" + (w.found.length ? " primary" : ""), onclick: () => searchSuggestion(w) }, icon("search"), t("Ansehen")),
        searchBtn,
        el("button", { type: "button", class: "icon-btn", title: t("Von der Liste nehmen"), onclick: async () => {
          try { await api("DELETE", `/api/wanted/${encodeURIComponent(w.id)}`); await loadWanted(); } catch (err) { toast(errMsg(err), true); }
        } }, icon("x")))));
}

async function loadWanted(): Promise<void> {
  const res = await updateWantedCount();
  if (!res) return;
  $("#discoverState").textContent = !res.indexers ? t("Kein Indexer eingerichtet (Suchen → Einrichten)")
    : t("Wird alle {n} Stunden gesucht", { n: res.interval }) + (res.auto ? " · " + t("Funde werden automatisch geladen") : "");
  $("#discoverList").replaceChildren(...(res.items.length ? res.items.map(wantedCard)
    : [el("li", { class: "empty" }, t("Noch nichts gemerkt. „Merken“ gibt es auf den Karten in Entdecken und bei einem in „Suchen“ gewählten Hörbuch."))]));
}

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
    reason = el("p", { class: "reason" }, t("Passt zu: {titles}", { titles: b.because.map((x) => `„${x}“`).join(", ") }));
  } else if (b.review) {
    reason = el("p", { class: "reason" }, t("Besprochen bei {source}", { source: b.review.source }),
      b.review.notes ? " · " + t("{n} Kritiken in der Presse", { n: b.review.notes }) : null,
      b.review.link ? el("span", {}, " · ", el("a", { href: b.review.link, target: "_blank", rel: "noopener noreferrer" }, t("Kritik lesen"))) : null);
  }
  return el("li", { class: "suggestion" },
    b.cover_url ? el("img", { class: "cover", src: b.cover_url, alt: "", loading: "lazy" }) : coverArt(b.title, author),
    el("div", { class: "s-body" },
      el("b", { class: "s-title", title: b.title }, b.title),
      el("span", { class: "s-author" }, (b.authors || []).join(", ")),
      b.narrators?.length ? el("span", { class: "muted small" }, t("gelesen von {names}", { names: b.narrators.slice(0, 2).join(", ") })) : null,
      b.rating ? el("span", { class: "rating", title: t("{n} Bewertungen bei Audible", { n: fmtCount(b.ratings || 0) }) },
        `★ ${b.rating.toFixed(1)}`, el("span", { class: "muted" }, ` (${fmtCount(b.ratings || 0)})`)) : null,
      facts ? el("span", { class: "muted small" }, facts) : null,
      reason,
      el("div", { class: "s-actions" },
        el("button", { type: "button", class: "btn small primary", onclick: () => searchSuggestion(b) }, icon("search"), t("Suchen")),
        rememberButton(b),
        b.url ? el("a", { class: "btn small ghost", href: b.url, target: "_blank", rel: "noopener noreferrer" }, "Audible") : null)));
}

/** Into the release search with this book picked (title, author and ASIN help later when tagging). */
function searchSuggestion(b: Suggestion): void {
  setView("search");
  pickBook(b);
}

function renderDiscover(res: DiscoverAnswer): void {
  const list = $("#discoverList");
  const state = $("#discoverState");
  const when = res.updated ? new Date(res.updated * 1000).toLocaleString(LANG, { dateStyle: "short", timeStyle: "short" }) : "";
  state.textContent = res.running ? t("Wird zusammengestellt …") : res.error ? t("Fehler: {errors}", { errors: res.error })
    : when ? t("Stand: {time}", { time: when }) : "";
  if (res.items.length) {
    list.replaceChildren(...res.items.map(suggestionCard));
    return;
  }
  let text = t("Keine Vorschläge, die alle Qualitätsregeln erfüllen.");
  if (res.running) text = t("Vorschläge werden zusammengestellt – das dauert beim ersten Mal etwas.");
  else if (discover.section === "genre" && !discover.genre) text = t("Genre wählen.");
  else if (discover.section === "foryou" && !res.updated) text = t("Noch keine Bücher mit ASIN in der Bibliothek.");
  list.replaceChildren(el("li", { class: "empty" }, text));
}

async function loadDiscover(): Promise<void> {
  clearTimeout(discover.timer);
  $("#discoverTitle").textContent = DISCOVER_TITLES[discover.section]();
  $("#discoverGenreWrap").hidden = discover.section !== "genre";
  $("#discoverRefresh").hidden = discover.section === "wanted";
  for (const b of document.querySelectorAll<HTMLElement>("#discoverSections button")) {
    b.setAttribute("aria-current", String(b.dataset.section === discover.section));
  }
  if (discover.section === "wanted") { await loadWanted(); return; }
  const params = new URLSearchParams({ section: discover.section, genre: discover.genre });
  try {
    const res = await api<DiscoverAnswer>("GET", `/api/discover?${params}`);
    renderDiscover(res);
    if (res.running && !$("#viewDiscover").hidden) discover.timer = setTimeout(loadDiscover, 3000);
  } catch (err) { toast(errMsg(err), true); }
}

async function loadGenres(): Promise<void> {
  if (discover.genres.length) return;
  try {
    const [g, s] = await Promise.all([api<{ genres: Genre[] }>("GET", "/api/discover/genres"), api<Settings>("GET", "/api/settings")]);
    discover.genres = g.genres;
    const sel = $<HTMLSelectElement>("#discoverGenre");
    sel.replaceChildren(el("option", { value: "" }, "–"),
      ...g.genres.map((x) => el("option", { value: x.id }, x.name + (x.nonfiction ? ` (${t("Sachbuch")})` : ""))));
    const saved = String(s.discover_genre || "");
    if (g.genres.some((x) => x.id === saved)) { sel.value = saved; discover.genre = saved; }
  } catch { /* the store is down: the genre list stays empty */ }
}

async function chooseGenre(): Promise<void> {
  discover.genre = $<HTMLSelectElement>("#discoverGenre").value;
  api<Settings>("PUT", "/api/settings", { discover_genre: discover.genre }).catch(() => { /* only a preselection */ });
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
  await updateWantedCount();
  if (discover.section === "genre") await loadGenres();
  await loadDiscover();
}

function initDiscover(): void {
  discover.section = store.get("discoverSection", "foryou");
  if (!(discover.section in DISCOVER_TITLES) || (!IS_ADMIN && discover.section === "wanted")) discover.section = "foryou";
  for (const b of document.querySelectorAll<HTMLElement>("#discoverSections button")) {
    b.addEventListener("click", () => {
      discover.section = b.dataset.section || "foryou";
      store.set("discoverSection", discover.section);
      showDiscover();
    });
  }
  $("#discoverGenre").addEventListener("change", chooseGenre);
  $("#discoverRefresh").addEventListener("click", refreshDiscover);
  document.addEventListener("viewchange", (e) => {
    if ((e as CustomEvent<string>).detail === "discover") showDiscover(); else clearTimeout(discover.timer);
  });
  if (!$("#viewDiscover").hidden) showDiscover();
}

initDiscover();
