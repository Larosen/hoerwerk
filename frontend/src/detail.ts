"use strict";
// ------------------------------------------------------------------------- //
// Details of one audiobook: a column on the right on wide screens (Mac, iPad landscape), a sheet from below on
// narrower ones. Cover, description, facts, what to do (request, search) and where to go on: more by the author or
// narrator, the series, and what listeners of this book also listened to – a tap on one of those opens it in the
// same place, so there is always a next one to look at.
// ------------------------------------------------------------------------- //
type DetailBook = Book & { requested?: string | null; rating?: number; ratings?: number };
interface DetailAnswer { book: DetailBook; similar: DetailBook[]; region: string }

const detail = {
  current: null as DetailBook | null,
  stack: [] as DetailBook[],                     // where "Zurück" leads
  cache: new Map<string, DetailAnswer>(),
  token: 0,
};

const WIDE = "(min-width: 1240px)";

/** Audible knows the book by its ASIN – only then there is a description and "similar" to ask for. */
function knownAtAudible(b: Book): boolean {
  return !!b.asin && (!b.source || b.source === "audible");
}

function detailChip(label: string, run: () => void): HTMLButtonElement {
  return el("button", { type: "button", class: "chip pill", onclick: run }, label);
}

function detailFacts(b: DetailBook): HTMLElement {
  const rows: [string, string][] = [];
  if (b.runtime_min) rows.push([t("Laufzeit"), fmtDuration(b.runtime_min * 60)]);
  if (b.release_date) rows.push([t("Erschienen"), b.release_date.slice(0, 4)]);
  if (b.rating) rows.push([t("Bewertung"), `★ ${b.rating.toFixed(1)} (${fmtCount(b.ratings || 0)})`]);
  if (b.series) rows.push([t("Reihe"), `${b.series} ${b.series_position || ""}`.trim()]);
  if (b.format_type) rows.push([t("Fassung"), b.format_type === "abridged" ? t("Gekürzt") : t("Ungekürzt")]);
  return el("dl", { class: "leaders" }, ...rows.map(([k, v]) => el("div", {}, el("dt", {}, k), el("dd", {}, v))));
}

function renderDetail(b: DetailBook, extra: DetailAnswer | null | "loading"): void {
  const author = (b.authors || [])[0] || "";
  const narrator = (b.narrators || [])[0] || "";
  const lang = bookLangCode(b);
  const loaded = extra && extra !== "loading" ? extra : null;
  const description = (loaded?.book.description || (b as { description?: string }).description || "").trim();
  const shelf = loaded && loaded.similar.length ? el("div", { class: "detail-similar" },
    el("h3", {}, t("Ähnlich wie „{title}“", { title: b.title })),
    el("div", { class: "sim-row" }, ...loaded.similar.map((s) => el("button", { type: "button", class: "sim", title: s.title, onclick: () => showDetail(s) },
      coverFor(s.cover_url, s.title, (s.authors || [])[0], "thumb"),
      el("span", { class: "sim-t" }, s.title),
      el("span", { class: "sim-a" }, (s.authors || [])[0] || "")))))
    : extra === "loading" && knownAtAudible(b) ? el("p", { class: "muted small" }, t("Lade …")) : null;
  const inner = el("div", { class: "detail-inner tint-card" },
    el("div", { class: "detail-bar" },
      detail.stack.length ? el("button", { type: "button", class: "linkish", onclick: backDetail }, "‹ ", t("Zurück")) : el("span"),
      el("button", { type: "button", class: "icon-btn", "aria-label": t("Schließen"), title: t("Schließen"), onclick: closeDetail }, icon("x"))),
    coverFor(b.cover_url, b.title, author, "cover big"),
    el("h2", { class: "detail-title" }, b.title),
    el("p", { class: "detail-by" }, ...personLinks(b.authors, "author", b, 4)),
    narrator ? el("p", { class: "muted small" }, t("gelesen von"), " ", ...personLinks(b.narrators, "narrator", b, 3)) : null,
    el("div", { class: "tag-row" }, genreTag(b), langTag(b)),
    description ? el("p", { class: "detail-blurb" }, description) : null,
    detailFacts(b),
    el("div", { class: "detail-acts" }, requestButton(b),
      el("button", { type: "button", class: "btn small", onclick: () => searchSuggestion(b as Suggestion) }, icon("search"), t("Suchen")),
      b.url ? el("a", { class: "btn small ghost", href: b.url, target: "_blank", rel: "noopener noreferrer" }, "Audible") : null),
    el("div", { class: "detail-more" },
      el("h3", {}, t("Weiterstöbern")),
      el("div", { class: "chips" },
        author ? detailChip(t("Mehr von {name}", { name: author }), () => { void openPerson("author", author, lang); }) : null,
        narrator ? detailChip(t("Gelesen von {name}", { name: narrator }), () => { void openPerson("narrator", narrator, lang); }) : null,
        b.series_asin ? detailChip(t("Ganze Reihe „{series}“", { series: b.series || "" }), () => { void openSeries(b); }) : null),
      shelf));
  $("#bookDetail").replaceChildren(inner);
  $("#bookDetail").scrollTop = 0;
  for (const card of document.querySelectorAll<HTMLElement>("[data-asin]")) card.classList.toggle("selected", !!b.asin && card.dataset.asin === b.asin);
}

async function loadDetail(b: DetailBook): Promise<void> {
  const key = `${b.asin}|${b.region || ""}`;
  const token = ++detail.token;
  let res = detail.cache.get(key);
  if (!res) {
    try {
      res = await api<DetailAnswer>("GET", `/api/book/detail?asin=${encodeURIComponent(b.asin || "")}&region=${encodeURIComponent(b.region || "")}`,
        undefined, { key: "bookdetail" });
      detail.cache.set(key, res);
    } catch { res = undefined; }                  // the shop is away: the book is shown with what the list knew
  }
  if (token !== detail.token || detail.current !== b) return;
  renderDetail(b, res || null);
}

/** Show a book in the column (wide) or the sheet (narrow). */
function showDetail(b: DetailBook, push = true): void {
  if (push && detail.current && detail.current !== b) detail.stack.push(detail.current);
  detail.current = b;
  $("#bookDetail").classList.add("open");
  $("#detailScrim").hidden = false;
  renderDetail(b, knownAtAudible(b) ? "loading" : null);
  if (knownAtAudible(b)) void loadDetail(b);
}

function backDetail(): void {
  const previous = detail.stack.pop();
  if (previous) showDetail(previous, false);
}

function closeDetail(): void {
  detail.current = null;
  detail.stack = [];
  detail.token++;
  $("#bookDetail").classList.remove("open");
  $("#bookDetail").replaceChildren();
  $("#detailScrim").hidden = true;
  for (const card of document.querySelectorAll<HTMLElement>("[data-asin].selected")) card.classList.remove("selected");
}

function initDetail(): void {
  $("#detailScrim").addEventListener("click", closeDetail);
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && detail.current && !window.matchMedia(WIDE).matches) closeDetail();
  });
  // the column belongs to Entdecken and Suchen
  document.addEventListener("viewchange", (e) => {
    const view = (e as CustomEvent<string>).detail;
    if (view !== "discover" && view !== "search") closeDetail();
  });
}

initDetail();
