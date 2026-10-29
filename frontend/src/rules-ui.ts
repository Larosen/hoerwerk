"use strict";
// Rule definitions for the editor. The logic lives in app/rules.py; keys here
// must match the fields read there. `show(rule)` hides fields that don't apply.

interface FieldDef {
  k: string; label: string; type: "text" | "number" | "select" | "check" | "textarea" | "datalist";
  options?: [string, string][] | string[]; grow?: boolean; size?: number; placeholder?: string;
  show?: (r: Rule) => boolean;
}
interface RuleType {
  label: string; summary?: (r: Rule) => string; defaults: Record<string, unknown>; fields: FieldDef[]; hint?: string;
}

const TARGET: FieldDef = { k: "target", label: "in", type: "select", options: [["stem", "Name"], ["ext", "Endung"], ["all", "Name + Endung"]] };
const CASE_SENS: FieldDef = { k: "case_sensitive", label: "Groß/klein", type: "check" };
const REGEX: FieldDef = { k: "regex", label: "Regex", type: "check" };

const TEMPLATE_HINT =
  "Platzhalter: <code>{name}</code> <code>{original}</code> <code>{ext}</code> <code>{n}</code> <code>{n:3}</code> <code>{folder_n}</code> " +
  "<code>{folder}</code> <code>{parent}</code> <code>{modified:%Y-%m-%d}</code> <code>{created:%Y}</code> <code>{taken:%Y-%m-%d %H.%M}</code> <code>{size}</code> · " +
  "Audio: <code>{title}</code> <code>{track:2}</code> <code>{disc}</code> <code>{artist}</code> <code>{album}</code> <code>{album_artist}</code> <code>{year}</code> <code>{genre}</code> <code>{duration}</code> · " +
  "Video: <code>{resolution}</code> <code>{width}</code> <code>{height}</code> <code>{codec}</code> <code>{show}</code> <code>{season:2}</code> <code>{episode:2}</code> · " +
  "Foto: <code>{camera}</code> <code>{make}</code> <code>{model}</code> <code>{lens}</code> <code>{iso}</code> <code>{dimensions}</code> · " +
  "Formate: <code>{artist:upper}</code> <code>{title:ascii}</code> <code>{name:0-20}</code>. Das ⓘ in der Vorschau zeigt alle Werte einer Datei.";

const RULE_GROUPS: [string, string[]][] = [
  ["Text", ["replace", "insert", "remove", "case", "truncate"]],
  ["Zeichen", ["cleanup", "remove_chars"]],
  ["Position", ["move", "swap"]],
  ["Nummern", ["number", "pad_numbers", "shift_numbers"]],
  ["Datum & Metadaten", ["date", "template", "list"]],
  ["Endung", ["extension"]],
];

const RULE_TYPES: Record<string, RuleType> = {
  replace: {
    label: "Text ersetzen",
    summary: (r) => r.find ? `„${r.find}“ → „${r.replace || ""}“` : "",
    defaults: { find: "", replace: "", target: "stem", regex: false, case_sensitive: false, whole_word: false, occurrence: "all" },
    fields: [
      { k: "find", label: "Suchen", type: "text", grow: true },
      { k: "replace", label: "Ersetzen durch", type: "text", grow: true },
      { k: "occurrence", label: "Vorkommen", type: "select", options: [["all", "alle"], ["first", "erstes"], ["last", "letztes"]] },
      TARGET, REGEX, CASE_SENS,
      { k: "whole_word", label: "Ganzes Wort", type: "check" },
    ],
    hint: "Regex-Gruppen mit <code>\\1</code> oder <code>$1</code>, z. B. <code>^(\\d+)\\s*</code> → <code>Kapitel $1 - </code>",
  },
  insert: {
    label: "Text einfügen",
    summary: (r) => r.text ? `„${r.text}“` : "",
    defaults: { text: "", position: "start", index: 0, from_end: false, anchor: "" },
    fields: [
      { k: "text", label: "Text", type: "text", grow: true },
      { k: "position", label: "Position", type: "select", options: [["start", "am Anfang"], ["end", "am Ende"], ["index", "an Stelle"], ["before", "vor Text"], ["after", "nach Text"]] },
      { k: "index", label: "Stelle", type: "number", show: (r) => r.position === "index" },
      { k: "from_end", label: "von hinten", type: "check", show: (r) => r.position === "index" },
      { k: "anchor", label: "Text", type: "text", show: (r) => r.position === "before" || r.position === "after" },
      { ...REGEX, show: (r) => r.position === "before" || r.position === "after" },
    ],
    hint: "Im Text sind Platzhalter wie bei „Vorlage“ erlaubt, z. B. <code>{year} - </code>.",
  },
  remove: {
    label: "Bereich entfernen",
    summary: (r) => r.mode === "range" ? `${r.count} Zeichen ab ${r.start}` : `${r.mode === "before" ? "vor" : "nach"} „${r.anchor || ""}“`,
    defaults: { mode: "range", start: 0, count: 1, from_end: false, anchor: "", keep_anchor: false, last: false },
    fields: [
      { k: "mode", label: "Was", type: "select", options: [["range", "Zeichenbereich"], ["before", "alles vor Text"], ["after", "alles nach Text"]] },
      { k: "start", label: "ab Stelle", type: "number", show: (r) => r.mode === "range" },
      { k: "count", label: "Anzahl", type: "number", show: (r) => r.mode === "range" },
      { k: "from_end", label: "von hinten", type: "check", show: (r) => r.mode === "range" },
      { k: "anchor", label: "Text", type: "text", show: (r) => r.mode !== "range" },
      { k: "keep_anchor", label: "Text behalten", type: "check", show: (r) => r.mode !== "range" },
      { k: "last", label: "letztes Vorkommen", type: "check", show: (r) => r.mode !== "range" },
      { ...REGEX, show: (r) => r.mode !== "range" },
    ],
  },
  case: {
    label: "Groß-/Kleinschreibung",
    summary: (r) => ({ title: "Titel", words: "Jedes Wort", sentence: "Satz", upper: "GROSS", lower: "klein", swap: "umkehren" } as Record<string, string>)[r.mode] ?? "",
    defaults: { mode: "title", target: "stem", small_words: true, keep: "" },
    fields: [
      { k: "mode", label: "Modus", type: "select", options: [["title", "Titel (Füllwörter klein)"], ["words", "Jedes Wort groß"], ["sentence", "Satzanfang groß"], ["upper", "GROSS"], ["lower", "klein"], ["swap", "uMKEHREN"]] },
      TARGET,
      { k: "keep", label: "Schreibweise beibehalten", type: "text", grow: true, placeholder: "MP3, DJ, McCartney" },
    ],
    hint: "Titel-Modus schreibt Füllwörter (der, und, of, the …) klein, außer am Anfang und Ende.",
  },
  truncate: {
    label: "Kürzen",
    summary: (r) => `max. ${r.length} Zeichen`,
    defaults: { length: 60, keep: "start", word_boundary: true },
    fields: [
      { k: "length", label: "Max. Zeichen", type: "number" },
      { k: "keep", label: "Behalten", type: "select", options: [["start", "Anfang"], ["end", "Ende"]] },
      { k: "word_boundary", label: "an Wortgrenze", type: "check", show: (r) => r.keep !== "end" },
    ],
  },
  cleanup: {
    label: "Aufräumen",
    summary: () => "",
    defaults: { underscores: true, dots: false, percent: false, dashes: false, strip_brackets: false, strip_digits_start: false, ascii: false, windows: false, collapse: true, trim: true },
    fields: [
      { k: "underscores", label: "_ → Leerzeichen", type: "check" },
      { k: "dots", label: ". → Leerzeichen", type: "check" },
      { k: "percent", label: "%20/+ → Leerzeichen", type: "check" },
      { k: "dashes", label: "„ - “ vereinheitlichen", type: "check" },
      { k: "strip_brackets", label: "(Klammern) entfernen", type: "check" },
      { k: "strip_digits_start", label: "Ziffern am Anfang weg", type: "check" },
      { k: "ascii", label: "Umlaute → ae/oe/ue", type: "check" },
      { k: "windows", label: "Windows/SMB-sicher", type: "check" },
      { k: "collapse", label: "Doppelte Leerzeichen", type: "check" },
      { k: "trim", label: "Ränder trimmen", type: "check" },
    ],
    hint: "„Windows/SMB-sicher“ ersetzt <code>&lt; &gt; : \" \\ | ? *</code> – wichtig, wenn die Unraid-Freigabe von Windows oder macOS aus genutzt wird.",
  },
  remove_chars: {
    label: "Zeichen/Wörter entfernen",
    summary: (r) => [r.digits && "Ziffern", r.punctuation && "Satzzeichen", r.chars && `„${r.chars}“`, r.words && r.words].filter(Boolean).join(", "),
    defaults: { digits: false, letters: false, spaces: false, punctuation: false, non_alnum: false, non_ascii: false, chars: "", words: "", target: "stem" },
    fields: [
      { k: "digits", label: "Ziffern", type: "check" },
      { k: "letters", label: "Buchstaben", type: "check" },
      { k: "spaces", label: "Leerzeichen", type: "check" },
      { k: "punctuation", label: "Satzzeichen", type: "check" },
      { k: "non_alnum", label: "alles außer Buchst./Ziffern", type: "check" },
      { k: "non_ascii", label: "Nicht-ASCII", type: "check" },
      { k: "chars", label: "Diese Zeichen", type: "text", size: 10 },
      { k: "words", label: "Wörter (Komma-getrennt)", type: "text", grow: true, placeholder: "1080p, x264, WEB-DL" },
      TARGET,
    ],
  },
  move: {
    label: "Text verschieben",
    summary: (r) => r.find ? `„${r.find}“ → ${r.to === "end" ? "Ende" : "Anfang"}` : "",
    defaults: { find: "", to: "start", separator: " ", regex: false },
    fields: [
      { k: "find", label: "Text", type: "text", grow: true },
      { k: "to", label: "nach", type: "select", options: [["start", "vorne"], ["end", "hinten"]] },
      { k: "separator", label: "Trenner", type: "text", size: 4 },
      REGEX,
    ],
    hint: "Beispiel: Regex <code>\\(\\d{4}\\)</code> nach vorne verschiebt die Jahreszahl in Klammern an den Anfang.",
  },
  swap: {
    label: "Teile tauschen",
    summary: (r) => `um „${r.separator}“`,
    defaults: { separator: " - ", occurrence: "first", joiner: "" },
    fields: [
      { k: "separator", label: "Trenner", type: "text", size: 6 },
      { k: "occurrence", label: "am", type: "select", options: [["first", "ersten"], ["last", "letzten"]] },
      { k: "joiner", label: "neuer Trenner", type: "text", size: 6, placeholder: "gleich" },
    ],
    hint: "„Künstler - Titel“ → „Titel - Künstler“.",
  },
  number: {
    label: "Nummerieren",
    summary: (r) => `ab ${r.start}`,
    defaults: { start: 1, step: 1, padding: 2, separator: " - ", position: "start", format: "decimal", per_folder: false, prefix: "", suffix: "", index: 0 },
    fields: [
      { k: "position", label: "Position", type: "select", options: [["start", "vorne"], ["end", "hinten"], ["index", "an Stelle"], ["replace", "ganzer Name"]] },
      { k: "index", label: "Stelle", type: "number", show: (r) => r.position === "index" },
      { k: "format", label: "Format", type: "select", options: [["decimal", "1, 2, 3"], ["alpha", "a, b, c"], ["ALPHA", "A, B, C"], ["roman", "I, II, III"], ["roman_lower", "i, ii, iii"]] },
      { k: "start", label: "Start", type: "number" },
      { k: "step", label: "Schritt", type: "number" },
      { k: "padding", label: "Stellen", type: "number", show: (r) => r.format === "decimal" },
      { k: "separator", label: "Trenner", type: "text", size: 5, show: (r) => r.position === "start" || r.position === "end" },
      { k: "prefix", label: "davor", type: "text", size: 6 },
      { k: "suffix", label: "danach", type: "text", size: 6 },
      { k: "per_folder", label: "pro Ordner neu", type: "check" },
    ],
    hint: "Reihenfolge = Sortierung der Vorschau (nur ausgewählte Einträge).",
  },
  pad_numbers: {
    label: "Zahlen auffüllen",
    summary: (r) => `${r.width} Stellen`,
    defaults: { width: 2, which: "all", strip_zeros: true },
    fields: [
      { k: "width", label: "Stellen", type: "number" },
      { k: "which", label: "Zahlen", type: "select", options: [["all", "alle"], ["first", "erste"], ["last", "letzte"]] },
      { k: "strip_zeros", label: "zu viele Nullen kürzen", type: "check" },
    ],
    hint: "„Track 1“ → „Track 01“, damit die Sortierung stimmt.",
  },
  shift_numbers: {
    label: "Zahlen verschieben",
    summary: (r) => (r.offset >= 0 ? "+" : "") + r.offset,
    defaults: { offset: 1, which: "first" },
    fields: [
      { k: "offset", label: "um", type: "number" },
      { k: "which", label: "Zahlen", type: "select", options: [["first", "erste"], ["last", "letzte"], ["all", "alle"]] },
    ],
    hint: "Beispiel: Folgen 0–9 auf 1–10 umnummerieren. Führende Nullen bleiben erhalten.",
  },
  date: {
    label: "Datum einfügen",
    summary: (r) => ({ modified: "Änderung", created: "Erstellung", taken: "Aufnahme", now: "heute" } as Record<string, string>)[r.source] ?? "",
    defaults: { source: "taken", format: "%Y-%m-%d", position: "start", separator: " ", fallback: true },
    fields: [
      { k: "source", label: "Datum", type: "select", options: [["taken", "Aufnahme (EXIF/Video/ID3)"], ["modified", "Änderungsdatum"], ["created", "Erstelldatum"], ["now", "heute"]] },
      { k: "format", label: "Format", type: "datalist", size: 14, options: ["%Y-%m-%d", "%Y-%m-%d %H.%M.%S", "%Y%m%d_%H%M%S", "%d.%m.%Y", "%Y", "%Y-%m", "%B %Y"] },
      { k: "position", label: "Position", type: "select", options: [["start", "vorne"], ["end", "hinten"], ["replace", "ganzer Name"]] },
      { k: "separator", label: "Trenner", type: "text", size: 4, show: (r) => r.position !== "replace" },
      { k: "fallback", label: "sonst Änderungsdatum", type: "check", show: (r) => r.source === "taken" },
    ],
    hint: "<code>%Y</code> Jahr · <code>%m</code> Monat · <code>%d</code> Tag · <code>%H</code> Stunde · <code>%M</code> Minute · <code>%S</code> Sekunde · <code>%B</code> Monatsname",
  },
  template: {
    label: "Vorlage / Metadaten",
    summary: (r) => r.template,
    defaults: { template: "{track:2} - {title}" },
    fields: [{ k: "template", label: "Vorlage", type: "text", grow: true }],
    hint: TEMPLATE_HINT + " Dateien ohne die verwendeten Tags bleiben unverändert.",
  },
  list: {
    label: "Aus Liste",
    summary: (r) => `${String(r.lines || "").split("\n").filter((l: string) => l.trim()).length} Zeilen`,
    defaults: { lines: "", mode: "replace", separator: " - " },
    fields: [
      { k: "mode", label: "Zeile wird", type: "select", options: [["replace", "neuer Name"], ["with_ext", "Name inkl. Endung"], ["prefix", "vorangestellt"], ["suffix", "angehängt"]] },
      { k: "separator", label: "Trenner", type: "text", size: 5, show: (r) => r.mode === "prefix" || r.mode === "suffix" },
      { k: "lines", label: "Liste (eine Zeile pro Datei)", type: "textarea", grow: true },
    ],
    hint: "Zeile 1 gilt für den ersten ausgewählten Eintrag in der Vorschau-Reihenfolge, Zeile 2 für den zweiten …",
  },
  extension: {
    label: "Dateiendung",
    summary: (r) => ({ lower: "klein", upper: "GROSS", set: `→ .${r.value}`, append: `+ .${r.value}`, normalize: "vereinheitlichen", remove: "entfernen" } as Record<string, string>)[r.mode] ?? "",
    defaults: { mode: "lower", value: "" },
    fields: [
      { k: "mode", label: "Aktion", type: "select", options: [["lower", "klein"], ["upper", "GROSS"], ["normalize", "vereinheitlichen (jpeg→jpg)"], ["set", "ersetzen durch"], ["append", "anhängen"], ["remove", "entfernen"]] },
      { k: "value", label: "Endung", type: "text", size: 6, show: (r) => r.mode === "set" || r.mode === "append" },
    ],
  },
};

const CONDITION_FIELDS: FieldDef[] = [
  { k: "only_mode", label: "Nur wenn Name", type: "select", options: [["contains", "enthält"], ["regex", "passt auf Regex"], ["ext", "Endung ist"]] },
  { k: "only", label: "", type: "text", grow: true, placeholder: "leer = immer anwenden" },
  { k: "only_invert", label: "umkehren (nicht)", type: "check" },
];
