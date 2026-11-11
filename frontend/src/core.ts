"use strict";
// Shared types of the API answers and the small helpers every page script uses.
// Loaded right after theme.js; the other scripts share these globals (classic scripts, no bundler).

// ------------------------------------------------------------------------- //
// Language: German is the source, /i18n.js brings the catalog of the chosen language
// ------------------------------------------------------------------------- //
declare const I18N: { lang: string; text: Record<string, string> } | undefined;
const LANG = typeof I18N === "undefined" ? "de" : I18N.lang;
const LOCALE = LANG === "en" ? "en-GB" : "de-DE";

// Who is signed in (/me.js; null on the login page). The server checks every right again –
// this only decides what the page shows.
interface Me { id: number; name: string; role: "admin" | "user"; perms: Record<string, boolean>; quota: number | null }
declare const ME: Me | null | undefined;
const IS_ADMIN = typeof ME !== "undefined" && !!ME && ME.role === "admin";
if (typeof ME !== "undefined" && ME) document.documentElement.classList.add(IS_ADMIN ? "role-admin" : "role-user");

/** Translate a German UI text; ``{name}`` placeholders are filled from ``values``. */
function t(text: string, values: Record<string, string | number> = {}): string {
  let out = (typeof I18N !== "undefined" && I18N.text[text]) || text;
  for (const [k, v] of Object.entries(values)) out = out.split("{" + k + "}").join(String(v));
  return out;
}

/** Singular or plural – both texts are passed already translated (so the catalog check finds them). */
function plural(n: number, one: string, many: string): string {
  return n === 1 ? one : many;
}

const TRANSLATED_ATTRS = ["placeholder", "title", "aria-label"];

/** The fixed texts of index.html: every text node and label attribute that has a translation. */
function translateDom(root: ParentNode): void {
  if (typeof I18N === "undefined" || !Object.keys(I18N.text).length) return;
  const norm = (s: string): string => s.replace(/\s+/g, " ").trim();
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const parent = node.parentElement;
    if (!parent || parent.closest("code, script, style")) continue;
    const key = norm(node.nodeValue || "");
    const hit = key && I18N.text[key];
    if (hit) {
      const raw = node.nodeValue || "";
      // "<b>Inbox</b> legen." -> "<b>Inbox</b>." : no space before a translation that starts with punctuation
      const lead = /^\s/.test(raw) && !/^[.,;:!?)]/.test(hit) ? " " : "";
      node.nodeValue = lead + hit + (/\s$/.test(raw) ? " " : "");
    }
  }
  for (const elem of root.querySelectorAll<HTMLElement>("[placeholder], [title], [aria-label]")) {
    for (const attr of TRANSLATED_ATTRS) {
      const v = elem.getAttribute(attr);
      const hit = v && I18N.text[norm(v)];
      if (hit) elem.setAttribute(attr, hit);
    }
  }
  document.documentElement.lang = LANG;
}

// ------------------------------------------------------------------------- //
// API data
// ------------------------------------------------------------------------- //
type Settings = Record<string, string | number | boolean>;

interface Book {
  id?: string; asin: string | null; title: string; subtitle?: string | null; authors: string[]; narrators?: string[];
  series?: string | null; series_asin?: string | null; series_position?: string | null; runtime_min?: number; release_date?: string | null;
  cover_url?: string | null; match?: { score: number }; region?: string | null; url?: string | null;
  format_type?: string | null; publisher?: string | null; language?: string | null; genres?: string[];
  description?: string; source?: string; isbn?: string | null; owned?: Owned | null; other_edition?: OtherEdition | null;
  request_id?: number; request_user?: string; mismatch?: string;
}
interface Owned { path: string; title: string; authors: string[]; why: string; count: number }
/** The same book in the library, but read by someone else – shown as a hint, it does not block a request. */
interface OtherEdition { title: string; narrators?: string[]; path?: string }

function editionNote(b: { other_edition?: OtherEdition | null; owned?: unknown }): HTMLElement | null {
  const e = b.other_edition;
  if (!e || b.owned) return null;
  const who = (e.narrators || []).slice(0, 2).join(", ");
  return el("span", { class: "muted small edition-note", title: e.path || "" },
    who ? t("Andere Fassung in der Bibliothek (gelesen von {names})", { names: who }) : t("Andere Fassung in der Bibliothek"));
}
interface Hints { query?: string; title?: string; author?: string; narrator?: string; folder?: string }
interface Quality { kbps: number; duration: number; size: number; files: number }
interface Duplicate {
  path: string; title: string; authors: string[]; asin: string | null; format: string; why: string;
  quality?: { old: Quality | null; new: Quality | null };
}
type BookStatus = "queued" | "matching" | "converting" | "review" | "tagging" | "done" | "failed" | "cancelled";
interface AudiobookJob {
  id: string; source: string; rel: string; name: string; origin: string; status: BookStatus; stage: string;
  message: string; progress: number; files: string[]; duration: number; hints: Hints; candidates: Book[];
  match: Book | null; region: string | null; dest: string | null; error: string | null;
  converted_info: { bitrate?: string; chapters?: number; copied?: boolean }; duplicate: Duplicate | null;
  log?: string[];
}
interface QueueStat { workers: number; running: number; waiting: number }
interface AudiobooksStatus {
  enabled: boolean; inbox: string; library: string; inbox_ok: boolean; library_ok: boolean;
  last_scan: number | null; scan_note: string; waiting: { name: string; since: number; ready_in: number }[];
  ignored?: string[];
  jobs: AudiobookJob[]; review: number; queues?: { encode: QueueStat; io: QueueStat };
}
interface LibraryStats { books: number; m4b: number; other: number; last_scan: number | null; scanning: boolean }
interface UnconvertedBook {
  path: string; title: string; authors: string[]; series: string | null; series_position: string | null;
  files: number; format: string; job: { id: string; status: BookStatus } | null; cover?: boolean;
}
interface Track { file: string; duration: number; auto: string; filename: string; tag: string }

interface Indexer { id?: string; name?: string; url?: string; categories?: string; enabled?: boolean; preferred?: boolean; apikey_set?: boolean }
interface AbsConfig { url: string; library: string; library_name: string; token_set: boolean; configured: boolean; last: string | null }
interface PlexConfig {
  url: string; section: string; section_name: string; empty_trash: boolean; token_set: boolean; configured: boolean; last: string | null;
}
interface Sources {
  indexers: Indexer[]; sab: { url?: string; category?: string; apikey_set?: boolean };
  configured: { indexers: number; sab: boolean }; abs?: AbsConfig; plex?: PlexConfig;
}
interface Release {
  id: string; title: string; size: number; published: number | null; grabs: number | null; indexer: string;
  also?: string[]; details: string | null; format: string | null; language: string | null; bitrate: number | null;
  abridged: boolean; drama: boolean; owned: Owned | null;
  pack?: { volumes: number[]; series: string; author: string; owned: Record<string, string> };
}
/** The order of a release list: as Hörwerk ranks them, by size or by age (store key "releaseSort"). */
function sortReleases<T extends { size: number; published: number | null }>(list: T[], order: string): T[] {
  const out = list.slice();
  const age = (r: T): number => r.published || 0;
  if (order === "size-desc") out.sort((a, b) => b.size - a.size);
  else if (order === "size-asc") out.sort((a, b) => a.size - b.size);
  else if (order === "new") out.sort((a, b) => age(b) - age(a));
  else if (order === "old") out.sort((a, b) => (age(a) || Infinity) - (age(b) || Infinity));
  return out;
}

/** A sort selector for a release list – remembered across both lists. */
function initReleaseSort(sel: HTMLSelectElement, render: () => void): void {
  sel.value = store.get("releaseSort", "best");
  if (!sel.value) sel.value = "best";
  sel.addEventListener("change", () => {
    store.set("releaseSort", sel.value);
    for (const other of document.querySelectorAll<HTMLSelectElement>(".sort-pick")) other.value = sel.value;
    render();
  });
}

interface Download {
  id?: string | null; name: string; status: string; progress: number; done: boolean; size: string; left: string;
  finished: number | null; error: string | null;
}
interface Downloads { configured?: boolean; items: Download[]; error?: string; speed?: string; paused?: boolean }
interface SystemInfo {
  version: string; ffmpeg: string;
  sandbox: { mode: string; landlock_abi: number; active: boolean; network_blocked: boolean };
}

// ------------------------------------------------------------------------- //
// Helpers
// ------------------------------------------------------------------------- //
const store = {
  get<T>(k: string, d: T): T {
    try { const v = localStorage.getItem("br:" + k); return v === null ? d : JSON.parse(v) as T; } catch { return d; }
  },
  set(k: string, v: unknown): void {
    try { localStorage.setItem("br:" + k, JSON.stringify(v)); } catch { /* private mode: only this page view */ }
  },
};

/** The element for a selector; a missing one is a bug in index.html, not a runtime case. */
function $<T extends Element = HTMLElement>(selector: string): T {
  const node = document.querySelector<T>(selector);
  if (!node) throw new Error(`Element fehlt: ${selector}`);
  return node;
}

/** Named form control (input, select, textarea). */
function field<T extends HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement = HTMLInputElement>(
  form: HTMLFormElement, name: string): T {
  const node = form.elements.namedItem(name);
  if (!node) throw new Error(`Feld fehlt: ${name}`);
  return node as T;
}

type Child = Node | string | number | null | undefined | false | Child[];
interface Attrs {
  [attribute: string]: unknown;
  onclick?: (e: MouseEvent) => void;
  onchange?: (e: Event) => void;
  oninput?: (e: Event) => void;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, attrs: Attrs = {}, ...children: Child[]): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") n.className = String(v);
    else if (k.startsWith("on")) n.addEventListener(k.slice(2), v as EventListener);
    else if (k === "html") n.innerHTML = String(v);   // only ever used with our own static texts
    else if ((k === "href" || k === "src") && !/^(https?:\/\/|#|\/|blob:)/i.test(String(v ?? ""))) continue;   // no javascript:/data: from foreign data
    else if (v !== false && v != null) n.setAttribute(k, v === true ? "" : String(v));
  }
  const add = (c: Child): void => {
    if (Array.isArray(c)) c.forEach(add);
    else if (c != null && c !== false) n.append(typeof c === "number" ? String(c) : c);
  };
  children.forEach(add);
  return n;
}

/** An icon from the sprite in index.html (``<symbol id="i-…">``). */
function icon(name: string, label?: string): SVGSVGElement {
  const NS = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(NS, "svg");
  svg.setAttribute("class", "ico");
  if (label) { svg.setAttribute("role", "img"); svg.setAttribute("aria-label", label); }
  else svg.setAttribute("aria-hidden", "true");
  const use = document.createElementNS(NS, "use");
  use.setAttribute("href", "#i-" + name);
  svg.append(use);
  return svg;
}

/** The main category of a book (Audible's top genre, e.g. "Krimis & Thriller") as a small label. */
function genreTag(b: { genres?: string[] | null }): HTMLElement | null {
  const g = (b.genres || [])[0];
  return g ? el("span", { class: "genre-tag", title: (b.genres || []).join(" · ") }, g) : null;
}

/** A label when a book is not in German – so nobody requests the English edition by mistake. */
function langTag(b: { language?: string | null }, always = false): HTMLElement | null {
  const lang = (b.language || "").toLowerCase();
  const german = lang === "german" || lang === "deutsch";
  if (!lang || (german && !always)) return null;
  const names: Record<string, () => string> = {
    german: () => t("Deutsch"), deutsch: () => t("Deutsch"),
    english: () => t("Englisch"), french: () => t("Französisch"), spanish: () => t("Spanisch"), italian: () => t("Italienisch"),
  };
  return el("span", { class: "lang-tag" + (german ? " de" : ""), title: t("Sprache des Hörbuchs") }, names[lang]?.() || b.language || "");
}

/** Stand-in cover for books without artwork: a calm tile coloured by the title, with its initial. */
function coverArt(title: string, author = "", tag = ""): HTMLDivElement {
  const clean = title.replace(/^[\s\d._-]+/, "").replace(/^(der|die|das|ein|eine|the|a|an|le|la|les|el|il)\s+/i, "") || title;
  let h = 0;
  for (const ch of clean.toLowerCase()) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  const initial = (clean.match(/\p{L}|\p{N}/u) || ["?"])[0].toUpperCase();
  const last = author.split(/[,\s]+/).filter(Boolean).pop() || "";
  return el("div", { class: "thumb gen", style: `--h:${h % 360}`, "aria-hidden": "true" },
    el("span", { class: "gen-initial" }, initial),
    last ? el("span", { class: "gen-author" }, last) : null,
    tag ? el("span", { class: "gen-tag" }, tag) : null);
}

/** The real cover of a library book (from its folder or audio file), the stand-in tile when there is none. */
function libraryCover(path: string, title: string, author = "", tag = "", version: number | string = "",
  hasCover = true): HTMLElement {
  const fallback = (): HTMLDivElement => coverArt(title, author, tag);
  if (!hasCover) return fallback();     // the library index knows: no request that can only fail
  const img = el("img", {
    class: "thumb", alt: "", loading: "lazy", decoding: "async",
    src: `/api/library/cover?path=${encodeURIComponent(path)}${version ? "&v=" + encodeURIComponent(String(version)) : ""}`,
  });
  // MP3 & co.: keep the format visible on top of the picture
  const node: HTMLElement = tag ? el("div", { class: "thumb-wrap" }, img, el("span", { class: "gen-tag" }, tag)) : img;
  img.addEventListener("error", () => node.replaceWith(fallback()), { once: true });
  return node;
}

const collator = new Intl.Collator(LANG, { numeric: true, sensitivity: "base" });

// ------------------------------------------------------------------------- //
// Requests and feedback – every action shows that it was registered, that it runs, and whether it worked.
//
// * ``api()`` is the one way to the server: timeouts, readable network errors, no double requests.
// * A button that starts a changing request (POST/PUT/DELETE) shows a spinner and the running action
//   ("Speichert …") until the answer is there, cannot be pressed again meanwhile, then briefly shows
//   success or failure. This is automatic: the click is remembered and the request is tied to its button.
// * ``toast()`` messages stack, errors stay longer and can be closed.
// * ``loadList()``: a skeleton while a list loads the first time, "Erneut versuchen" when it failed.
// A server answer of 200 only means the server took the job – what happens to a book afterwards is shown by
// its status in the lists (live updates), never by the button.
// ------------------------------------------------------------------------- //
interface ApiOptions {
  /** ms until the request counts as failed (reading 30 s, changing 90 s) */
  timeout?: number;
  /** a newer request with the same key replaces this one (searches while typing): its answer is dropped */
  key?: string;
  /** the button that shows the progress; false: none (default: the button just clicked) */
  button?: HTMLElement | false | null;
  /** text while running, e.g. t("Speichert …") – otherwise derived from the button's label */
  busy?: string;
}

const inflight = new Map<string, Promise<unknown>>();
const latest = new Map<string, AbortController>();
let trigger: { el: HTMLElement; at: number } | null = null;
const TRIGGER_MS = 3000;          // a request started this soon after a click belongs to that click

/** The control that was pressed last (if just now) – its request shows its progress there. */
function currentTrigger(): HTMLElement | null {
  if (!trigger || performance.now() - trigger.at > TRIGGER_MS || !trigger.el.isConnected) return null;
  return trigger.el;
}

function isBusy(node: Element | null): boolean {
  return !!node && node.getAttribute("aria-busy") === "true";
}

// capture phase: before any page handler – a busy button swallows further clicks (no double requests)
document.addEventListener("click", (e) => {
  const target = e.target instanceof Element ? e.target.closest<HTMLElement>("button, a.btn, [role=button], input[type=submit]") : null;
  if (!target) return;
  if (isBusy(target)) { e.preventDefault(); e.stopImmediatePropagation(); return; }
  trigger = { el: target, at: performance.now() };
}, true);
// a setting that saves itself on change (a select, a checkbox) is no button: its request shows no spinner
for (const type of ["change", "input"]) {
  document.addEventListener(type, (e) => {
    if (!(e.target instanceof HTMLButtonElement)) trigger = null;
  }, true);
}
document.addEventListener("submit", (e) => {
  const form = e.target as HTMLFormElement;
  const sub = (e as SubmitEvent).submitter as HTMLElement | null || form.querySelector<HTMLElement>("[type=submit], button:not([type])");
  if (sub && isBusy(sub)) { e.preventDefault(); e.stopImmediatePropagation(); return; }
  if (sub) trigger = { el: sub, at: performance.now() };
}, true);

class ApiError extends Error {
  status: number;
  constructor(message: string, status: number) { super(message); this.status = status; }
}

/** A request replaced by a newer one with the same key (a search while typing): not an error, nothing to show.
 *  Its message is empty, so ``toast(errMsg(e))`` in any caller stays silent; buttons and lists ignore it. */
class SupersededError extends Error {
  constructor() { super(""); this.name = "SupersededError"; }
}

function isSuperseded(e: unknown): boolean {
  return e instanceof SupersededError;
}

/** What the server said, readable and short – never a stack trace or a page of HTML. */
function cleanMessage(text: string): string {
  const first = text.split(/\n\s*(?:Traceback|File ")/)[0].replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim();
  return first.length > 300 ? first.slice(0, 297) + " …" : first;
}

async function request<T>(method: string, url: string, body: unknown, opts: ApiOptions): Promise<T> {
  const ctrl = new AbortController();
  const write = method !== "GET" && method !== "HEAD";
  let timedOut = false;
  // the time limit covers everything: connecting, the headers AND the whole body – a proxy that stalls in the
  // middle of an answer must not leave a button spinning forever
  const timer = setTimeout(() => { timedOut = true; ctrl.abort(); }, opts.timeout ?? (write ? 90_000 : 30_000));
  if (opts.key) {
    latest.get(opts.key)?.abort();
    latest.set(opts.key, ctrl);
  }
  const current = (): boolean => !opts.key || latest.get(opts.key) === ctrl;
  try {
    let res: Response;
    let text: string;
    try {
      res = await fetch(url, {
        method, signal: ctrl.signal,
        // custom header = proof for the server that the request comes from this page (CSRF protection)
        headers: { "X-Requested-With": "Hoerwerk", ...(body ? { "Content-Type": "application/json" } : {}) },
        body: body ? JSON.stringify(body) : undefined,
      });
      text = await res.text();
    } catch {
      if (!current()) throw new SupersededError();
      throw new ApiError(timedOut ? t("Der Server antwortet nicht – bitte noch einmal versuchen")
        : t("Keine Verbindung zu Hörwerk – Netzwerk prüfen und noch einmal versuchen"), 0);
    }
    if (!current()) throw new SupersededError();          // a newer search answered meanwhile or is running
    let data = {} as T & { error?: string; login?: boolean };
    let parsed = !text;
    try { if (text) { data = JSON.parse(text) as typeof data; parsed = true; } } catch { /* not JSON: a proxy or the limit page */ }
    if (res.status === 401 && data.login && location.pathname !== "/login") location.replace("/login");
    if (!res.ok) {
      const fallback = res.status === 503 ? t("Hörwerk ist gerade ausgelastet – gleich noch einmal versuchen")
        : res.status === 502 || res.status === 504 ? t("Hörwerk ist gerade nicht erreichbar")
        : res.status === 421 ? cleanMessage(text) : `HTTP ${res.status}`;
      throw new ApiError(cleanMessage(data.error || "") || fallback, res.status);
    }
    // a 200 whose body is cut off or no JSON is no success
    if (!parsed) throw new ApiError(t("Die Antwort von Hörwerk war unvollständig – bitte noch einmal versuchen"), res.status);
    return data;
  } finally {
    clearTimeout(timer);
    if (opts.key && latest.get(opts.key) === ctrl) latest.delete(opts.key);
  }
}

async function api<T = unknown>(method: string, url: string, body?: unknown, opts: ApiOptions = {}): Promise<T> {
  const write = method !== "GET" && method !== "HEAD";
  // the same change twice at once (double click, Enter held down): one request, both get its answer.
  // Writes are never repeated automatically – whether to try again is the user's decision.
  const sig = write ? `${method} ${url} ${body === undefined ? "" : JSON.stringify(body)}` : "";
  const running = sig ? inflight.get(sig) : undefined;
  if (running) return running as Promise<T>;
  const p = request<T>(method, url, body, opts);
  if (sig) {
    inflight.set(sig, p);
    const done = (): void => { inflight.delete(sig); };
    p.then(done, done);
  }
  const btn = opts.button === false ? null : opts.button || (write ? currentTrigger() : null);
  if (btn) trackButton(btn, p, opts.busy);
  return p;
}

/** Text of a caught error (anything can be thrown in JavaScript). */
function errMsg(e: unknown): string {
  return cleanMessage(e instanceof Error ? e.message : String(e));
}

// -- buttons -------------------------------------------------------------------
// "Speichern" -> "Speichert …": the running action in the button's own words
const BUSY_WORDS: [string, string][] = [
  [t("Speichern"), t("Speichert …")], [t("Löschen"), t("Löscht …")], [t("Entfernen"), t("Entfernt …")],
  [t("Suchen"), t("Sucht …")], [t("Anfragen"), t("Fragt an …")], [t("Hinzufügen"), t("Fügt hinzu …")],
  [t("Erstellen"), t("Erstellt …")], [t("Übernehmen"), t("Übernimmt …")], [t("Testen"), t("Testet …")],
  [t("Senden"), t("Sendet …")], [t("Hochladen"), t("Lädt hoch …")], [t("Anmelden"), t("Meldet an …")],
  [t("Verbinden"), t("Verbindet …")], [t("Umwandeln"), t("Wandelt um …")], [t("Laden"), t("Lädt …")],
  [t("Erneut"), t("Startet neu …")], [t("Abbrechen"), t("Bricht ab …")], [t("Aktualisieren"), t("Aktualisiert …")],
  [t("Ablehnen"), t("Lehnt ab …")], [t("Genehmigen"), t("Genehmigt …")], [t("Einlesen"), t("Liest ein …")],
];

function busyText(btn: HTMLElement): string {
  const label = (btn.textContent || "").replace(/\s+/g, " ").trim().toLowerCase();
  if (!label) return "";
  const hit = BUSY_WORDS.find(([word]) => label.startsWith(word.toLowerCase()));
  return hit ? hit[1] : t("Bitte warten …");
}

const busyCount = new WeakMap<HTMLElement, number>();

/** Show a request's progress on its button (also usable for work that is no ``api()`` call). */
function trackButton(btn: HTMLElement, work: Promise<unknown>, label?: string): void {
  const n = (busyCount.get(btn) || 0) + 1;
  busyCount.set(btn, n);
  if (n === 1) setBusy(btn, true, label);
  const end = (ok: boolean | null): void => {      // null: replaced by a newer request – no green, no red
    const left = (busyCount.get(btn) || 1) - 1;
    busyCount.set(btn, left);
    if (left) return;
    setBusy(btn, false);
    if (ok !== null) flash(btn, ok);
  };
  work.then(() => end(true), (e: unknown) => end(isSuperseded(e) ? null : false));
}

function setBusy(btn: HTMLElement, on: boolean, label?: string): void {
  if (on) {
    const width = btn.getBoundingClientRect().width;
    if (width) btn.style.minWidth = `${width}px`;               // the button keeps its size
    btn.setAttribute("aria-busy", "true");
    btn.setAttribute("aria-disabled", "true");
    btn.classList.add("is-busy");
    const text = label || (btn.querySelector(".ico") && !(btn.textContent || "").trim() ? "" : busyText(btn));
    const spin = el("span", { class: "spinner", "aria-hidden": "true" });
    if (text) {
      btn.dataset.label = btn.innerHTML;                         // our own markup, restored as it was
      btn.replaceChildren(spin, el("span", { class: "busy-text" }, text));
    } else {
      btn.append(spin);                                          // icon buttons: the spinner covers the icon
    }
    btn.dataset.wasTitle = btn.getAttribute("aria-label") || "";
    if (text) btn.setAttribute("aria-label", text);
  } else {
    if (btn.dataset.label !== undefined) { btn.innerHTML = btn.dataset.label; delete btn.dataset.label; }
    btn.querySelector(":scope > .spinner")?.remove();
    btn.classList.remove("is-busy");
    btn.removeAttribute("aria-busy");
    btn.removeAttribute("aria-disabled");
    if (btn.dataset.wasTitle) btn.setAttribute("aria-label", btn.dataset.wasTitle); else btn.removeAttribute("aria-label");
    delete btn.dataset.wasTitle;
    btn.style.minWidth = "";
  }
}

function flash(btn: HTMLElement, ok: boolean): void {
  if (!btn.isConnected) return;
  const cls = ok ? "did-work" : "did-fail";
  btn.classList.remove("did-work", "did-fail");
  void btn.offsetWidth;                                          // restart the short animation
  btn.classList.add(cls);
  setTimeout(() => btn.classList.remove(cls), ok ? 1200 : 1600);
}

// -- lists ---------------------------------------------------------------------
/** Grey placeholders in the shape of what is coming (rows or cards) – only while nothing is there yet. */
/** An element that may sit in this container (li in lists, a full-width cell in tables, else a div). */
function slot(container: HTMLElement, attrs: Attrs, ...children: Child[]): HTMLElement {
  const tag = container.tagName;
  if (tag === "TBODY" || tag === "TABLE") return el("tr", attrs, el("td", { colspan: "99" }, ...children));
  return el(tag === "UL" || tag === "OL" ? "li" : "div", attrs, ...children);
}

function skeleton(container: HTMLElement, count = 4, kind: "row" | "card" | "line" = "row"): void {
  const item = (): HTMLElement => slot(container, { class: `skeleton skeleton-${kind}`, "aria-hidden": "true" },
    kind === "line" ? null : el("span", { class: "sk-thumb" }),
    el("span", { class: "sk-lines" }, el("span", { class: "sk-line" }), el("span", { class: "sk-line short" })));
  container.replaceChildren(...Array.from({ length: count }, item));
}

/** Load a list: skeleton the first time, the current content stays visible while it refreshes,
 *  a failure shows "Erneut versuchen" where the list would be (or a message when something is shown). */
async function loadList(container: HTMLElement, load: () => Promise<void>, opts: {
  count?: number; kind?: "row" | "card" | "line";
  /** something else is shown now (another section): placeholders instead of the old content */
  fresh?: boolean;
  /** a background refresh: a failure keeps the old content without a message (the next round tries again) */
  quiet?: boolean;
} = {}): Promise<boolean> {
  const empty = opts.fresh || !container.children.length || container.dataset.state === "error"
    || !!container.querySelector(":scope > .skeleton");
  if (empty) skeleton(container, opts.count, opts.kind);
  container.setAttribute("aria-busy", "true");
  try {
    await load();
    container.dataset.state = "ok";
    container.querySelectorAll(":scope > .skeleton").forEach((n) => n.remove());
    return true;
  } catch (e) {
    if (isSuperseded(e)) return false;              // a newer load of this list is on its way
    if (empty || container.querySelector(":scope > .skeleton")) {
      container.dataset.state = "error";
      const retry = el("button", { type: "button", class: "btn small" }, icon("refresh"), t("Erneut versuchen"));
      retry.addEventListener("click", () => { void loadList(container, load, opts); });
      container.replaceChildren(slot(container, { class: "load-error", role: "alert" },
        el("span", {}, t("Laden fehlgeschlagen: {error}", { error: errMsg(e) })), retry));
    } else if (!opts.quiet) {
      toast(t("Aktualisieren fehlgeschlagen: {error}", { error: errMsg(e) }), "warning");
    }
    return false;
  } finally {
    container.removeAttribute("aria-busy");
  }
}

/** A setting changed right where it matters (saved at once, the settings dialog shows the same value). */
function settingSelect(key: string, label: string, options: [string, string][]): HTMLLabelElement {
  const sel = el("select", { "data-setting": key }, options.map(([v, l]) => el("option", { value: v }, l)));
  const current = (s: Settings): void => { if (s[key] !== undefined) sel.value = String(s[key]); };
  api<Settings>("GET", "/api/settings").then(current).catch(() => { /* keeps the first option */ });
  sel.addEventListener("change", async () => {
    try {
      const s = await api<Settings>("PUT", "/api/settings", { [key]: sel.value });
      if (typeof state !== "undefined") state.settings = s;
      for (const other of document.querySelectorAll<HTMLSelectElement>(`select[data-setting="${key}"]`)) other.value = sel.value;
      toast(t("Gespeichert"));
    } catch (e) { toast(errMsg(e), true); }
  });
  return el("label", { class: "inline-setting" }, label, sel);
}

// -- messages ------------------------------------------------------------------
type ToastKind = "success" | "error" | "warning" | "info";
const TOAST_MS: Record<ToastKind, number> = { success: 3500, info: 5000, warning: 8000, error: 12000 };
const TOAST_MAX = 4;
const TOAST_ICON: Record<ToastKind, string> = { success: "check", info: "info", warning: "alert", error: "alert" };

function toastBox(): HTMLElement {
  let box = document.getElementById("toast");
  if (!box) {
    box = el("div", { id: "toast", class: "toasts", "aria-live": "polite" });
    document.body.append(box);
  }
  return box;
}

/** A message at the bottom; several stack in order. ``true`` = error (as before). */
function toast(msg: string, kind: boolean | ToastKind = "success"): void {
  const type: ToastKind = kind === true ? "error" : kind === false ? "success" : kind;
  const text = cleanMessage(msg);
  if (!text) return;
  const box = toastBox();
  for (const old of Array.from(box.children)) if ((old as HTMLElement).dataset.text === text) old.remove();   // no twins
  const item = el("div", { class: `toast-item ${type}`, role: type === "error" ? "alert" : "status", "data-text": text },
    icon(TOAST_ICON[type]), el("span", { class: "toast-text" }, text));
  let timer = 0;
  const close = (): void => {
    clearTimeout(timer);
    item.classList.add("leaving");
    setTimeout(() => item.remove(), 180);
  };
  const x = el("button", { type: "button", class: "toast-x", title: t("Schließen"), "aria-label": t("Meldung schließen") }, icon("x"));
  x.addEventListener("click", close);
  item.append(x);
  const start = (): void => { clearTimeout(timer); timer = setTimeout(close, TOAST_MS[type]); };
  item.addEventListener("mouseenter", () => clearTimeout(timer));     // reading: stays
  item.addEventListener("mouseleave", start);
  item.addEventListener("focusin", () => clearTimeout(timer));
  item.addEventListener("focusout", start);
  box.append(item);
  const items = Array.from(box.children) as HTMLElement[];
  for (const extra of items.slice(0, Math.max(0, items.length - TOAST_MAX))) extra.remove();   // the oldest go first
  start();
}

function confirmBox(text: string, okLabel = "OK"): Promise<boolean> {
  const before = trigger;                     // the button that asked – it shows the progress afterwards
  return new Promise((resolve) => {
    const d = $<HTMLDialogElement>("#confirmDialog");
    $("#confirmText").textContent = text;
    $("#confirmOk").textContent = okLabel;
    d.returnValue = "";
    d.addEventListener("close", () => {
      trigger = before && before.el.isConnected ? { el: before.el, at: performance.now() } : null;
      resolve(d.returnValue === "ok");
    }, { once: true });
    d.showModal();
  });
}

function fmtDuration(sec: number | null | undefined): string {
  if (!sec) return "–";
  const h = Math.floor(sec / 3600), m = Math.round((sec % 3600) / 60);
  return h ? `${h} h ${String(m).padStart(2, "0")} min` : `${m} min`;
}

/** Indexers whose daily search limit is used up – normal for small accounts, they are back at midnight. */
function restingText(names: string[]): string {
  return t("{names}: Tageslimit erreicht – sucht ab Mitternacht weiter", { names: names.join(", ") });
}

function fmtAgo(ts: number | null | undefined): string {
  if (!ts) return t("nie");
  const s = Math.round(Date.now() / 1000 - ts);
  if (s < 60) return t("gerade eben");
  if (s < 3600) return t("vor {n} min", { n: Math.round(s / 60) });
  return new Date(ts * 1000).toLocaleString(LOCALE, { dateStyle: "short", timeStyle: "short" });
}

const SOURCE_NAMES: Record<string, string> = {
  audible: "Audible", audnexus: "Audnexus", bookbeat: "BookBeat", thalia: "Thalia",
  apple: "Apple Books", openlibrary: "Open Library",
};

translateDom(document.body);

// ------------------------------------------------------------------------- //
// Help behind a small "?": explanations are there for whoever needs them, out of the way for everyone else.
// Short hints ("Stunden", "kommagetrennt") stay visible on the label's own line.
// ------------------------------------------------------------------------- //
const SHORT_HINT = 26;

function helpToggle(content: Node[]): [HTMLButtonElement, HTMLElement] {
  const tip = el("span", { class: "help-tip", hidden: true }, ...content);
  const btn = el("button", { type: "button", class: "help-q", "aria-label": t("Erklärung"), "aria-expanded": "false",
    title: t("Erklärung") }, "?");
  btn.addEventListener("click", (e) => {
    e.preventDefault();
    e.stopPropagation();
    tip.hidden = !tip.hidden;
    btn.setAttribute("aria-expanded", String(!tip.hidden));
  });
  return [btn, tip];
}

/** The text of a label (everything before its field) on one line, with its hint or its "?" next to it. */
function labelLine(label: HTMLElement): HTMLElement {
  const existing = label.querySelector<HTMLElement>(":scope > .label-line");
  if (existing) return existing;
  const line = el("span", { class: "label-line" });
  const lead: ChildNode[] = [];
  for (const n of Array.from(label.childNodes)) {
    if (n instanceof HTMLElement && n.matches("input, select, textarea, div, button:not(.help-q), .seg, .row")) break;
    lead.push(n);
  }
  label.insertBefore(line, lead[0] || label.firstChild);
  for (const n of lead) line.append(n);
  return line;
}

function initHelp(root: ParentNode = document.body): void {
  for (const small of Array.from(root.querySelectorAll<HTMLElement>("label:not(.check) > small"))) {
    const label = small.parentElement as HTMLElement;
    const line = labelLine(label);
    if ((small.textContent || "").trim().length <= SHORT_HINT) {
      small.classList.add("hint");                       // a unit or a format: stays visible, same line
      continue;
    }
    const [btn, tip] = helpToggle(Array.from(small.childNodes));
    small.replaceWith(btn);
    line.after(tip);
  }
  // the introduction of a section: behind a "?" at its heading
  for (const legend of Array.from(root.querySelectorAll<HTMLElement>("fieldset > legend"))) {
    const p = legend.nextElementSibling as HTMLElement | null;
    if (!p || !p.matches("p.muted.small") || p.id || p.dataset.keep !== undefined) continue;
    const [btn, tip] = helpToggle(Array.from(p.childNodes));
    tip.classList.add("help-block");
    legend.append(btn);
    p.replaceWith(tip);
  }
}

initHelp();

for (const btn of document.querySelectorAll<HTMLElement>("dialog [data-close]")) {
  btn.addEventListener("click", () => btn.closest("dialog")?.close());
}

// Every window gets a close button at the top – on a phone the buttons at the bottom can be out of reach.
// It does what the window's own "Abbrechen"/"Schließen" does (e.g. stops a running upload).
for (const d of document.querySelectorAll<HTMLDialogElement>("dialog")) {
  const x = el("button", { type: "button", class: "dialog-x", title: t("Schließen"), "aria-label": t("Schließen") }, icon("x"));
  x.addEventListener("click", () => {
    const own = d.querySelector<HTMLButtonElement>("#uploadCancel, [data-close], button[value=cancel]");
    if (own) own.click(); else d.close("cancel");
  });
  d.prepend(x);
}

// ------------------------------------------------------------------------- //
// Password rules – the same as on the server (accounts.password_problems), shown as a live checklist
// ------------------------------------------------------------------------- //
const PW_MIN = 12;
const PW_PHRASE = 20;
const PW_OBVIOUS = ["hörwerk", "hoerwerk", "horwerk", "passwort", "password", "kennwort", "123456", "qwertz", "qwerty",
  "asdfgh", "hörbuch", "hoerbuch", "audiobook"];

function passwordRules(pw: string, name = ""): { text: string; ok: boolean }[] {
  const low = pw.toLowerCase();
  const kinds = [/[a-zäöüß]/, /[A-ZÄÖÜ]/, /\d/, /[^\p{L}\p{N}\s]|_|\s/u].filter((rx) => rx.test(pw)).length;
  const clean = name.trim().toLowerCase();
  return [
    { text: t("mindestens {n} Zeichen", { n: PW_MIN }), ok: pw.length >= PW_MIN },
    { text: t("drei von vier: Kleinbuchstaben, Großbuchstaben, Ziffern, Sonderzeichen – oder ein Satz ab {n} Zeichen", { n: PW_PHRASE }),
      ok: kinds >= 3 || pw.length >= PW_PHRASE },
    { text: t("nicht der eigene Name"), ok: clean.length < 3 || !low.includes(clean) },
    { text: t("nicht „Hörwerk“, „Passwort“, „123456“ oder Ähnliches"), ok: !PW_OBVIOUS.some((w) => low.includes(w)) },
    { text: t("nicht nur ein Wort mit Zahlen oder Zeichen drumherum"), ok: !/^[^\p{L}]*\p{L}{1,10}[^\p{L}]*$/u.test(pw.trim()) },
    { text: t("nicht immer dieselben Zeichen"), ok: new Set(low).size >= 6 },
  ];
}

/** A checklist under a new-password field that ticks off the rules while typing. */
function attachPasswordRules(input: HTMLInputElement, name: () => string = () => ""): void {
  const list = el("ul", { class: "pw-rules", "aria-live": "polite" });
  const hint = el("p", { class: "pw-hint muted small" },
    t("Tipp: Ein Passwort-Manager erzeugt sichere Passwörter. Oder ein Satz aus vier Wörtern, z. B. „Kaffee Garten Hund Fahrrad“."));
  const render = (): void => {
    list.replaceChildren(...passwordRules(input.value, name()).map((r) =>
      el("li", { class: r.ok ? "ok" : "" }, icon(r.ok ? "check" : "x"), r.text)));
    list.hidden = !input.value;
  };
  input.addEventListener("input", render);
  input.setAttribute("passwordrules", `minlength: ${PW_MIN}; required: lower; required: upper; required: digit; required: [-_.!];`);
  // below the whole group of fields (in a two-column grid the list would squeeze into one cell)
  (input.closest(".grid2") || input.closest("label") || input).after(hint, list);
  render();
}
