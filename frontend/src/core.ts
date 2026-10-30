"use strict";
// Shared types of the API answers and the small helpers every page script uses.
// Loaded right after theme.js; the other scripts share these globals (classic scripts, no bundler).

// ------------------------------------------------------------------------- //
// Language: German is the source, /i18n.js brings the catalog of the chosen language
// ------------------------------------------------------------------------- //
declare const I18N: { lang: string; text: Record<string, string> } | undefined;
const LANG = typeof I18N === "undefined" ? "de" : I18N.lang;
const LOCALE = LANG === "en" ? "en-GB" : "de-DE";

/** Translate a German UI text; ``{name}`` placeholders are filled from ``values``. */
function t(text: string, values: Record<string, string | number> = {}): string {
  let out = (typeof I18N !== "undefined" && I18N.text[text]) || text;
  for (const [k, v] of Object.entries(values)) out = out.split("{" + k + "}").join(String(v));
  return out;
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

interface FileEntry { name: string; size: number; mtime: number; dir: boolean; kind: string }
interface Folder {
  path: string; parent: string | null; dirs: { name: string }[]; files: FileEntry[];
  truncated: boolean; mp3_count: number; busy: boolean;
}
interface PreviewItem {
  old: string; new: string; changed: boolean; error: string | null; dir: boolean;
  resolved?: boolean; sidecar?: boolean; of?: string;
}
/** A rename rule: ``type`` plus the fields its editor defines (see rules-ui.ts). */
interface Rule { type: string; enabled?: boolean; collapsed?: boolean; only?: string; [field: string]: any }
interface HistoryEntry { id: string; time: number; folder: string; count: number; sample: [string, string][] }

type ConvertStatus = "queued" | "running" | "done" | "error" | "cancelled";
interface ConvertJob {
  id: string; folder: string; reason: string; status: ConvertStatus; progress: number; message: string;
  output: string | null; created: number; finished: number | null; log: string[];
}

interface Book {
  id?: string; asin: string | null; title: string; subtitle?: string | null; authors: string[]; narrators?: string[];
  series?: string | null; series_position?: string | null; runtime_min?: number; release_date?: string | null;
  cover_url?: string | null; match?: { score: number }; region?: string | null; url?: string | null;
  format_type?: string | null; publisher?: string | null; language?: string | null; genres?: string[];
  description?: string; source?: string; isbn?: string | null; owned?: Owned | null;
}
interface Owned { path: string; title: string; authors: string[]; why: string; count: number }
interface Hints { query?: string; title?: string; author?: string; narrator?: string; folder?: string }
interface Duplicate { path: string; title: string; authors: string[]; asin: string | null; format: string; why: string }
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
  last_scan: number | null; scan_note: string; waiting: { name: string; since: number }[];
  jobs: AudiobookJob[]; review: number; queues?: { encode: QueueStat; io: QueueStat };
}
interface LibraryStats { books: number; m4b: number; other: number; last_scan: number | null; scanning: boolean }
interface UnconvertedBook {
  path: string; title: string; authors: string[]; series: string | null; series_position: string | null;
  files: number; format: string; job: { id: string; status: BookStatus } | null;
}
interface Track { file: string; duration: number; auto: string; filename: string; tag: string }

interface Indexer { id?: string; name?: string; url?: string; categories?: string; enabled?: boolean; apikey_set?: boolean }
interface AbsConfig { url: string; library: string; library_name: string; token_set: boolean; configured: boolean; last: string | null }
interface Sources {
  indexers: Indexer[]; sab: { url?: string; category?: string; apikey_set?: boolean };
  configured: { indexers: number; sab: boolean }; abs?: AbsConfig;
}
interface Release {
  id: string; title: string; size: number; published: number | null; grabs: number | null; indexer: string;
  also?: string[]; details: string | null; format: string | null; language: string | null; bitrate: number | null;
  abridged: boolean; drama: boolean; owned: Owned | null;
}
interface Download {
  name: string; status: string; progress: number; done: boolean; size: string; left: string;
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

const collator = new Intl.Collator(LANG, { numeric: true, sensitivity: "base" });

async function api<T = unknown>(method: string, url: string, body?: unknown): Promise<T> {
  const res = await fetch(url, {
    method,
    // custom header = proof for the server that the request comes from this page (CSRF protection)
    headers: { "X-Requested-With": "BetterRenamer", ...(body ? { "Content-Type": "application/json" } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => ({})) as T & { error?: string };
  if (!res.ok) throw new Error(data.error || `HTTP ${res.status}`);
  return data;
}

/** Text of a caught error (anything can be thrown in JavaScript). */
function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

let toastTimer: number | undefined;
function toast(msg: string, isError = false): void {
  const t = $("#toast");
  t.textContent = msg;
  t.className = "toast show" + (isError ? " error" : "");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.className = "toast"), isError ? 6000 : 3000);
}

function confirmBox(text: string, okLabel = "OK"): Promise<boolean> {
  return new Promise((resolve) => {
    const d = $<HTMLDialogElement>("#confirmDialog");
    $("#confirmText").textContent = text;
    $("#confirmOk").textContent = okLabel;
    d.returnValue = "";
    d.addEventListener("close", () => resolve(d.returnValue === "ok"), { once: true });
    d.showModal();
  });
}

function fmtDuration(sec: number | null | undefined): string {
  if (!sec) return "–";
  const h = Math.floor(sec / 3600), m = Math.round((sec % 3600) / 60);
  return h ? `${h} h ${String(m).padStart(2, "0")} min` : `${m} min`;
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

for (const btn of document.querySelectorAll<HTMLElement>("dialog [data-close]")) {
  btn.addEventListener("click", () => btn.closest("dialog")?.close());
}
