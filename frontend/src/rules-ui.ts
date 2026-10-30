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

const TARGET: FieldDef = { k: "target", label: t("in"), type: "select", options: [["stem", t("Name")], ["ext", t("Endung")], ["all", t("Name + Endung")]] };
const CASE_SENS: FieldDef = { k: "case_sensitive", label: t("Groß/klein"), type: "check" };
const REGEX: FieldDef = { k: "regex", label: t("Regex"), type: "check" };

const TEMPLATE_HINT = t("Platzhalter: <code>{name}</code> <code>{original}</code> <code>{ext}</code> <code>{n}</code> <code>{n:3}</code> <code>{folder_n}</code> <code>{folder}</code> <code>{parent}</code> <code>{modified:%Y-%m-%d}</code> <code>{created:%Y}</code> <code>{taken:%Y-%m-%d %H.%M}</code> <code>{size}</code> · Audio: <code>{title}</code> <code>{track:2}</code> <code>{disc}</code> <code>{artist}</code> <code>{album}</code> <code>{album_artist}</code> <code>{year}</code> <code>{genre}</code> <code>{duration}</code> · Video: <code>{resolution}</code> <code>{width}</code> <code>{height}</code> <code>{codec}</code> <code>{show}</code> <code>{season:2}</code> <code>{episode:2}</code> · Foto: <code>{camera}</code> <code>{make}</code> <code>{model}</code> <code>{lens}</code> <code>{iso}</code> <code>{dimensions}</code> · Formate: <code>{artist:upper}</code> <code>{title:ascii}</code> <code>{name:0-20}</code>. Das ⓘ in der Vorschau zeigt alle Werte einer Datei.");

const RULE_GROUPS: [string, string[]][] = [
  [t("Text"), ["replace", "insert", "remove", "case", "truncate"]],
  [t("Zeichen"), ["cleanup", "remove_chars"]],
  [t("Position"), ["move", "swap"]],
  [t("Nummern"), ["number", "pad_numbers", "shift_numbers"]],
  [t("Datum & Metadaten"), ["date", "template", "list"]],
  [t("Endung"), ["extension"]],
];

const RULE_TYPES: Record<string, RuleType> = {
  replace: {
    label: t("Text ersetzen"),
    summary: (r) => r.find ? `„${r.find}“ → „${r.replace || ""}“` : "",
    defaults: { find: "", replace: "", target: "stem", regex: false, case_sensitive: false, whole_word: false, occurrence: "all" },
    fields: [
      { k: "find", label: t("Suchen"), type: "text", grow: true },
      { k: "replace", label: t("Ersetzen durch"), type: "text", grow: true },
      { k: "occurrence", label: t("Vorkommen"), type: "select", options: [["all", t("alle")], ["first", t("erstes")], ["last", t("letztes")]] },
      TARGET, REGEX, CASE_SENS,
      { k: "whole_word", label: t("Ganzes Wort"), type: "check" },
    ],
    hint: t("Regex-Gruppen mit <code>\\1</code> oder <code>$1</code>, z. B. <code>^(\\d+)\\s*</code> → <code>Kapitel $1 - </code>"),
  },
  insert: {
    label: t("Text einfügen"),
    summary: (r) => r.text ? `„${r.text}“` : "",
    defaults: { text: "", position: "start", index: 0, from_end: false, anchor: "" },
    fields: [
      { k: "text", label: t("Text"), type: "text", grow: true },
      { k: "position", label: t("Position"), type: "select", options: [["start", t("am Anfang")], ["end", t("am Ende")], ["index", t("an Stelle")], ["before", t("vor Text")], ["after", t("nach Text")]] },
      { k: "index", label: t("Stelle"), type: "number", show: (r) => r.position === "index" },
      { k: "from_end", label: t("von hinten"), type: "check", show: (r) => r.position === "index" },
      { k: "anchor", label: t("Text"), type: "text", show: (r) => r.position === "before" || r.position === "after" },
      { ...REGEX, show: (r) => r.position === "before" || r.position === "after" },
    ],
    hint: t("Im Text sind Platzhalter wie bei „Vorlage“ erlaubt, z. B. <code>{year} - </code>."),
  },
  remove: {
    label: t("Bereich entfernen"),
    summary: (r) => r.mode === "range" ? t("{count} Zeichen ab {start}", { count: r.count, start: r.start })
      : r.mode === "before" ? t("vor „{text}“", { text: r.anchor || "" }) : t("nach „{text}“", { text: r.anchor || "" }),
    defaults: { mode: "range", start: 0, count: 1, from_end: false, anchor: "", keep_anchor: false, last: false },
    fields: [
      { k: "mode", label: t("Was"), type: "select", options: [["range", t("Zeichenbereich")], ["before", t("alles vor Text")], ["after", t("alles nach Text")]] },
      { k: "start", label: t("ab Stelle"), type: "number", show: (r) => r.mode === "range" },
      { k: "count", label: t("Anzahl"), type: "number", show: (r) => r.mode === "range" },
      { k: "from_end", label: t("von hinten"), type: "check", show: (r) => r.mode === "range" },
      { k: "anchor", label: t("Text"), type: "text", show: (r) => r.mode !== "range" },
      { k: "keep_anchor", label: t("Text behalten"), type: "check", show: (r) => r.mode !== "range" },
      { k: "last", label: t("letztes Vorkommen"), type: "check", show: (r) => r.mode !== "range" },
      { ...REGEX, show: (r) => r.mode !== "range" },
    ],
  },
  case: {
    label: t("Groß-/Kleinschreibung"),
    summary: (r) => ({ title: t("Titel"), words: t("Jedes Wort"), sentence: t("Satz"), upper: t("GROSS"), lower: t("klein"), swap: t("umkehren") } as Record<string, string>)[r.mode] ?? "",
    defaults: { mode: "title", target: "stem", small_words: true, keep: "" },
    fields: [
      { k: "mode", label: t("Modus"), type: "select", options: [["title", t("Titel (Füllwörter klein)")], ["words", t("Jedes Wort groß")], ["sentence", t("Satzanfang groß")], ["upper", t("GROSS")], ["lower", t("klein")], ["swap", t("uMKEHREN")]] },
      TARGET,
      { k: "keep", label: t("Schreibweise beibehalten"), type: "text", grow: true, placeholder: t("MP3, DJ, McCartney") },
    ],
    hint: t("Titel-Modus schreibt Füllwörter (der, und, of, the …) klein, außer am Anfang und Ende."),
  },
  truncate: {
    label: t("Kürzen"),
    summary: (r) => t("max. {n} Zeichen", { n: r.length }),
    defaults: { length: 60, keep: "start", word_boundary: true },
    fields: [
      { k: "length", label: t("Max. Zeichen"), type: "number" },
      { k: "keep", label: t("Behalten"), type: "select", options: [["start", t("Anfang")], ["end", t("Ende")]] },
      { k: "word_boundary", label: t("an Wortgrenze"), type: "check", show: (r) => r.keep !== "end" },
    ],
  },
  cleanup: {
    label: t("Aufräumen"),
    summary: () => "",
    defaults: { underscores: true, dots: false, percent: false, dashes: false, strip_brackets: false, strip_digits_start: false, ascii: false, windows: false, collapse: true, trim: true },
    fields: [
      { k: "underscores", label: t("_ → Leerzeichen"), type: "check" },
      { k: "dots", label: t(". → Leerzeichen"), type: "check" },
      { k: "percent", label: t("%20/+ → Leerzeichen"), type: "check" },
      { k: "dashes", label: t("„ - “ vereinheitlichen"), type: "check" },
      { k: "strip_brackets", label: t("(Klammern) entfernen"), type: "check" },
      { k: "strip_digits_start", label: t("Ziffern am Anfang weg"), type: "check" },
      { k: "ascii", label: t("Umlaute → ae/oe/ue"), type: "check" },
      { k: "windows", label: t("Windows/SMB-sicher"), type: "check" },
      { k: "collapse", label: t("Doppelte Leerzeichen"), type: "check" },
      { k: "trim", label: t("Ränder trimmen"), type: "check" },
    ],
    hint: t("„Windows/SMB-sicher“ ersetzt <code>&lt; &gt; : \" \\ | ? *</code> – wichtig, wenn die Unraid-Freigabe von Windows oder macOS aus genutzt wird."),
  },
  remove_chars: {
    label: t("Zeichen/Wörter entfernen"),
    summary: (r) => [r.digits && t("Ziffern"), r.punctuation && t("Satzzeichen"), r.chars && `„${r.chars}“`, r.words && r.words].filter(Boolean).join(", "),
    defaults: { digits: false, letters: false, spaces: false, punctuation: false, non_alnum: false, non_ascii: false, chars: "", words: "", target: "stem" },
    fields: [
      { k: "digits", label: t("Ziffern"), type: "check" },
      { k: "letters", label: t("Buchstaben"), type: "check" },
      { k: "spaces", label: t("Leerzeichen"), type: "check" },
      { k: "punctuation", label: t("Satzzeichen"), type: "check" },
      { k: "non_alnum", label: t("alles außer Buchst./Ziffern"), type: "check" },
      { k: "non_ascii", label: t("Nicht-ASCII"), type: "check" },
      { k: "chars", label: t("Diese Zeichen"), type: "text", size: 10 },
      { k: "words", label: t("Wörter (Komma-getrennt)"), type: "text", grow: true, placeholder: t("1080p, x264, WEB-DL") },
      TARGET,
    ],
  },
  move: {
    label: t("Text verschieben"),
    summary: (r) => r.find ? `„${r.find}“ → ${r.to === "end" ? t("Ende") : t("Anfang")}` : "",
    defaults: { find: "", to: "start", separator: " ", regex: false },
    fields: [
      { k: "find", label: t("Text"), type: "text", grow: true },
      { k: "to", label: t("nach"), type: "select", options: [["start", t("vorne")], ["end", t("hinten")]] },
      { k: "separator", label: t("Trenner"), type: "text", size: 4 },
      REGEX,
    ],
    hint: t("Beispiel: Regex <code>\\(\\d{4}\\)</code> nach vorne verschiebt die Jahreszahl in Klammern an den Anfang."),
  },
  swap: {
    label: t("Teile tauschen"),
    summary: (r) => t("um „{separator}“", { separator: r.separator }),
    defaults: { separator: " - ", occurrence: "first", joiner: "" },
    fields: [
      { k: "separator", label: t("Trenner"), type: "text", size: 6 },
      { k: "occurrence", label: t("am"), type: "select", options: [["first", t("ersten")], ["last", t("letzten")]] },
      { k: "joiner", label: t("neuer Trenner"), type: "text", size: 6, placeholder: t("gleich") },
    ],
    hint: t("„Künstler - Titel“ → „Titel - Künstler“."),
  },
  number: {
    label: t("Nummerieren"),
    summary: (r) => t("ab {start}", { start: r.start }),
    defaults: { start: 1, step: 1, padding: 2, separator: " - ", position: "start", format: "decimal", per_folder: false, prefix: "", suffix: "", index: 0 },
    fields: [
      { k: "position", label: t("Position"), type: "select", options: [["start", t("vorne")], ["end", t("hinten")], ["index", t("an Stelle")], ["replace", t("ganzer Name")]] },
      { k: "index", label: t("Stelle"), type: "number", show: (r) => r.position === "index" },
      { k: "format", label: t("Format"), type: "select", options: [["decimal", t("1, 2, 3")], ["alpha", "a, b, c"], ["ALPHA", "A, B, C"], ["roman", t("I, II, III")], ["roman_lower", t("i, ii, iii")]] },
      { k: "start", label: t("Start"), type: "number" },
      { k: "step", label: t("Schritt"), type: "number" },
      { k: "padding", label: t("Stellen"), type: "number", show: (r) => r.format === "decimal" },
      { k: "separator", label: t("Trenner"), type: "text", size: 5, show: (r) => r.position === "start" || r.position === "end" },
      { k: "prefix", label: t("davor"), type: "text", size: 6 },
      { k: "suffix", label: t("danach"), type: "text", size: 6 },
      { k: "per_folder", label: t("pro Ordner neu"), type: "check" },
    ],
    hint: t("Reihenfolge = Sortierung der Vorschau (nur ausgewählte Einträge)."),
  },
  pad_numbers: {
    label: t("Zahlen auffüllen"),
    summary: (r) => t("{n} Stellen", { n: r.width }),
    defaults: { width: 2, which: "all", strip_zeros: true },
    fields: [
      { k: "width", label: t("Stellen"), type: "number" },
      { k: "which", label: t("Zahlen"), type: "select", options: [["all", t("alle")], ["first", t("erste")], ["last", t("letzte")]] },
      { k: "strip_zeros", label: t("zu viele Nullen kürzen"), type: "check" },
    ],
    hint: t("„Track 1“ → „Track 01“, damit die Sortierung stimmt."),
  },
  shift_numbers: {
    label: t("Zahlen verschieben"),
    summary: (r) => (r.offset >= 0 ? "+" : "") + r.offset,
    defaults: { offset: 1, which: "first" },
    fields: [
      { k: "offset", label: t("um"), type: "number" },
      { k: "which", label: t("Zahlen"), type: "select", options: [["first", t("erste")], ["last", t("letzte")], ["all", t("alle")]] },
    ],
    hint: t("Beispiel: Folgen 0–9 auf 1–10 umnummerieren. Führende Nullen bleiben erhalten."),
  },
  date: {
    label: t("Datum einfügen"),
    summary: (r) => ({ modified: t("Änderung"), created: t("Erstellung"), taken: t("Aufnahme"), now: t("heute") } as Record<string, string>)[r.source] ?? "",
    defaults: { source: "taken", format: "%Y-%m-%d", position: "start", separator: " ", fallback: true },
    fields: [
      { k: "source", label: t("Datum"), type: "select", options: [["taken", t("Aufnahme (EXIF/Video/ID3)")], ["modified", t("Änderungsdatum")], ["created", t("Erstelldatum")], ["now", t("heute")]] },
      { k: "format", label: t("Format"), type: "datalist", size: 14, options: ["%Y-%m-%d", "%Y-%m-%d %H.%M.%S", "%Y%m%d_%H%M%S", "%d.%m.%Y", "%Y", "%Y-%m", "%B %Y"] },
      { k: "position", label: t("Position"), type: "select", options: [["start", t("vorne")], ["end", t("hinten")], ["replace", t("ganzer Name")]] },
      { k: "separator", label: t("Trenner"), type: "text", size: 4, show: (r) => r.position !== "replace" },
      { k: "fallback", label: t("sonst Änderungsdatum"), type: "check", show: (r) => r.source === "taken" },
    ],
    hint: t("<code>%Y</code> Jahr · <code>%m</code> Monat · <code>%d</code> Tag · <code>%H</code> Stunde · <code>%M</code> Minute · <code>%S</code> Sekunde · <code>%B</code> Monatsname"),
  },
  template: {
    label: t("Vorlage / Metadaten"),
    summary: (r) => r.template,
    defaults: { template: "{track:2} - {title}" },
    fields: [{ k: "template", label: t("Vorlage"), type: "text", grow: true }],
    hint: TEMPLATE_HINT + " " + t("Dateien ohne die verwendeten Tags bleiben unverändert."),
  },
  list: {
    label: t("Aus Liste"),
    summary: (r) => t("{n} Zeilen", { n: String(r.lines || "").split("\n").filter((l: string) => l.trim()).length }),
    defaults: { lines: "", mode: "replace", separator: " - " },
    fields: [
      { k: "mode", label: t("Zeile wird"), type: "select", options: [["replace", t("neuer Name")], ["with_ext", t("Name inkl. Endung")], ["prefix", t("vorangestellt")], ["suffix", t("angehängt")]] },
      { k: "separator", label: t("Trenner"), type: "text", size: 5, show: (r) => r.mode === "prefix" || r.mode === "suffix" },
      { k: "lines", label: t("Liste (eine Zeile pro Datei)"), type: "textarea", grow: true },
    ],
    hint: t("Zeile 1 gilt für den ersten ausgewählten Eintrag in der Vorschau-Reihenfolge, Zeile 2 für den zweiten …"),
  },
  extension: {
    label: t("Dateiendung"),
    summary: (r) => ({ lower: t("klein"), upper: t("GROSS"), set: `→ .${r.value}`, append: `+ .${r.value}`, normalize: t("vereinheitlichen"), remove: t("entfernen") } as Record<string, string>)[r.mode] ?? "",
    defaults: { mode: "lower", value: "" },
    fields: [
      { k: "mode", label: t("Aktion"), type: "select", options: [["lower", t("klein")], ["upper", t("GROSS")], ["normalize", t("vereinheitlichen (jpeg→jpg)")], ["set", t("ersetzen durch")], ["append", t("anhängen")], ["remove", t("entfernen")]] },
      { k: "value", label: t("Endung"), type: "text", size: 6, show: (r) => r.mode === "set" || r.mode === "append" },
    ],
  },
};

const CONDITION_FIELDS: FieldDef[] = [
  { k: "only_mode", label: t("Nur wenn Name"), type: "select", options: [["contains", t("enthält")], ["regex", t("passt auf Regex")], ["ext", t("Endung ist")]] },
  { k: "only", label: "", type: "text", grow: true, placeholder: t("leer = immer anwenden") },
  { k: "only_invert", label: t("umkehren (nicht)"), type: "check" },
];
