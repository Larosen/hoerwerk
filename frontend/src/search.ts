"use strict";

// ------------------------------------------------------------------------- //
// Indexer search -> SABnzbd (like Readarr), downloads land in the inbox
// ------------------------------------------------------------------------- //
const search = {
  sources: null as Sources | null,
  dlTimer: undefined as number | undefined,
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
  const box = $("#sourcesInfo");
  box.replaceChildren();
  if (!s.indexers.length) box.append(el("div", { class: "muted" }, t("Noch kein Indexer eingerichtet.")));
  for (const i of s.indexers) {
    box.append(el("div", { class: "kv" }, el("span", {}, t("Indexer")),
      el("span", { title: i.url }, (i.name || "") + (i.enabled ? "" : " " + t("(aus)"))),
      el("span", { class: i.apikey_set ? "ok" : "err", title: i.apikey_set ? t("API-Key gespeichert") : t("API-Key fehlt") }, icon(i.apikey_set ? "check" : "alert"))));
  }
  box.append(el("div", { class: "kv" }, el("span", {}, "SABnzbd"),
    el("span", { title: s.sab.url || "" }, s.sab.url ? `${s.sab.url}${s.sab.category ? " · " + s.sab.category : ""}` : t("nicht eingerichtet")),
    el("span", { class: s.configured.sab ? "ok" : "err" }, icon(s.configured.sab ? "check" : "alert"))));
  const abs = s.abs;
  if (abs?.url) {
    box.append(el("div", { class: "kv" }, el("span", {}, "Audiobookshelf"),
      el("span", { title: abs.last || abs.url }, abs.library_name || abs.library || t("Bibliothek fehlt")),
      el("span", { class: abs.configured ? "ok" : "err" }, icon(abs.configured ? "check" : "alert"))));
  }
  const plex = s.plex;
  if (plex?.url) {
    box.append(el("div", { class: "kv" }, el("span", {}, "Plex"),
      el("span", { title: plex.last || plex.url }, plex.section_name || plex.section || t("Mediathek fehlt")),
      el("span", { class: plex.configured ? "ok" : "err" }, icon(plex.configured ? "check" : "alert"))));
  }
}

function indexerRow(i: Indexer = {}): HTMLDivElement {
  const row = el("div", { class: "indexer-row" },
    el("input", { name: "name", type: "text", placeholder: t("Name"), value: i.name || "" }),
    el("input", { name: "url", type: "text", placeholder: "https://indexer.example/api", value: i.url || "", spellcheck: "false" }),
    el("input", { name: "apikey", type: "password", autocomplete: "off", placeholder: i.apikey_set ? t("gespeichert – leer lassen") : "API-Key" }),
    el("input", { name: "categories", type: "text", value: i.categories || "3030", title: t("Newznab-Kategorien, kommagetrennt"), class: "cat" }),
    el("label", { class: "check", title: t("aktiv") }, el("input", { name: "enabled", type: "checkbox", checked: i.enabled !== false })),
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
  sabKey.placeholder = s.sab.apikey_set ? t("gespeichert – leer lassen") : t("API-Key (SABnzbd → Config → General)");
  const abs = s.abs;
  field(f, "abs_url").value = abs?.url || "";
  field(f, "abs_library").value = abs?.library || "";
  const token = field(f, "abs_token");
  token.value = "";
  token.placeholder = abs?.token_set ? t("gespeichert – leer lassen") : "API-Token";
  const plex = s.plex;
  field(f, "plex_url").value = plex?.url || "";
  field(f, "plex_section").value = plex?.section || "";
  field(f, "plex_empty_trash").checked = !!plex?.empty_trash;
  const plexToken = field(f, "plex_token");
  plexToken.value = "";
  plexToken.placeholder = plex?.token_set ? t("gespeichert – leer lassen") : "X-Plex-Token";
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
  pollDownloads(true);
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
  btn.disabled = true;
  btn.textContent = t("Suche …");
  try {
    const language = document.querySelector<HTMLInputElement>("#bookLang input:checked")?.value || "de";
    store.set("bookLang", language);
    const res = await api<{ books: Book[]; region: string }>("POST", "/api/search/books", { query: q, language });
    const box = $("#bookPick");
    box.hidden = false;
    box.replaceChildren(
      el("div", { class: "muted small" }, res.books.length ? t("Welches Hörbuch? Auswählen – dann sucht Hörwerk die Releases dazu.")
        : t("Weder bei Audible noch bei BookBeat gefunden. Anders schreiben, einen Shop-Link einfügen oder direkt bei den Indexern suchen.")),
      seriesRow(res.books) || "",
      el("div", { class: "pick-list" }, res.books.map((b) => el("button", { type: "button", class: "pick", onclick: () => pickBook(b) },
        b.cover_url ? el("img", { class: "thumb", src: b.cover_url, alt: "", loading: "lazy" }) : coverArt(b.title, (b.authors || [])[0]),
        el("span", { class: "pick-text" },
          el("b", {}, b.title),
          el("span", {}, (b.authors || []).join(", ")),
          el("span", { class: "tag-row" }, langTag(b), genreTag(b)),
          el("span", { class: "muted" }, [b.series ? `${b.series} ${b.series_position || ""}`.trim() : "",
            b.runtime_min ? fmtDuration(b.runtime_min * 60) : "", b.format_type === "abridged" ? t("gekürzt") : "",
            b.source && b.source !== "audible" ? t("via {source}", { source: SOURCE_NAMES[b.source] || b.source }) : ""].filter(Boolean).join(" · ")),
          b.owned ? el("span", { class: "owned", title: b.owned.path }, t("✓ schon in der Bibliothek")) : null)))));
  } catch (err) { toast(errMsg(err), true); } finally { btn.disabled = false; btn.textContent = t("Suchen"); }
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
    el("span", {}, el("b", {}, `${(b.authors || []).join(", ")} – ${b.title}`),
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
interface SearchAnswer { results: Release[]; errors: string[]; queries?: string[]; library?: LibraryHit[] }

/** Releases at the indexers: for the picked book (term changeable), or – without a book – for the typed text. */
async function runSearch(e?: Event, free = false): Promise<void> {
  e?.preventDefault();
  const q = (free ? $<HTMLInputElement>("#searchQuery") : $<HTMLInputElement>("#releaseQuery")).value.trim();
  if (!q) return;
  if (free) clearBook();
  const go = $<HTMLButtonElement>("#searchGo");
  go.disabled = true;
  go.textContent = t("Suche …");
  const lang = $<HTMLSelectElement>("#searchLang").value;
  store.set("searchLang", lang);
  try {
    // while the term is still the suggested one, the server tries more spellings and broader terms if nothing fits
    const book = search.book && q === bookQueries(search.book).query ? search.book : null;
    const res = await api<SearchAnswer>("POST", "/api/search", { query: q, language: lang, book });
    search.results = res.results;
    renderResults();
    const queries = res.queries || [];
    $("#searchNote").textContent = (res.results.length ? t("{n} Treffer", { n: res.results.length })
      : t("Keine Releases gefunden – oben den Suchbegriff ändern (z. B. nur Titel oder nur Nachname) und erneut suchen.")) +
      (queries.length > 1 ? " · " + t("gesucht als: {queries}", { queries: queries.map((x) => `„${x}“`).join(", ") }) : "") +
      (res.errors.length ? " · " + t("Fehler: {errors}", { errors: res.errors.join("; ") }) : "");
    renderOwnedNote(res.library || []);
  } catch (err) {
    toast(errMsg(err), true);
    $("#searchNote").textContent = errMsg(err);
  } finally { go.disabled = false; go.textContent = t("Releases suchen"); }
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
  for (const r of sortReleases(search.results, $<HTMLSelectElement>("#searchSort").value)) {
    const tags = [
      r.format, r.language === "de" ? t("Deutsch") : r.language === "en" ? t("Englisch") : null,
      r.bitrate ? r.bitrate + " kbps" : null, r.abridged ? t("gekürzt") : null, r.drama ? t("Hörspiel") : null,
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
  $("#packInfo").textContent = t("„{title}“ enthält {n} Bände. Nur die ausgewählten werden nach dem Download zu M4B gemacht, getaggt und abgelegt; die anderen werden übersprungen.", { title: r.title, n: p.volumes.length });
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
    $("#packOk").textContent = t("{n} Bände laden", { n });
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
    if (owned && !(await confirmBox(t("Sieht aus, als wäre das schon in der Bibliothek: „{path}“. Trotzdem herunterladen?", { path: owned.path }), t("Trotzdem laden")))) return;
  }
  btn.disabled = true;
  try {
    const b = search.book;
    await api("POST", "/api/search/send", {
      id: r.id, ...(b && !r.pack ? { asin: b.asin, title: b.title } : {}), ...(b ? { author: (b.authors || [])[0] } : {}),
      ...(volumes ? { volumes } : {}),
    });
    btn.textContent = t("✓ gesendet");
    toast(t("An SABnzbd geschickt: {title}", { title: r.title }));
    pollDownloads(true);
  } catch (e) { btn.disabled = false; toast(errMsg(e), true); }
}

async function pollDownloads(force = false): Promise<void> {
  clearTimeout(search.dlTimer);
  const visible = !$("#viewSearch").hidden;
  if (!visible && !force) return;
  let data: Downloads;
  try { data = await api<Downloads>("GET", "/api/downloads"); } catch { data = { items: [], error: t("Server nicht erreichbar") }; }
  const list = $("#dlList");
  list.replaceChildren();
  $("#dlSpeed").textContent = data.speed && data.speed !== "0 " ? `${data.speed}B/s${data.paused ? " · " + t("pausiert") : ""}` : data.paused ? t("pausiert") : "";
  if (!data.configured) list.append(el("li", { class: "empty" }, t("SABnzbd ist nicht eingerichtet.")));
  else if (data.error) list.append(el("li", { class: "empty err" }, data.error));
  else if (!data.items.length) list.append(el("li", { class: "empty" }, t("Keine Downloads in dieser Kategorie.")));
  for (const d of data.items || []) {
    const pct = Math.round((d.progress || 0) * 100);
    const failed = d.status === "Failed";
    list.append(el("li", { class: "job " + (failed ? "error" : d.done ? "done" : "running") },
      el("div", { class: "job-top" },
        el("span", { class: "job-name", title: d.name }, d.name),
        el("span", { class: "badge " + (failed ? "error" : d.done ? "done" : "running") },
          d.done ? (failed ? t("fehlgeschlagen") : t("fertig → Eingang")) : `${d.status} ${pct}%`)),
      d.error ? el("div", { class: "job-msg" }, d.error)
        : el("div", { class: "job-msg" }, [d.size, d.left && !d.done ? t("noch {time}", { time: d.left }) : "", d.finished ? fmtAgo(d.finished) : ""].filter(Boolean).join(" · ")),
      !d.done ? el("div", { class: "bar" }, el("i", { style: `width:${pct}%` })) : null));
  }
  const busy = (data.items || []).some((d) => !d.done);
  if (visible) search.dlTimer = setTimeout(pollDownloads, busy ? 3000 : 15000);
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
  initReleaseSort($<HTMLSelectElement>("#searchSort"), () => { if (search.results.length) renderResults(); });
  $("#freeSearchBtn").addEventListener("click", () => runSearch(undefined, true));
  $("#addIndexerBtn").addEventListener("click", () => $("#indexerRows").append(indexerRow()));
  $("#sourcesForm").addEventListener("submit", (e) => { e.preventDefault(); saveSources(); });
  document.addEventListener("viewchange", (e) => {
    if ((e as CustomEvent<string>).detail === "search" && IS_ADMIN) { loadSources(); pollDownloads(true); }
  });
  if (!$("#viewSearch").hidden && IS_ADMIN) { loadSources(); pollDownloads(true); }
}

initSearch();
