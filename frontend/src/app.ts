"use strict";

// ------------------------------------------------------------------------- //
// State
// ------------------------------------------------------------------------- //
type Kind = "all" | "audio" | "video" | "image" | "folder" | "other";
const KINDS: [Kind, string][] = [["all", t("Alle")], ["audio", t("Audio")], ["video", t("Video")], ["image", t("Bilder")], ["folder", t("Ordner")], ["other", t("Andere")]];
interface ListOpts { recursive: boolean; dirs: boolean; changedOnly: boolean }
interface RenameOpts { sidecars: boolean; allow_subdirs: boolean; conflict: string; sidecar_exts?: unknown }

const state = {
  path: store.get("path", ""),
  folder: null as Folder | null,
  rules: store.get<Rule[]>("rules", [{ type: "cleanup", enabled: true, ...RULE_TYPES.cleanup.defaults }]),
  unchecked: new Set<string>(),
  filter: "",
  kind: "all" as Kind,
  sort: store.get("sort", "name"),
  opts: store.get<ListOpts>("opts", { recursive: false, dirs: false, changedOnly: false }),
  preview: new Map<string, PreviewItem>(),
  extraRows: [] as PreviewItem[],        // sidecars that are not in the listing selection
  previewOk: false,
  settings: {} as Settings,
  presets: {} as Record<string, Rule[]>,
  lastClicked: null as number | null,
};

// ------------------------------------------------------------------------- //
// Folders & listing
// ------------------------------------------------------------------------- //
async function openFolder(path: string, keepSelection = false): Promise<void> {
  const q = new URLSearchParams({ path, recursive: state.opts.recursive ? "1" : "0", dirs: state.opts.dirs ? "1" : "0" });
  let folder: Folder;
  try {
    folder = await api<Folder>("GET", "/api/browse?" + q);
  } catch (e) {
    if (path) return openFolder("");
    toast(errMsg(e), true);
    return;
  }
  state.folder = folder;
  if (folder.path !== state.path || !keepSelection) state.unchecked.clear();
  state.path = folder.path;
  store.set("path", state.path);
  state.lastClicked = null;
  state.preview = new Map();
  state.extraRows = [];
  renderFolder(folder);
  renderChips();
  renderFiles();
  schedulePreview(0);
}

function renderFolder(f: Folder): void {
  const crumbs = $("#crumbs");
  crumbs.replaceChildren(el("a", { href: "#", onclick: (e) => { e.preventDefault(); openFolder(""); } }, t("Daten")));
  let acc = "";
  for (const part of f.path ? f.path.split("/") : []) {
    acc = acc ? acc + "/" + part : part;
    const target = acc;
    crumbs.append(el("span", { class: "sep" }, "/"), el("a", { href: "#", onclick: (e) => { e.preventDefault(); openFolder(target); } }, part));
  }
  const list = $("#dirList");
  list.replaceChildren();
  const parent = f.parent;
  if (parent !== null) list.append(el("li", {}, el("button", { onclick: () => openFolder(parent) }, "↩︎ ..")));
  for (const d of f.dirs) {
    const p = f.path ? f.path + "/" + d.name : d.name;
    list.append(el("li", {}, el("button", { onclick: () => openFolder(p), title: d.name }, "📁 " + d.name)));
  }
  if (!f.dirs.length && f.parent === null) list.append(el("li", { class: "empty" }, t("Keine Unterordner")));
  const audio = f.files.filter((x) => x.kind === "audio" && !x.name.includes("/")).length;
  $("#mp3Box").hidden = !f.mp3_count && !audio;
  $("#convertNowBtn").hidden = !f.mp3_count;
  $("#mp3Info").textContent = f.mp3_count ? t("{n} MP3 in diesem Ordner", { n: f.mp3_count }) : t("{n} Audiodatei(en)", { n: audio });
}

function renderChips(): void {
  const counts: Partial<Record<Kind, number>> = { all: 0 };
  for (const f of state.folder?.files || []) {
    counts.all = (counts.all || 0) + 1;
    const k = f.kind as Kind;
    counts[k] = (counts[k] || 0) + 1;
  }
  if (state.kind !== "all" && !counts[state.kind]) state.kind = "all";
  $("#kindChips").replaceChildren(...KINDS.filter(([k]) => k === "all" || counts[k]).map(([k, label]) =>
    el("button", {
      class: "chip" + (state.kind === k ? " on" : ""), "aria-pressed": String(state.kind === k),
      onclick: () => { state.kind = k; renderChips(); renderFiles(); schedulePreview(0); },
    }, label, el("span", { class: "n" }, String(counts[k] || 0)))));
}

function sortedFiles(): FileEntry[] {
  const files = [...(state.folder?.files || [])];
  const byName = (a: FileEntry, b: FileEntry): number => collator.compare(a.name, b.name);
  switch (state.sort) {
    case "name-desc": files.sort((a, b) => byName(b, a)); break;
    case "mtime": files.sort((a, b) => a.mtime - b.mtime || byName(a, b)); break;
    case "mtime-desc": files.sort((a, b) => b.mtime - a.mtime || byName(a, b)); break;
    case "size": files.sort((a, b) => a.size - b.size || byName(a, b)); break;
    case "kind": files.sort((a, b) => collator.compare(a.kind, b.kind) || byName(a, b)); break;
    default: files.sort(byName);
  }
  return files;
}

function visible(f: FileEntry): boolean {
  if (state.kind !== "all" && f.kind !== state.kind) return false;
  const q = state.filter.trim().toLowerCase();
  return !q || f.name.toLowerCase().includes(q);
}

/** Items that will be renamed, in display order. */
function selectedFiles(): string[] {
  return sortedFiles().filter((f) => visible(f) && !state.unchecked.has(f.name)).map((f) => f.name);
}

function renderFiles(): void {
  const body = $("#previewBody");
  body.replaceChildren();
  const files = sortedFiles().filter(visible);
  if (!files.length) {
    body.append(el("tr", {}, el("td", { colspan: 5, class: "empty-row" },
      state.folder?.files.length ? t("Keine Einträge für diesen Filter") : t("Keine Dateien in diesem Ordner"))));
  }
  files.forEach((f, i) => {
    const cb = el("input", { type: "checkbox", "aria-label": t("Auswählen") });
    cb.checked = !state.unchecked.has(f.name);
    cb.addEventListener("click", (ev) => toggleFile(files, i, cb.checked, ev.shiftKey));
    const info = f.dir ? null : el("button", { class: "icon-btn", title: t("Metadaten & Platzhalter"), onclick: () => showMeta(f.name) }, "ⓘ");
    body.append(el("tr", { "data-name": f.name, class: f.dir ? "is-dir" : "" },
      el("td", { class: "c-check" }, cb),
      el("td", { class: "old" }, nameCell(f)),
      el("td", { class: "c-arrow" }, "→"),
      el("td", { class: "new" }, ""),
      el("td", { class: "c-info" }, info)));
  });
  if (state.folder?.truncated) {
    body.append(el("tr", {}, el("td", { colspan: 5, class: "empty-row" }, t("Liste gekürzt – zu viele Dateien. Engeren Ordner wählen."))));
  }
  updateRows();
}

function nameCell(f: FileEntry): DocumentFragment {
  const frag = document.createDocumentFragment();
  const slash = f.name.lastIndexOf("/");
  if (slash >= 0) frag.append(el("span", { class: "dirpart" }, f.name.slice(0, slash + 1)));
  frag.append((f.dir ? "📁 " : "") + f.name.slice(slash + 1));
  if (f.kind === "audio" && f.name.toLowerCase().endsWith(".mp3")) frag.append(el("span", { class: "tag" }, "MP3"));
  return frag;
}

function toggleFile(files: FileEntry[], i: number, checked: boolean, shift: boolean): void {
  const last = state.lastClicked;
  const range = shift && last !== null
    ? files.slice(Math.min(i, last), Math.max(i, last) + 1)
    : [files[i]];
  for (const f of range) {
    if (checked) state.unchecked.delete(f.name); else state.unchecked.add(f.name);
  }
  state.lastClicked = i;
  renderFiles();
  schedulePreview();
}

/** Highlight the changed middle part of the new name. */
function diffNode(oldName: string, newName: string): DocumentFragment {
  let a = 0;
  while (a < oldName.length && a < newName.length && oldName[a] === newName[a]) a++;
  let b = 0;
  while (b < oldName.length - a && b < newName.length - a &&
         oldName[oldName.length - 1 - b] === newName[newName.length - 1 - b]) b++;
  const frag = document.createDocumentFragment();
  frag.append(newName.slice(0, a));
  const mid = newName.slice(a, newName.length - b);
  if (mid) frag.append(el("mark", {}, mid));
  else if (oldName.length - a - b > 0) frag.append(el("mark", { class: "del", title: "entfernt" }, "·"));
  frag.append(newName.slice(newName.length - b));
  return frag;
}

function updateRows(): void {
  let changed = 0, errors = 0, selected = 0;
  const body = $("#previewBody");
  for (const tr of body.querySelectorAll<HTMLTableRowElement>("tr[data-name]")) {
    const name = tr.dataset.name || "";
    const on = !state.unchecked.has(name);
    const cell = tr.querySelector("td.new");
    if (!cell) continue;
    const item = on ? state.preview.get(name) : undefined;
    cell.replaceChildren();
    tr.classList.toggle("off", !on);
    tr.classList.remove("conflict", "unchanged");
    if (on) selected++;
    if (item) {
      cell.append(item.changed ? diffNode(item.old, item.new) : item.new);
      if (item.changed) changed++; else tr.classList.add("unchanged");
      if (item.resolved) cell.append(el("span", { class: "note" }, t("automatisch nummeriert")));
      if (item.sidecar) cell.append(el("span", { class: "note" }, t("Begleitdatei – folgt {file}", { file: (item.of || "").split("/").pop() || "" })));
      if (item.error) { tr.classList.add("conflict"); cell.append(el("span", { class: "err" }, item.error)); errors++; }
    } else {
      cell.append(on ? "…" : name);
    }
    tr.hidden = !!(state.opts.changedOnly && item && !item.changed && !item.error);
  }
  // Sidecar rows (files that follow their main file)
  for (const tr of body.querySelectorAll("tr.sidecar")) tr.remove();
  for (const item of state.extraRows) {
    const anchor = body.querySelector(`tr[data-name="${CSS.escape(item.of || "")}"]`);
    const tr = el("tr", { class: "sidecar" + (item.error ? " conflict" : "") },
      el("td", { class: "c-check" }),
      el("td", { class: "old" }, el("span", { class: "tag soft" }, t("Begleitdatei")), item.old),
      el("td", { class: "c-arrow" }, "→"),
      el("td", { class: "new" }, diffNode(item.old, item.new), item.error ? el("span", { class: "err" }, item.error) : null),
      el("td", { class: "c-info" }));
    if (anchor) anchor.after(tr); else body.append(tr);
    changed++;
    if (item.error) errors++;
  }

  const visibleRows = body.querySelectorAll("tr[data-name]").length;
  const all = $<HTMLInputElement>("#checkAll");
  all.checked = selected > 0 && selected === visibleRows;
  all.indeterminate = selected > 0 && selected < visibleRows;

  const s = $("#summary");
  s.replaceChildren(el("b", {}, String(changed)), " " + t("von {n} Einträgen werden umbenannt", { n: selected + state.extraRows.length }));
  if (state.extraRows.length) s.append(" " + t("(inkl. {n} nicht ausgewählter Begleitdateien)", { n: state.extraRows.length }));
  if (errors) s.append(" · ", el("span", { class: "bad" }, errors > 1 ? t("{n} Konflikte", { n: errors }) : t("1 Konflikt")));
  $<HTMLButtonElement>("#applyBtn").disabled = !state.previewOk || errors > 0 || changed === 0;
  const hasMp3 = selectedFiles().some((n) => n.toLowerCase().endsWith(".mp3"));
  $("#convertCheck").parentElement?.classList.toggle("dim", !hasMp3);
}

// ------------------------------------------------------------------------- //
// Rules editor
// ------------------------------------------------------------------------- //
function saveRules(): void {
  store.set("rules", state.rules);
  $("#ruleCount").textContent = state.rules.length ? `(${state.rules.length})` : "";
  schedulePreview();
}

let dragIndex: number | null = null;

function renderRules(): void {
  const list = $("#ruleList");
  list.replaceChildren();
  state.rules.forEach((rule, idx) => list.append(ruleCard(rule, idx)));
  $("#ruleCount").textContent = state.rules.length ? `(${state.rules.length})` : "";
}

function ruleCard(rule: Rule, idx: number): HTMLLIElement {
  const def = RULE_TYPES[rule.type];
  if (!def) return el("li", { class: "rule" }, t("Unbekannte Regel: {type}", { type: rule.type }));
  const li = el("li", { class: "rule" + (rule.enabled === false ? " disabled" : "") + (rule.collapsed ? " collapsed" : ""), draggable: "false" });

  const enabled = el("input", { type: "checkbox", title: t("Regel aktiv"), "aria-label": t("Regel aktiv") });
  enabled.checked = rule.enabled !== false;
  enabled.addEventListener("change", () => { rule.enabled = enabled.checked; li.classList.toggle("disabled", !enabled.checked); saveRules(); });

  const summary = el("span", { class: "rule-summary" }, def.summary?.(rule) || "");
  const cond = rule.only ? el("span", { class: "tag soft" }, t("bedingt")) : null;
  const handle = el("span", { class: "handle", title: t("Ziehen zum Sortieren"), "aria-hidden": "true" }, "⋮⋮");
  handle.addEventListener("mousedown", () => li.setAttribute("draggable", "true"));
  handle.addEventListener("touchstart", () => li.setAttribute("draggable", "true"), { passive: true });

  const head = el("div", { class: "rule-head" },
    handle, enabled,
    el("button", { class: "rule-title", "aria-expanded": String(!rule.collapsed), onclick: () => { rule.collapsed = !rule.collapsed; li.classList.toggle("collapsed", rule.collapsed); store.set("rules", state.rules); } },
      `${idx + 1}. ${def.label}`),
    summary, cond,
    el("div", { class: "rule-tools" },
      el("button", { class: "icon-btn", title: t("Nach oben"), disabled: idx === 0, onclick: () => moveRule(idx, idx - 1) }, "↑"),
      el("button", { class: "icon-btn", title: t("Nach unten"), disabled: idx === state.rules.length - 1, onclick: () => moveRule(idx, idx + 1) }, "↓"),
      el("button", { class: "icon-btn", title: t("Duplizieren"), onclick: () => { state.rules.splice(idx + 1, 0, structuredClone(rule)); renderRules(); saveRules(); } }, "⧉"),
      el("button", { class: "icon-btn", title: t("Entfernen"), onclick: () => { state.rules.splice(idx, 1); renderRules(); saveRules(); } }, "✕")));

  const fields = el("div", { class: "rule-fields" });
  const refresh = (): void => {
    fields.replaceChildren();
    for (const f of def.fields) if (!f.show || f.show(rule)) fields.append(fieldInput(rule, f, refresh));
    if (def.hint) fields.append(el("div", { class: "hint", html: def.hint }));
    const condBox = el("details", { class: "cond", open: rule.only ? true : null },
      el("summary", {}, t("Bedingung")),
      el("div", { class: "rule-fields" }, CONDITION_FIELDS.map((f) => fieldInput(rule, f, refresh))));
    fields.append(condBox);
    summary.textContent = def.summary?.(rule) || "";
  };
  refresh();

  li.append(head, fields);
  li.addEventListener("dragstart", (e) => {
    dragIndex = idx; li.classList.add("dragging");
    if (e.dataTransfer) { e.dataTransfer.effectAllowed = "move"; e.dataTransfer.setData("text/plain", String(idx)); }
  });
  li.addEventListener("dragend", () => { li.classList.remove("dragging"); li.setAttribute("draggable", "false"); dragIndex = null; });
  li.addEventListener("dragover", (e) => { if (dragIndex !== null) { e.preventDefault(); li.classList.add("drop"); } });
  li.addEventListener("dragleave", () => li.classList.remove("drop"));
  li.addEventListener("drop", (e) => {
    e.preventDefault(); li.classList.remove("drop");
    if (dragIndex !== null && dragIndex !== idx) moveRule(dragIndex, idx);
  });
  return li;
}

function moveRule(from: number, to: number): void {
  if (to < 0 || to >= state.rules.length) return;
  const [r] = state.rules.splice(from, 1);
  state.rules.splice(to, 0, r);
  renderRules(); saveRules();
}

let fieldSeq = 0;
function fieldInput(rule: Rule, f: FieldDef, refresh: () => void): HTMLLabelElement {
  const id = "f" + (++fieldSeq);
  const options = (f.options || []) as [string, string][];
  if (f.type === "select") {
    const input = el("select", { id }, options.map(([v, l]) => el("option", { value: v }, l)));
    input.value = rule[f.k] ?? options[0][0];
    rule[f.k] = input.value;
    input.addEventListener("change", () => { rule[f.k] = input.value; saveRules(); refresh(); });
    return el("label", { class: f.grow ? "grow" : "", for: id }, f.label, input);
  }
  if (f.type === "check") {
    const input = el("input", { type: "checkbox", id });
    input.checked = !!rule[f.k];
    input.addEventListener("change", () => { rule[f.k] = input.checked; saveRules(); refresh(); });
    return el("label", { class: "check" }, input, f.label);
  }
  if (f.type === "textarea") {
    const input = el("textarea", { id, rows: 5, spellcheck: "false" });
    input.value = rule[f.k] ?? "";
    input.addEventListener("input", () => { rule[f.k] = input.value; saveRules(); });
    return el("label", { class: (f.grow ? "grow" : "") + " block", for: id }, f.label, input);
  }
  const listId = f.type === "datalist" ? id + "l" : null;
  const input = el("input", { id, type: f.type === "number" ? "number" : "text", size: f.size || null, spellcheck: "false", placeholder: f.placeholder || null, list: listId });
  input.value = rule[f.k] ?? "";
  input.addEventListener("input", () => { rule[f.k] = f.type === "number" ? Number(input.value) : input.value; saveRules(); });
  if (listId) {
    return el("label", { class: f.grow ? "grow" : "", for: id }, f.label, input,
      el("datalist", { id: listId }, (f.options as string[]).map((o) => el("option", { value: o }))));
  }
  return el("label", { class: f.grow ? "grow" : "", for: id }, f.label, input);
}

function addRuleOptions(): void {
  $("#addRuleSelect").replaceChildren(el("option", { value: "" }, t("+ Regel hinzufügen …")),
    ...RULE_GROUPS.map(([g, types]) => el("optgroup", { label: g }, types.map((t) => el("option", { value: t }, RULE_TYPES[t].label)))));
}

// ------------------------------------------------------------------------- //
// Preview + apply
// ------------------------------------------------------------------------- //
function renameOptions(): RenameOpts {
  return {
    sidecars: $<HTMLInputElement>("#optSidecars").checked,
    allow_subdirs: $<HTMLInputElement>("#optSubdirs").checked,
    conflict: $<HTMLInputElement>("#optConflict").checked ? "number" : "error",
    sidecar_exts: state.settings.sidecar_exts,
  };
}

let previewTimer: number | undefined, previewSeq = 0;
function schedulePreview(delay = 250): void {
  state.previewOk = false;
  clearTimeout(previewTimer);
  previewTimer = setTimeout(runPreview, delay);
  updateRows();
}

async function runPreview(): Promise<void> {
  if (!state.folder) return;
  const seq = ++previewSeq;
  const files = selectedFiles();
  if (!files.length) { state.preview = new Map(); state.extraRows = []; state.previewOk = true; updateRows(); return; }
  try {
    const res = await api<{ items: PreviewItem[] }>("POST", "/api/preview",
      { path: state.path, files, rules: state.rules, options: renameOptions() });
    if (seq !== previewSeq) return;
    // Sidecars that are listed get their own row; the others are shown under their main file
    const listed = new Set(files);
    state.preview = new Map(res.items.filter((it) => listed.has(it.old)).map((it) => [it.old, it]));
    state.extraRows = res.items.filter((it) => it.sidecar && !listed.has(it.old));
    state.previewOk = true;
  } catch (e) {
    if (seq !== previewSeq) return;
    state.preview = new Map();
    state.extraRows = [];
    toast(errMsg(e), true);
  }
  updateRows();
}

async function apply(): Promise<void> {
  const btn = $<HTMLButtonElement>("#applyBtn");
  if (btn.disabled) return;
  btn.disabled = true;
  try {
    const res = await api<{ renamed: number; jobs: ConvertJob[] }>("POST", "/api/rename", {
      path: state.path, files: selectedFiles(), rules: state.rules,
      options: renameOptions(), convert: $<HTMLInputElement>("#convertCheck").checked,
    });
    let msg = t("{n} Einträge umbenannt", { n: res.renamed });
    if (res.jobs?.length) msg += " · " + t("{n} M4B-Konvertierung(en) gestartet", { n: res.jobs.length });
    toast(msg);
    if (res.jobs?.length) pollJobs(true);
    await openFolder(state.path, true);
  } catch (e) {
    toast(errMsg(e), true);
    btn.disabled = false;
  }
}

async function undo(): Promise<void> {
  try {
    const res = await api<{ restored: number; folder: string }>("POST", "/api/undo");
    toast(t("{n} Einträge zurückbenannt", { n: res.restored }));
    await openFolder(res.folder, true);
    if ($<HTMLDialogElement>("#historyDialog").open) showHistory();
  } catch (e) { toast(errMsg(e), true); }
}

async function showHistory(): Promise<void> {
  const list = $("#historyList");
  const dialog = $<HTMLDialogElement>("#historyDialog");
  list.replaceChildren(el("li", { class: "muted" }, t("Lade …")));
  if (!dialog.open) dialog.showModal();
  try {
    const hist = await api<HistoryEntry[]>("GET", "/api/history");
    list.replaceChildren();
    if (!hist.length) list.append(el("li", { class: "muted" }, t("Noch keine Umbenennungen.")));
    hist.forEach((h, i) => {
      const when = new Date(h.time * 1000).toLocaleString(LOCALE, { dateStyle: "short", timeStyle: "short" });
      list.append(el("li", { class: "hist" },
        el("div", { class: "hist-top" },
          el("b", {}, t("{n} Einträge", { n: h.count })), el("span", { class: "muted" }, " " + t("in /{folder} · {when}", { folder: h.folder, when })),
          i === 0 ? el("button", { class: "btn small", onclick: undo }, t("Rückgängig")) : null),
        el("ul", { class: "hist-sample" }, (h.sample || []).map(([o, n]) => el("li", {}, `${o} → ${n}`)))));
    });
  } catch (e) { list.replaceChildren(el("li", { class: "err" }, errMsg(e))); }
}

// ------------------------------------------------------------------------- //
// Metadata dialog
// ------------------------------------------------------------------------- //
interface FileMeta { kind: string; meta: Record<string, unknown>; dates: { modified: string; created: string; taken: string | null } }

async function showMeta(file: string): Promise<void> {
  const d = $<HTMLDialogElement>("#metaDialog");
  $("#metaTitle").textContent = file.split("/").pop() || file;
  const body = $("#metaBody");
  body.replaceChildren(el("p", { class: "muted" }, t("Lese Metadaten …")));
  d.showModal();
  try {
    const res = await api<FileMeta>("GET", "/api/meta?" + new URLSearchParams({ path: state.path, file }));
    const rows: [string, unknown][] = [
      ["modified:%Y-%m-%d", res.dates.modified], ["created:%Y-%m-%d", res.dates.created],
      ["taken:%Y-%m-%d", res.dates.taken || "–"],
      ...Object.entries(res.meta).sort(([a], [b]) => a.localeCompare(b)),
    ];
    body.replaceChildren(...rows.map(([k, v]) => {
      const code = "{" + k + "}";
      return el("div", { class: "meta-row" },
        el("button", {
          class: "ph", title: t("Kopieren"),
          onclick: async () => {
            try { await navigator.clipboard.writeText(code); toast(t("{code} kopiert", { code })); } catch { toast(code); }
          },
        }, code),
        el("span", {}, String(v)));
    }));
    if (!Object.keys(res.meta).length) body.append(el("p", { class: "muted" }, t("Keine Tags in dieser Datei gefunden.")));
  } catch (e) { body.replaceChildren(el("p", { class: "err" }, errMsg(e))); }
}

// ------------------------------------------------------------------------- //
// Jobs („Jetzt zu M4B“)
// ------------------------------------------------------------------------- //
const STATUS: Record<ConvertStatus, string> = { queued: t("wartet"), running: t("läuft"), done: t("fertig"), error: t("Fehler"), cancelled: t("abgebrochen") };
let jobTimer: number | undefined, lastJobsJson = "", hadRunning = false;

function applyJobs(jobs: ConvertJob[], force = false): boolean {
  const json = JSON.stringify(jobs);
  if (force || json !== lastJobsJson) { lastJobsJson = json; renderJobs(jobs); }
  const running = jobs.some((j) => j.status === "queued" || j.status === "running");
  if (hadRunning && !running && state.folder) openFolder(state.path, true);
  hadRunning = running;
  return running;
}

async function pollJobs(force = false): Promise<void> {
  clearTimeout(jobTimer);
  let running = false;
  try { running = applyJobs(await api<ConvertJob[]>("GET", "/api/jobs"), force); } catch { /* retry later */ }
  if (!live.ok) jobTimer = setTimeout(pollJobs, running ? 1000 : 6000);   // fallback without live updates
}
onLive("jobs", (jobs) => applyJobs(jobs));
document.addEventListener("livechange", (e) => { if ((e as CustomEvent<boolean>).detail) clearTimeout(jobTimer); else pollJobs(); });

function renderJobs(jobs: ConvertJob[]): void {
  const list = $("#jobList");
  list.replaceChildren();
  if (!jobs.length) { list.append(el("li", { class: "empty" }, t("Keine Jobs"))); return; }
  for (const j of jobs) {
    const pct = Math.round(j.progress * 100);
    const li = el("li", { class: "job " + j.status },
      el("div", { class: "job-top" },
        el("span", { class: "job-name", title: "/" + j.folder }, j.output || j.folder.split("/").pop() || "/"),
        el("span", { class: "badge " + j.status }, STATUS[j.status] + (j.status === "running" ? ` ${pct}%` : ""))),
      el("div", { class: "job-msg" }, j.message));
    if (j.status === "running" || j.status === "queued") {
      li.append(el("div", { class: "bar" }, el("i", { style: `width:${pct}%` })));
      li.append(el("button", { class: "btn small ghost", onclick: () => cancelJob(j.id) }, t("Abbrechen")));
    }
    if (j.log?.length) li.append(el("details", {}, el("summary", {}, t("Log")), el("pre", {}, j.log.join("\n"))));
    list.append(li);
  }
}

async function cancelJob(id: string): Promise<void> {
  try { await api("POST", `/api/jobs/${id}/cancel`); pollJobs(true); } catch (e) { toast(errMsg(e), true); }
}

// ------------------------------------------------------------------------- //
// Settings & presets
// ------------------------------------------------------------------------- //
async function loadSettings(): Promise<void> {
  state.settings = await api<Settings>("GET", "/api/settings");
  $<HTMLInputElement>("#convertCheck").checked = !!state.settings.auto_convert;
  const saved = store.get<RenameOpts | null>("renameOpts", null);
  const o = saved || { sidecars: !!state.settings.sidecars, allow_subdirs: !!state.settings.allow_subdirs, conflict: String(state.settings.conflict) };
  $<HTMLInputElement>("#optSidecars").checked = !!o.sidecars;
  $<HTMLInputElement>("#optSubdirs").checked = !!o.allow_subdirs;
  $<HTMLInputElement>("#optConflict").checked = o.conflict === "number";
}

function settingsInputs(): [string, HTMLInputElement | HTMLSelectElement][] {
  const form = $<HTMLFormElement>("#settingsForm");
  return Object.keys(state.settings).flatMap((k) => {
    const input = form.elements.namedItem(k);
    return input instanceof HTMLInputElement || input instanceof HTMLSelectElement ? [[k, input] as [string, HTMLInputElement | HTMLSelectElement]] : [];
  });
}

function openSettings(): void {
  for (const [k, input] of settingsInputs()) {
    const v = state.settings[k];
    if (input instanceof HTMLInputElement && input.type === "checkbox") input.checked = !!v; else input.value = String(v);
  }
  $<HTMLDialogElement>("#settingsDialog").showModal();
  api<SystemInfo>("GET", "/api/system").then((s) => {
    const sb = s.sandbox;
    const jail = !sb.active ? (sb.mode === "off" ? t("ffmpeg ohne Sandbox (FFMPEG_SANDBOX=off)") : t("ffmpeg ohne Sandbox – Kernel/Docker erlaubt kein Landlock"))
      : sb.network_blocked ? t("ffmpeg abgeschottet (Landlock ABI {abi}, ohne Netz)", { abi: sb.landlock_abi })
        : t("ffmpeg abgeschottet (Landlock ABI {abi})", { abi: sb.landlock_abi });
    $("#systemInfo").textContent = t("Version {version} · ffmpeg {ffmpeg} · {jail}", { version: s.version, ffmpeg: s.ffmpeg, jail });
  }).catch(() => { /* optional */ });
}

async function saveSettings(): Promise<void> {
  const data: Settings = {};
  for (const [k, input] of settingsInputs()) {
    data[k] = input instanceof HTMLInputElement && input.type === "checkbox" ? input.checked : input.value;
  }
  try {
    await saveNotify();
    const langChanged = data.language !== undefined && data.language !== state.settings.language;
    state.settings = await api<Settings>("PUT", "/api/settings", data);
    if (langChanged) { location.reload(); return; }   // all texts come from /i18n.js
    store.set("renameOpts", null);
    await loadSettings();
    schedulePreview(0);
    toast(t("Einstellungen gespeichert"));
  } catch (e) { toast(errMsg(e), true); }
}

async function loadPresets(): Promise<void> {
  try { state.presets = await api<Record<string, Rule[]>>("GET", "/api/presets"); } catch { state.presets = {}; }
  const sel = $<HTMLSelectElement>("#presetSelect");
  const current = sel.value;
  sel.replaceChildren(el("option", { value: "" }, t("Voreinstellungen …")),
    ...Object.keys(state.presets).sort(collator.compare).map((n) => el("option", { value: n }, n)));
  if (state.presets[current]) sel.value = current;
}

function savePreset(): void {
  const d = $<HTMLDialogElement>("#presetDialog");
  const nameInput = $<HTMLInputElement>("#presetName");
  nameInput.value = $<HTMLSelectElement>("#presetSelect").value || "";
  d.returnValue = "";
  d.addEventListener("close", async () => {
    const name = nameInput.value.trim();
    if (d.returnValue !== "save" || !name) return;
    if (state.presets[name] && !(await confirmBox(t("„{name}“ überschreiben?", { name }), t("Überschreiben")))) return;
    state.presets[name] = structuredClone(state.rules).map((r) => { delete r.collapsed; return r; });
    try {
      await api("PUT", "/api/presets", state.presets);
      await loadPresets();
      $<HTMLSelectElement>("#presetSelect").value = name;
      toast(t("„{name}“ gespeichert", { name }));
    } catch (e) { toast(errMsg(e), true); }
  }, { once: true });
  d.showModal();
}

async function deletePreset(): Promise<void> {
  const name = $<HTMLSelectElement>("#presetSelect").value;
  if (!name) { toast(t("Erst eine Voreinstellung auswählen")); return; }
  if (!(await confirmBox(t("Voreinstellung „{name}“ löschen?", { name }), t("Löschen")))) return;
  delete state.presets[name];
  await api("PUT", "/api/presets", state.presets);
  await loadPresets();
}

function exportPresets(): void {
  const blob = new Blob([JSON.stringify(state.presets, null, 2)], { type: "application/json" });
  const a = el("a", { href: URL.createObjectURL(blob), download: "better-renamer-presets.json" });
  document.body.append(a); a.click(); a.remove();
}

async function importPresets(file: File): Promise<void> {
  try {
    const data: unknown = JSON.parse(await file.text());
    if (typeof data !== "object" || data === null || Array.isArray(data)) throw new Error(t("Erwartet ein Objekt { Name: [Regeln] }"));
    const merged = { ...state.presets, ...(data as Record<string, Rule[]>) };
    await api("PUT", "/api/presets", merged);
    await loadPresets();
    toast(t("{n} Voreinstellung(en) importiert", { n: Object.keys(data).length }));
  } catch (e) { toast(t("Import fehlgeschlagen: {error}", { error: errMsg(e) }), true); }
}

// ------------------------------------------------------------------------- //
// Wiring
// ------------------------------------------------------------------------- //
function init(): void {
  addRuleOptions();
  const add = $<HTMLSelectElement>("#addRuleSelect");
  add.addEventListener("change", () => {
    const type = add.value;
    if (!type) return;
    state.rules.push({ type, enabled: true, ...structuredClone(RULE_TYPES[type].defaults) });
    add.value = "";
    renderRules(); saveRules();
    $("#ruleList").lastElementChild?.scrollIntoView({ block: "nearest" });
  });
  $("#clearRulesBtn").addEventListener("click", async () => {
    if (state.rules.length && !(await confirmBox(t("Alle Regeln entfernen?"), t("Entfernen")))) return;
    state.rules = []; renderRules(); saveRules();
  });
  $("#collapseRulesBtn").addEventListener("click", () => {
    const collapse = state.rules.some((r) => !r.collapsed);
    state.rules.forEach((r) => (r.collapsed = collapse));
    $("#collapseRulesBtn").textContent = collapse ? t("Alle ausklappen") : t("Alle einklappen");
    renderRules(); store.set("rules", state.rules);
  });

  const presetSelect = $<HTMLSelectElement>("#presetSelect");
  presetSelect.addEventListener("change", () => {
    const p = state.presets[presetSelect.value];
    if (p) { state.rules = structuredClone(p); renderRules(); saveRules(); }
  });
  $("#savePresetBtn").addEventListener("click", savePreset);
  $("#deletePresetBtn").addEventListener("click", deletePreset);
  $("#exportPresetBtn").addEventListener("click", exportPresets);
  const importInput = $<HTMLInputElement>("#importPresetInput");
  importInput.addEventListener("change", () => { const f = importInput.files?.[0]; if (f) importPresets(f); importInput.value = ""; });

  const filter = $<HTMLInputElement>("#filterInput");
  filter.addEventListener("input", () => { state.filter = filter.value; renderFiles(); schedulePreview(); });
  const sort = $<HTMLSelectElement>("#sortSelect");
  sort.value = state.sort;
  sort.addEventListener("change", () => { state.sort = sort.value; store.set("sort", state.sort); renderFiles(); schedulePreview(0); });

  const optBox = (id: string): HTMLInputElement => $<HTMLInputElement>(id);
  optBox("#optRecursive").checked = !!state.opts.recursive;
  optBox("#optDirs").checked = !!state.opts.dirs;
  optBox("#optChangedOnly").checked = !!state.opts.changedOnly;
  for (const [id, key] of [["#optRecursive", "recursive"], ["#optDirs", "dirs"]] as const) {
    const box = optBox(id);
    box.addEventListener("change", () => { state.opts[key] = box.checked; store.set("opts", state.opts); openFolder(state.path); });
  }
  optBox("#optChangedOnly").addEventListener("change", () => { state.opts.changedOnly = optBox("#optChangedOnly").checked; store.set("opts", state.opts); updateRows(); });
  for (const id of ["#optSidecars", "#optSubdirs", "#optConflict"]) {
    $(id).addEventListener("change", () => {
      const o = renameOptions();
      store.set("renameOpts", { sidecars: o.sidecars, allow_subdirs: o.allow_subdirs, conflict: o.conflict });
      schedulePreview(0);
    });
  }

  const checkAll = $<HTMLInputElement>("#checkAll");
  checkAll.addEventListener("change", () => {
    for (const f of state.folder?.files || []) {
      if (!visible(f)) continue;
      if (checkAll.checked) state.unchecked.delete(f.name); else state.unchecked.add(f.name);
    }
    renderFiles(); schedulePreview(0);
  });

  $("#applyBtn").addEventListener("click", apply);
  $("#undoBtn").addEventListener("click", undo);
  $("#historyBtn").addEventListener("click", showHistory);
  $("#refreshBtn").addEventListener("click", () => openFolder(state.path, true));
  $("#convertNowBtn").addEventListener("click", async () => {
    try { await api("POST", "/api/jobs", { path: state.path }); toast(t("M4B-Konvertierung gestartet")); pollJobs(true); }
    catch (e) { toast(errMsg(e), true); }
  });
  $("#clearJobsBtn").addEventListener("click", async () => { await api("DELETE", "/api/jobs"); pollJobs(true); });

  $("#settingsBtn").addEventListener("click", openSettings);
  const settingsDialog = $<HTMLDialogElement>("#settingsDialog");
  settingsDialog.addEventListener("close", () => { if (settingsDialog.returnValue === "save") saveSettings(); });

  document.addEventListener("keydown", (e) => {
    const mod = e.metaKey || e.ctrlKey;
    const typing = /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement?.tagName || "");
    if (mod && e.key === "Enter") { e.preventDefault(); apply(); }
    else if (mod && e.key.toLowerCase() === "z" && !typing && !document.querySelector("dialog[open]")) { e.preventDefault(); undo(); }
  });

  renderRules();
  loadSettings().catch((e) => toast(errMsg(e), true)).finally(() => openFolder(state.path));
  loadPresets();
  pollJobs(true);
}

init();
