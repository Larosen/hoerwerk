"use strict";

// ------------------------------------------------------------------------- //
// Indexer search -> SABnzbd, downloads land in the inbox
// ------------------------------------------------------------------------- //
const search = {
  sources: null as Sources | null,
  results: [] as Release[],
  book: null as Book | null,
};

function fmtSize(bytes: number): string {
  if (!bytes) return "–";
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let n = bytes;
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return `${n.toFixed(n < 10 && i > 1 ? 1 : 0)} ${u[i]}`;
}

function fmtAge(ts: number | null): string {
  if (!ts) return "–";
  const d = (Date.now() / 1000 - ts) / 86400;
  if (d < 1) return `${Math.max(1, Math.round(d * 24))} h`;
  if (d < 60) return t("{n} T", { n: Math.round(d) });
  if (d < 730) return t("{n} M", { n: Math.round(d / 30) });
  return t("{n} J", { n: Math.round(d / 365) });
}

async function loadSources(): Promise<void> {
  let s: Sources;
  try { s = search.sources = await api<Sources>("GET", "/api/sources"); } catch (e) { toast(errMsg(e), true); return; }
  // the connections themselves live under Einstellungen; here only a note when something is missing for the search
  const missing: string[] = [];
  if (!s.indexers.some((i) => i.enabled && i.apikey_set)) missing.push(t("ein Indexer"));
  if (!s.configured.sab) missing.push("SABnzbd");
  const box = $("#sourcesInfo");
  box.hidden = !missing.length;
  box.replaceChildren(...(missing.length ? [
    t("Für die Release-Suche fehlt noch: {what}.", { what: missing.length > 1 ? t("ein Indexer und SABnzbd") : missing[0] }), " ",
    el("a", { href: "/settings/downloads" }, t("Einrichten")),
  ] : []));
}

function indexerRow(i: Indexer = {}): HTMLDivElement {
  const row = el("div", { class: "indexer-row" },
    el("input", { name: "name", type: "text", placeholder: t("Name"), value: i.name || "" }),
    el("input", { name: "url", type: "text", placeholder: "https://indexer.example/api", value: i.url || "", spellcheck: "false" }),
    el("input", { name: "apikey", type: "password", autocomplete: "off", placeholder: i.apikey_set ? t("Gespeichert – leer lassen") : "API-Key" }),
    el("input", { name: "categories", type: "text", value: i.categories || "3030", title: t("Newznab-Kategorien, kommagetrennt"), class: "cat" }),
    el("label", { class: "check", title: t("Aktiv") }, el("input", { name: "enabled", type: "checkbox", checked: i.enabled !== false })),
    el("label", { class: "check first", title: t("Diesen Indexer zuerst fragen – die anderen nur, wenn er nichts Passendes hat") },
      el("input", { name: "preferred", type: "checkbox", checked: !!i.preferred, onchange: (e: Event) => {
        // only one can be first
        if (!(e.target as HTMLInputElement).checked) return;
        for (const other of document.querySelectorAll<HTMLInputElement>("#indexerRows [name=preferred]")) {
          if (other !== e.target) other.checked = false;
        }
      } }), " ", t("Zuerst")),
    el("button", { type: "button", class: "icon-btn", title: t("Entfernen"), onclick: () => row.remove() }, icon("x")));
  row.dataset.id = i.id || "";
  return row;
}

/** Fill the form on /settings/downloads and /settings/services with the saved sources. */
function fillSources(): void {
  const s: Sources = search.sources || { indexers: [], sab: {}, configured: { indexers: 0, sab: false } };
  $("#indexerRows").replaceChildren(...(s.indexers.length ? s.indexers : [{}]).map((i) => indexerRow(i)));
  const f = $<HTMLFormElement>("#sourcesForm");
  field(f, "sab_url").value = s.sab.url || "";
  field(f, "sab_category").value = s.sab.category || "";
  const sabKey = field(f, "sab_apikey");
  sabKey.value = "";
  sabKey.placeholder = s.sab.apikey_set ? t("Gespeichert – leer lassen") : t("API-Key (SABnzbd → Config → General)");
  const abs = s.abs;
  field(f, "abs_url").value = abs?.url || "";
  field(f, "abs_library").value = abs?.library || "";
  const token = field(f, "abs_token");
  token.value = "";
  token.placeholder = abs?.token_set ? t("Gespeichert – leer lassen") : "API-Token";
  const plex = s.plex;
  field(f, "plex_url").value = plex?.url || "";
  field(f, "plex_section").value = plex?.section || "";
  field(f, "plex_empty_trash").checked = !!plex?.empty_trash;
  const plexToken = field(f, "plex_token");
  plexToken.value = "";
  plexToken.placeholder = plex?.token_set ? t("Gespeichert – leer lassen") : "X-Plex-Token";
}

async function saveSources(): Promise<void> {
  const f = $<HTMLFormElement>("#sourcesForm");
  const value = (row: Element, name: string): HTMLInputElement => {
    const input = row.querySelector<HTMLInputElement>(`[name=${name}]`);
    if (!input) throw new Error(`Feld fehlt: ${name}`);
    return input;
  };
  const indexers = [...document.querySelectorAll<HTMLElement>("#indexerRows .indexer-row")].map((r) => ({
    id: r.dataset.id || undefined,
    name: value(r, "name").value,
    url: value(r, "url").value,
    apikey: value(r, "apikey").value,
    categories: value(r, "categories").value,
    enabled: value(r, "enabled").checked,
    preferred: value(r, "preferred").checked,
  }));
  const sab = { url: field(f, "sab_url").value, apikey: field(f, "sab_apikey").value, category: field(f, "sab_category").value };
  const abs = { url: field(f, "abs_url").value, token: field(f, "abs_token").value, library: field(f, "abs_library").value };
  const plex = { url: field(f, "plex_url").value, token: field(f, "plex_token").value, section: field(f, "plex_section").value,
    empty_trash: field(f, "plex_empty_trash").checked };
  let saved: Sources;
  try {
    saved = search.sources = await api<Sources>("PUT", "/api/sources", { indexers, sab, abs, plex });
  } catch (e) { toast(errMsg(e), true); return; }
  await loadSources();
  // test everything that is set up, report in one toast
  type TestResult = { ok: boolean; message: string };
  const tests: Promise<string>[] = saved.indexers.filter((i) => i.enabled && i.apikey_set)
    .map((i) => api<TestResult>("POST", "/api/sources/test", { kind: "indexer", id: i.id })
      .then((r) => `${i.name}: ${r.ok ? "✓ " + r.message : "✗ " + r.message}`));
  if (saved.configured.sab) {
    tests.push(api<TestResult>("POST", "/api/sources/test", { kind: "sab" }).then((r) => (r.ok ? "✓ " : "✗ SABnzbd: ") + r.message));
  }
  if (saved.abs?.url && saved.abs?.token_set) {
    tests.push(api<TestResult>("POST", "/api/sources/test", { kind: "abs" })
      .then((r) => (r.ok ? "✓ Audiobookshelf: " : "✗ ") + r.message)
      .finally(loadSources));
  }
  if (saved.plex?.url && saved.plex?.token_set) {
    tests.push(api<TestResult>("POST", "/api/sources/test", { kind: "plex" })
      .then((r) => (r.ok ? "✓ Plex: " : "✗ ") + r.message)
      .finally(loadSources));
  }
  const lines = await Promise.all(tests.map((p) => p.catch((e: unknown) => "✗ " + errMsg(e))));
  if (lines.length) toast(lines.join(" · "), lines.some((l) => l.startsWith("✗") || l.includes(": ✗")));
}

function surname(name: string | undefined): string {
  const parts = (name || "").trim().split(/\s+/);
  return parts[parts.length - 1] || "";
}

/** The shop title without the series remark it often carries – "Schneewittchen muss sterben (Ein Bodenstein-Kirchhoff-Krimi 4)"
 *  is "Schneewittchen muss sterben" in release names (the server does the same, see releases.clean_title). */
function cleanTitle(title: string): string {
  let s = title.split(/\s+/).join(" ").trim();
  for (;;) {
    const shorter = s.replace(/\s*[([][^()[\]]*[)\]]\s*$/, "")
      .replace(/\s*[,:\-–—]\s*(?:band|bd|teil|folge|buch|book|vol|volume|nr)\.?\s*\d{1,3}\s*$/i, "").trim();
    if (shorter === s || !/\p{L}|\d/u.test(shorter)) return s;
    s = shorter;
  }
}

/** The first search term for a book; when it stays unchanged, the server searches round by round with variants. */
function bookQueries(b: Book): { query: string } {
  return { query: `${surname((b.authors || [])[0])} ${cleanTitle(b.title)}`.trim() };
}

async function lookupBooks(e?: Event): Promise<void> {
  e?.preventDefault();
  const input = $<HTMLInputElement>("#searchQuery");
  const q = input.value.trim();
  if (!q) { input.focus(); return; }
  clearBook();
  const btn = $<HTMLButtonElement>("#bookLookupBtn");
  try {
    const language = document.querySelector<HTMLInputElement>("#bookLang input:checked")?.value || "de";
    store.set("bookLang", language);
    const res = await api<{ books: Book[]; region: string }>("POST", "/api/search/books", { query: q, language },
      { key: "booksearch", busy: t("Sucht …"), button: btn });
    const box = $("#bookPick");
    box.hidden = false;
    box.replaceChildren(
      el("div", { class: "muted small" }, res.books.length ? t("Welches Hörbuch? Bitte auswählen.")
        : t("Weder bei Audible noch bei BookBeat gefunden. Anders schreiben, einen Shop-Link einfügen oder direkt bei den Indexern suchen.")),
      seriesRow(res.books) || "",
      el("div", { class: "pick-list" }, res.books.map((b) => el("button", { type: "button", class: "pick", onclick: () => pickBook(b) },
        b.cover_url ? el("img", { class: "thumb", src: b.cover_url, alt: "", loading: "lazy" }) : coverArt(b.title, (b.authors || [])[0]),
        el("span", { class: "pick-text" },
          el("b", {}, b.title),
          el("span", {}, (b.authors || []).join(", ")),
          el("span", { class: "tag-row" }, langTag(b), genreTag(b)),
          el("span", { class: "muted" }, [b.series ? `${b.series} ${b.series_position || ""}`.trim() : "",
            b.runtime_min ? fmtDuration(b.runtime_min * 60) : "", b.format_type === "abridged" ? t("Gekürzt") : "",
            b.source && b.source !== "audible" ? t("via {source}", { source: SOURCE_NAMES[b.source] || b.source }) : ""].filter(Boolean).join(" · ")),
          b.owned ? el("span", { class: "owned", title: b.owned.path }, t("✓ schon in der Bibliothek")) : editionNote(b))))));
  } catch (err) { if (!isSuperseded(err)) toast(errMsg(err), true); }
  void lookupFree(q);
}

interface FreeBook { id: string; title: string; authors?: string[]; language?: string; runtime_min?: number | null;
  url: string; cover_url?: string; state?: string; owned?: { path: string } | null }

/** Free and legal: LibriVox recordings (public domain) that match the same text – loaded straight into the inbox. */
async function lookupFree(q: string): Promise<void> {
  const box = $("#freePick");
  box.hidden = true;
  if (!IS_ADMIN) return;
  const language = document.querySelector<HTMLInputElement>("#bookLang input:checked")?.value || "de";
  let books: FreeBook[];
  try {
    books = (await api<{ books: FreeBook[] }>("POST", "/api/search/free", { query: q, language }, { key: "freesearch" })).books;
  } catch { return; }                                // LibriVox away: the shop search is what matters
  if (!books.length) return;
  box.hidden = false;
  box.replaceChildren(
    el("div", { class: "muted small" }, t("Frei und legal bei LibriVox (gemeinfrei, von Freiwilligen gelesen) – ein Klick lädt das Hörbuch in den Eingang:")),
    el("div", { class: "pick-list" }, books.map((b) => {
      const state = el("span", { class: b.owned ? "owned" : "muted" },
        b.owned ? t("✓ schon in der Bibliothek") : b.state === "loading" ? t("Wird geladen …") : b.state === "done" ? t("✓ geladen") : t("In den Eingang laden"));
      const card = el("button", { type: "button", class: "pick", title: b.url, onclick: async () => {
        if (card.disabled) return;
        card.disabled = true;
        try {
          const res = await api<{ title: string; files: number }>("POST", "/api/search/free/get", { id: b.id });
          state.textContent = t("Wird geladen …");
          toast(t("„{title}“ wird geladen ({n} Dateien) und erscheint dann unter Hörbücher.", { title: res.title, n: res.files }));
        } catch (err) { card.disabled = false; toast(errMsg(err), true); }
      } },
        b.cover_url ? el("img", { class: "thumb", src: b.cover_url, alt: "", loading: "lazy", referrerpolicy: "no-referrer" }) : coverArt(b.title, (b.authors || [])[0]),
        el("span", { class: "pick-text" },
          el("b", {}, b.title),
          el("span", {}, (b.authors || []).join(", ")),
          el("span", { class: "muted" }, [b.language === "german" ? t("Deutsch") : b.language === "english" ? t("Englisch") : b.language || "",
            b.runtime_min ? fmtDuration(b.runtime_min * 60) : "", "LibriVox"].filter(Boolean).join(" · ")),
          state));
      card.disabled = !!b.owned || b.state === "loading";
      return card;
    })));
}

/** The series among the hits (once each): "Ganze Reihe anfragen" without picking a single volume first. */
function seriesRow(books: Book[]): HTMLElement | null {
  const seen = new Map<string, Book>();
  for (const b of books) if (b.series_asin && !seen.has(b.series_asin)) seen.set(b.series_asin, b);
  if (!seen.size) return null;
  return el("div", { class: "series-row" }, ...[...seen.values()].slice(0, 4).map((b) =>
    el("button", { type: "button", class: "series-chip", onclick: () => openSeries(b),
      title: t("Alle Bände der Reihe „{series}“ ansehen und anfragen", { series: b.series || "" }) },
      icon("books"), el("span", { class: "series-chip-text" }, el("b", {}, b.series || ""),
        el("span", { class: "muted small" }, [t("Ganze Reihe"), (b.authors || [])[0] || ""].filter(Boolean).join(" · "))),
      langTag(b))));
}

function pickBook(b: Book): void {
  search.book = b;
  // the releases are searched in the language of the chosen edition
  const lang = (b.language || "").toLowerCase();
  if (lang) $<HTMLSelectElement>("#searchLang").value = lang === "english" ? "en" : lang === "german" || lang === "deutsch" ? "de" : "";
  $<HTMLInputElement>("#releaseQuery").value = bookQueries(b).query;
  $("#releaseForm").hidden = !IS_ADMIN;
  const box = $("#bookPick");
  box.hidden = false;
  box.replaceChildren(el("div", { class: "picked" },
    b.cover_url ? el("img", { class: "thumb", src: b.cover_url, alt: "" }) : null,
    el("span", {}, el("b", {}, ...personLinks(b.authors, "author", b), ` – ${b.title}`),
      b.narrators?.length ? el("span", { class: "muted" }, " · ", t("gelesen von"), " ", ...personLinks(b.narrators, "narrator", b, 2)) : null,
      el("span", { class: "muted" }, " · " + (b.asin ? t("ASIN {asin} wird beim Download mitgegeben", { asin: b.asin })
        : t("Titel und Autor werden beim Download mitgegeben")))),
    typeof requestButton === "function" ? requestButton(b) : null,
    typeof seriesButton === "function" ? seriesButton(b) : null,
    el("button", { type: "button", class: "icon-btn", title: t("Auswahl aufheben"), onclick: clearBook }, icon("x"))));
  if (IS_ADMIN) runSearch();                 // family accounts ask for the book; releases are Hörwerk's job
}

function clearBook(): void {
  search.book = null;
  const box = $("#bookPick");
  box.hidden = true;
  box.replaceChildren();
  $("#releaseForm").hidden = true;
}

interface LibraryHit { path: string; title: string; authors: string[]; format: string }
interface SearchAnswer { results: Release[]; errors: string[]; resting?: string[]; queries?: string[]; library?: LibraryHit[]; asked?: string[]; skipped?: string[] }

/** Releases at the indexers: for the picked book (term changeable), or – without a book – for the typed text. */
async function runSearch(e?: Event, free = false, everywhere = false): Promise<void> {
  e?.preventDefault();
  const q = (free ? $<HTMLInputElement>("#searchQuery") : $<HTMLInputElement>("#releaseQuery")).value.trim();
  if (!q) return;
  if (free) clearBook();
  const go = $<HTMLButtonElement>("#searchGo");
  const lang = $<HTMLSelectElement>("#searchLang").value;
  store.set("searchLang", lang);
  try {
    // while the term is still the suggested one, the server tries more spellings and broader terms if nothing fits
    const book = search.book && q === bookQueries(search.book).query ? search.book : null;
    const res = await api<SearchAnswer>("POST", "/api/search", { query: q, language: lang, book, everywhere },
      { key: "releasesearch", busy: t("Sucht …"), timeout: 120_000, button: go });
    search.results = res.results;
    renderResults();
    const queries = res.queries || [];
    $("#searchNote").textContent = (res.results.length ? t("{n} Treffer", { n: res.results.length })
      : t("Keine Releases gefunden. Suchbegriff oben ändern, z. B. nur Titel oder Nachname.")) +
      (queries.length > 1 ? " · " + t("gesucht als: {queries}", { queries: queries.map((x) => t("„{title}“", { title: x })).join(", ") }) : "") +
      (res.errors.length ? " · " + t("Fehler: {errors}", { errors: res.errors.join("; ") }) : "") +
      (res.resting?.length ? " · " + restingText(res.resting) : "");
    const skipped = res.skipped || [];
    if (skipped.length) {
      // the preferred indexer had something: the others were not needed – but can be asked with one click
      $("#searchNote").append(" · " + t("Nur bei {name} gesucht", { name: (res.asked || []).join(", ") }) + " ",
        el("button", { type: "button", class: "btn small", onclick: () => { void runSearch(undefined, free, true); } },
          t("Auch bei {names} suchen", { names: skipped.join(", ") })));
    }
    renderOwnedNote(res.library || []);
  } catch (err) {
    if (isSuperseded(err)) return;                 // a newer search shows its own result
    toast(errMsg(err), true);
    $("#searchNote").textContent = errMsg(err);
  }
}

/** The search term itself matches books in the library: say so above the results. */
function renderOwnedNote(hits: LibraryHit[]): void {
  const box = $("#searchOwned");
  box.replaceChildren();
  box.hidden = !hits.length;
  if (!hits.length) return;
  box.append(icon("check"), el("span", {}, el("b", {}, t("Schon in deiner Bibliothek:")), " ",
    ...hits.flatMap((h, i) => [i ? " · " : "", el("span", { title: h.path },
      `${(h.authors || [])[0] ? (h.authors || [])[0] + " – " : ""}${h.title}`,
      h.format && h.format !== "m4b" ? el("span", { class: "tag soft" }, h.format.toUpperCase()) : null)])));
}

function renderResults(): void {
  const body = $("#searchResults");
  body.replaceChildren();
  if (!search.results.length) {
    body.append(el("tr", {}, el("td", { colspan: "5", class: "empty-row" }, t("Keine Treffer – anders formulieren (weniger Wörter, ohne Band-Nummer)."))));
    return;
  }
  for (const r of sortReleases(search.results)) {
    const tags = [
      r.format, r.language === "de" ? t("Deutsch") : r.language === "en" ? t("Englisch") : null,
      r.bitrate ? r.bitrate + " kbps" : null, r.abridged ? t("Gekürzt") : null, r.drama ? t("Hörspiel") : null,
    ].filter((t): t is string => !!t);
    const btn: HTMLButtonElement = el("button", { class: "btn small primary", onclick: () => sendResult(r, btn) }, t("Download"));
    body.append(el("tr", {},
      el("td", { class: "rel" },
        el("div", { class: "rel-title", title: r.title }, r.title),
        el("div", { class: "rel-meta" },
          r.pack ? packNote(r.pack) : null,
          r.owned ? el("span", { class: "owned", title: r.owned.path },
            t("✓ vorhanden: {book}", { book: `${(r.owned.authors || [])[0] || ""} – ${r.owned.title}${r.owned.count > 1 ? ` (+${r.owned.count - 1})` : ""}` })) : null,
          tags.map((t) => el("span", { class: "tag" }, t)),
          el("span", { class: "muted" }, [r.indexer, ...(r.also || [])].join(", ")),
          r.details ? el("a", { href: r.details, target: "_blank", rel: "noopener noreferrer" }, t("Details")) : null)),
      el("td", { class: "c-num" }, fmtSize(r.size)),
      el("td", { class: "c-num", "data-label": t("Alter"), title: r.published ? new Date(r.published * 1000).toLocaleString(LOCALE) : "" }, fmtAge(r.published)),
      el("td", { class: "c-num", "data-label": "Grabs" }, r.grabs ?? "–"),
      el("td", { class: "c-act" }, btn)));
  }
}

/** "Sneijder und Nemez · Bände 1–7 · vorhanden: 1, 2" under a release with several volumes. */
function packNote(p: NonNullable<Release["pack"]>): HTMLSpanElement {
  const have = p.volumes.filter((v) => p.owned[String(v)]);
  const range = `${p.volumes[0]}–${p.volumes[p.volumes.length - 1]}`;
  return el("span", { class: "pack-note" },
    el("span", { class: "tag" }, t("{n} Bände", { n: p.volumes.length })),
    `${p.series ? p.series + " · " : ""}${t("Bände {range}", { range })}`,
    have.length ? el("span", { class: "owned" }, " · " + (have.length === p.volumes.length ? t("✓ alle vorhanden")
      : t("✓ vorhanden: {list}", { list: have.join(", ") }))) : null);
}

/** Which volumes of a pack to keep – the ones already in the library start unticked. */
function choosePackVolumes(r: Release): Promise<number[] | null> {
  const p = r.pack!;
  const d = $<HTMLDialogElement>("#packDialog");
  $("#packInfo").textContent = t("„{title}“ enthält {n} Bände. Nur die ausgewählten werden nach dem Download verarbeitet, die anderen übersprungen.", { title: r.title, n: p.volumes.length });
  const boxes = p.volumes.map((v) => {
    const owned = p.owned[String(v)];
    const cb = el("input", { type: "checkbox", value: String(v), checked: !owned });
    return { v, cb, row: el("label", { class: "check pack-vol" + (owned ? " is-owned" : "") }, cb,
      el("span", {}, t("Band {n}", { n: v })),
      owned ? el("span", { class: "muted small", title: owned }, t("vorhanden: {path}", { path: owned })) : null) };
  });
  const list = $("#packVolumes");
  list.replaceChildren(...boxes.map((b) => b.row));
  const count = (): void => {
    const n = boxes.filter((b) => b.cb.checked).length;
    $<HTMLButtonElement>("#packOk").disabled = !n;
    $("#packOk").textContent = t("{n} Bände herunterladen", { n });
  };
  boxes.forEach((b) => b.cb.addEventListener("change", count));
  $("#packAll").onclick = () => { boxes.forEach((b) => (b.cb.checked = true)); count(); };
  $("#packMissing").onclick = () => { boxes.forEach((b) => (b.cb.checked = !p.owned[String(b.v)])); count(); };
  count();
  return new Promise((resolve) => {
    d.returnValue = "";
    d.addEventListener("close", () => resolve(d.returnValue === "ok" ? boxes.filter((b) => b.cb.checked).map((b) => b.v) : null), { once: true });
    d.showModal();
  });
}

async function sendResult(r: Release, btn: HTMLButtonElement): Promise<void> {
  if (!search.sources?.configured.sab) { toast(t("Erst SABnzbd einrichten"), true); navigate("/settings/downloads"); return; }
  let volumes: number[] | undefined;
  if (r.pack) {
    const chosen = await choosePackVolumes(r);
    if (!chosen) return;
    volumes = chosen;
  } else {
    const owned = r.owned || search.book?.owned;
    if (owned && !(await confirmBox(t("Sieht aus, als wäre das schon in der Bibliothek: „{path}“. Trotzdem herunterladen?", { path: owned.path }), t("Trotzdem herunterladen")))) return;
  }
  btn.disabled = true;
  try {
    const b = search.book;
    await api("POST", "/api/search/send", {
      id: r.id, ...(b && !r.pack ? { asin: b.asin, title: b.title } : {}), ...(b ? { author: (b.authors || [])[0] } : {}),
      ...(volumes ? { volumes } : {}),
    });
    btn.textContent = t("✓ gesendet");
    toast(t("An SABnzbd geschickt: {title} – zu sehen unter Einstellungen › Downloads", { title: r.title }));
  } catch (e) { btn.disabled = false; toast(errMsg(e), true); }
}

function initSearch(): void {
  const lang = $<HTMLSelectElement>("#searchLang");
  lang.value = store.get("searchLang", "de");
  const bookLang = store.get("bookLang", "de");
  for (const r of document.querySelectorAll<HTMLInputElement>("#bookLang input")) {
    r.checked = r.value === bookLang;
    r.addEventListener("change", () => { store.set("bookLang", r.value); if ($<HTMLInputElement>("#searchQuery").value.trim()) lookupBooks(); });
  }
  $("#searchForm").addEventListener("submit", lookupBooks);
  $("#releaseForm").addEventListener("submit", (e) => runSearch(e));
  initReleaseSort($("#searchSort"), () => { if (search.results.length) renderResults(); });
  $("#freeSearchBtn").addEventListener("click", () => runSearch(undefined, true));
  $("#addIndexerBtn").addEventListener("click", () => $("#indexerRows").append(indexerRow()));
  $("#sourcesForm").addEventListener("submit", (e) => { e.preventDefault(); saveSources(); });
  document.addEventListener("viewchange", (e) => {
    if ((e as CustomEvent<string>).detail === "search" && IS_ADMIN) loadSources();
  });
  if (!$("#viewSearch").hidden && IS_ADMIN) loadSources();
}

initSearch();
