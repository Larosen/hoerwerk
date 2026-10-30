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
      el("span", { class: i.apikey_set ? "ok" : "err", title: i.apikey_set ? t("API-Key gespeichert") : t("API-Key fehlt") }, i.apikey_set ? "✓" : "✗")));
  }
  box.append(el("div", { class: "kv" }, el("span", {}, "SABnzbd"),
    el("span", { title: s.sab.url || "" }, s.sab.url ? `${s.sab.url}${s.sab.category ? " · " + s.sab.category : ""}` : t("nicht eingerichtet")),
    el("span", { class: s.configured.sab ? "ok" : "err" }, s.configured.sab ? "✓" : "✗")));
  const abs = s.abs;
  if (abs?.url) {
    box.append(el("div", { class: "kv" }, el("span", {}, "Audiobookshelf"),
      el("span", { title: abs.last || abs.url }, abs.library_name || abs.library || t("Bibliothek fehlt")),
      el("span", { class: abs.configured ? "ok" : "err" }, abs.configured ? "✓" : "✗")));
  }
}

function indexerRow(i: Indexer = {}): HTMLDivElement {
  const row = el("div", { class: "indexer-row" },
    el("input", { name: "name", type: "text", placeholder: t("Name"), value: i.name || "" }),
    el("input", { name: "url", type: "text", placeholder: "https://indexer.example/api", value: i.url || "", spellcheck: "false" }),
    el("input", { name: "apikey", type: "password", autocomplete: "off", placeholder: i.apikey_set ? t("gespeichert – leer lassen") : "API-Key" }),
    el("input", { name: "categories", type: "text", value: i.categories || "3030", title: t("Newznab-Kategorien, kommagetrennt"), class: "cat" }),
    el("label", { class: "check", title: t("aktiv") }, el("input", { name: "enabled", type: "checkbox", checked: i.enabled !== false })),
    el("button", { type: "button", class: "icon-btn", title: t("Entfernen"), onclick: () => row.remove() }, "✕"));
  row.dataset.id = i.id || "";
  return row;
}

function openSources(): void {
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
  $<HTMLDialogElement>("#sourcesDialog").showModal();
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
  let saved: Sources;
  try {
    saved = search.sources = await api<Sources>("PUT", "/api/sources", { indexers, sab, abs });
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
  const lines = await Promise.all(tests.map((p) => p.catch((e: unknown) => "✗ " + errMsg(e))));
  if (lines.length) toast(lines.join(" · "), lines.some((l) => l.startsWith("✗") || l.includes(": ✗")));
  pollDownloads(true);
}

function surname(name: string | undefined): string {
  const parts = (name || "").trim().split(/\s+/);
  return parts[parts.length - 1] || "";
}

function bookQueries(b: Book): { query: string; extra: string[] } {
  const author = surname((b.authors || [])[0]);
  const extra = [b.title];
  if (b.series && b.series_position) extra.push(`${b.series} ${b.series_position}`);
  return { query: `${author} ${b.title}`.trim(), extra };
}

async function lookupBooks(): Promise<void> {
  const input = $<HTMLInputElement>("#searchQuery");
  const q = input.value.trim();
  if (!q) { input.focus(); return; }
  const btn = $<HTMLButtonElement>("#bookLookupBtn");
  btn.disabled = true;
  btn.textContent = t("Schlage nach …");
  try {
    const res = await api<{ books: Book[]; region: string }>("POST", "/api/search/books", { query: q });
    const box = $("#bookPick");
    box.hidden = false;
    box.replaceChildren(
      el("div", { class: "muted small" }, res.books.length ? t("Welches Buch? ({shop})", { shop: `audible.${res.region === "us" ? "com" : res.region}` }) : t("Bei Audible nichts gefunden – einfach direkt suchen.")),
      el("div", { class: "pick-list" }, res.books.map((b) => el("button", { type: "button", class: "pick", onclick: () => pickBook(b) },
        b.cover_url ? el("img", { class: "thumb", src: b.cover_url, alt: "", loading: "lazy" }) : el("span", { class: "thumb none" }, "?"),
        el("span", { class: "pick-text" },
          el("b", {}, b.title),
          el("span", {}, (b.authors || []).join(", ")),
          el("span", { class: "muted" }, [b.series ? `${b.series} ${b.series_position || ""}`.trim() : "",
            b.runtime_min ? fmtDuration(b.runtime_min * 60) : "", b.format_type === "abridged" ? t("gekürzt") : "",
            b.source && b.source !== "audible" ? t("via {source}", { source: SOURCE_NAMES[b.source] || b.source }) : ""].filter(Boolean).join(" · ")),
          b.owned ? el("span", { class: "owned", title: b.owned.path }, t("✓ schon in der Bibliothek")) : null)))));
  } catch (err) { toast(errMsg(err), true); } finally { btn.disabled = false; btn.textContent = t("Bei Audible nachschlagen"); }
}

function pickBook(b: Book): void {
  search.book = b;
  const { query } = bookQueries(b);
  $<HTMLInputElement>("#searchQuery").value = query;
  $("#bookPick").replaceChildren(el("div", { class: "picked" },
    b.cover_url ? el("img", { class: "thumb", src: b.cover_url, alt: "" }) : null,
    el("span", {}, t("Gesucht wird:") + " ", el("b", {}, `${(b.authors || []).join(", ")} – ${b.title}`),
      el("span", { class: "muted" }, " · " + t("ASIN {asin} wird beim Download mitgegeben", { asin: b.asin || "–" }))),
    el("button", { type: "button", class: "icon-btn", title: t("Auswahl aufheben"), onclick: clearBook }, "✕")));
  runSearch();
}

function clearBook(): void {
  search.book = null;
  const box = $("#bookPick");
  box.hidden = true;
  box.replaceChildren();
}

interface SearchAnswer { results: Release[]; errors: string[]; queries?: string[] }

async function runSearch(e?: Event): Promise<void> {
  e?.preventDefault();
  const q = $<HTMLInputElement>("#searchQuery").value.trim();
  if (!q) return;
  // typed something else since picking a book: that book no longer applies
  if (search.book && q !== bookQueries(search.book).query) clearBook();
  const go = $<HTMLButtonElement>("#searchGo");
  go.disabled = true;
  go.textContent = t("Suche …");
  const lang = $<HTMLSelectElement>("#searchLang").value;
  store.set("searchLang", lang);
  try {
    const extra = search.book ? bookQueries(search.book).extra : [];
    const res = await api<SearchAnswer>("POST", "/api/search", { query: q, language: lang, extra });
    search.results = res.results;
    renderResults();
    const queries = res.queries || [];
    $("#searchNote").textContent = t("{n} Treffer", { n: res.results.length }) +
      (queries.length > 1 ? " · " + t("gesucht als: {queries}", { queries: queries.map((x) => `„${x}“`).join(", ") }) : "") +
      (res.errors.length ? " · " + t("Fehler: {errors}", { errors: res.errors.join("; ") }) : "");
  } catch (err) {
    toast(errMsg(err), true);
    $("#searchNote").textContent = errMsg(err);
  } finally { go.disabled = false; go.textContent = t("Suchen"); }
}

function renderResults(): void {
  const body = $("#searchResults");
  body.replaceChildren();
  if (!search.results.length) {
    body.append(el("tr", {}, el("td", { colspan: "5", class: "empty-row" }, t("Keine Treffer – anders formulieren (weniger Wörter, ohne Band-Nummer)."))));
    return;
  }
  for (const r of search.results) {
    const tags = [
      r.format, r.language === "de" ? t("Deutsch") : r.language === "en" ? t("Englisch") : null,
      r.bitrate ? r.bitrate + " kbps" : null, r.abridged ? t("gekürzt") : null, r.drama ? t("Hörspiel") : null,
    ].filter((t): t is string => !!t);
    const btn: HTMLButtonElement = el("button", { class: "btn small primary", onclick: () => sendResult(r, btn) }, t("Download"));
    body.append(el("tr", {},
      el("td", { class: "rel" },
        el("div", { class: "rel-title", title: r.title }, r.title),
        el("div", { class: "rel-meta" },
          r.owned ? el("span", { class: "owned", title: r.owned.path },
            t("✓ vorhanden: {book}", { book: `${(r.owned.authors || [])[0] || ""} – ${r.owned.title}${r.owned.count > 1 ? ` (+${r.owned.count - 1})` : ""}` })) : null,
          tags.map((t) => el("span", { class: "tag" }, t)),
          el("span", { class: "muted" }, [r.indexer, ...(r.also || [])].join(", ")),
          r.details ? el("a", { href: r.details, target: "_blank", rel: "noopener noreferrer" }, t("Details")) : null)),
      el("td", { class: "c-num" }, fmtSize(r.size)),
      el("td", { class: "c-num", title: r.published ? new Date(r.published * 1000).toLocaleString(LOCALE) : "" }, fmtAge(r.published)),
      el("td", { class: "c-num" }, r.grabs ?? "–"),
      el("td", { class: "c-act" }, btn)));
  }
}

async function sendResult(r: Release, btn: HTMLButtonElement): Promise<void> {
  if (!search.sources?.configured.sab) { toast(t("Erst SABnzbd einrichten"), true); openSources(); return; }
  const owned = r.owned || search.book?.owned;
  if (owned && !(await confirmBox(t("Sieht aus, als wäre das schon in der Bibliothek: „{path}“. Trotzdem herunterladen?", { path: owned.path }), t("Trotzdem laden")))) return;
  btn.disabled = true;
  try {
    const b = search.book;
    await api("POST", "/api/search/send", b ? { id: r.id, asin: b.asin, title: b.title, author: (b.authors || [])[0] } : { id: r.id });
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
  $("#searchForm").addEventListener("submit", runSearch);
  $("#sourcesBtn").addEventListener("click", openSources);
  $("#bookLookupBtn").addEventListener("click", lookupBooks);
  $("#addIndexerBtn").addEventListener("click", () => $("#indexerRows").append(indexerRow()));
  const dialog = $<HTMLDialogElement>("#sourcesDialog");
  dialog.addEventListener("close", () => { if (dialog.returnValue === "save") saveSources(); });
  document.addEventListener("viewchange", (e) => {
    if ((e as CustomEvent<string>).detail === "search") { loadSources(); pollDownloads(true); }
  });
  if (!$("#viewSearch").hidden) { loadSources(); pollDownloads(true); }
}

initSearch();
