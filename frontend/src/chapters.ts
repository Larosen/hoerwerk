"use strict";
// The chapter editor of a library book: rename, merge with the previous chapter, split, move a start.

interface EditChapter { start: number; title: string }

const chapterEdit = { path: "", duration: 0, list: [] as EditChapter[], original: "" };

/** 3725.5 -> "1:02:05.5", 65 -> "1:05" */
function fmtChapterTime(sec: number): string {
  const h = Math.floor(sec / 3600), m = Math.floor((sec % 3600) / 60), s = sec - h * 3600 - m * 60;
  const ss = (Math.round(s * 10) / 10).toFixed(Number.isInteger(Math.round(s * 10) / 10) ? 0 : 1).padStart(Number.isInteger(Math.round(s * 10) / 10) ? 2 : 4, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

/** "1:02:05.5", "65", "1:05" -> seconds; NaN when it is none of them */
function parseChapterTime(text: string): number {
  const parts = text.trim().replace(",", ".").split(":");
  if (!parts.length || parts.length > 3 || parts.some((p) => !/^\d+(\.\d+)?$/.test(p))) return NaN;
  return parts.reduce((total, p) => total * 60 + parseFloat(p), 0);
}

/** Why the list can not be saved – or "" */
function chapterProblem(list: EditChapter[], duration: number): string {
  if (!list.length) return t("Ein Hörbuch braucht mindestens ein Kapitel");
  for (let i = 0; i < list.length; i++) {
    const c = list[i];
    if (!Number.isFinite(c.start) || c.start < 0 || (duration > 0 && c.start >= duration))
      return t("Kapitel {n}: der Beginn liegt außerhalb des Hörbuchs", { n: i + 1 });
    if (i > 0 && c.start <= list[i - 1].start) return t("Kapitel {n}: der Beginn muss nach dem vorigen liegen", { n: i + 1 });
    if (!c.title.trim()) return t("Kapitel {n} hat keinen Namen", { n: i + 1 });
  }
  return "";
}

function renderChapterEditor(): void {
  const list = chapterEdit.list;
  const problem = chapterProblem(list, chapterEdit.duration);
  $("#chaptersNote").textContent = problem || plural(list.length, t("1 Kapitel"), t("{n} Kapitel", { n: list.length }))
    + (chapterEdit.duration ? " · " + fmtDuration(chapterEdit.duration) : "");
  $("#chaptersNote").classList.toggle("err", !!problem);
  $<HTMLButtonElement>("#chaptersSave").disabled = !!problem || JSON.stringify(list) === chapterEdit.original;
  $("#chaptersList").replaceChildren(...list.map((c, i) => {
    const time = el("input", { type: "text", class: "chapter-time", value: fmtChapterTime(c.start), disabled: i === 0,
      inputmode: "decimal", "aria-label": t("Beginn von Kapitel {n}", { n: i + 1 }), spellcheck: "false" });
    time.addEventListener("change", () => {
      const v = parseChapterTime(time.value);
      if (Number.isNaN(v)) { time.value = fmtChapterTime(c.start); toast(t("Zeit bitte als h:mm:ss oder mm:ss"), true); return; }
      c.start = v;
      renderChapterEditor();
    });
    const name = el("input", { type: "text", class: "chapter-name", value: c.title, maxlength: "200", "aria-label": t("Name von Kapitel {n}", { n: i + 1 }) });
    name.addEventListener("input", () => { c.title = name.value; $<HTMLButtonElement>("#chaptersSave").disabled = !!chapterProblem(list, chapterEdit.duration) || JSON.stringify(list) === chapterEdit.original; });
    const end = i + 1 < list.length ? list[i + 1].start : chapterEdit.duration;
    return el("li", { class: "chapter-row" },
      el("span", { class: "chapter-n muted small" }, String(i + 1)), time, name,
      el("span", { class: "chapter-tools" },
        el("button", { type: "button", class: "btn small ghost", disabled: i === 0, title: t("Mit dem vorigen Kapitel zusammenführen"),
          onclick: () => { list.splice(i, 1); renderChapterEditor(); } }, t("Zusammenführen")),
        el("button", { type: "button", class: "btn small ghost", disabled: end - c.start < 2, title: t("In der Mitte teilen – danach den Beginn anpassen"),
          onclick: () => { list.splice(i + 1, 0, { start: Math.round(((c.start + end) / 2) * 10) / 10, title: `${c.title} (2)` }); renderChapterEditor(); } }, t("Teilen"))));
  }));
}

async function openChapterEditor(path: string, title: string): Promise<void> {
  try {
    const res = await api<{ duration: number; chapters: EditChapter[] }>("GET", `/api/library/chapters?path=${encodeURIComponent(path)}`);
    chapterEdit.path = path;
    chapterEdit.duration = res.duration;
    chapterEdit.list = res.chapters.map((c) => ({ start: c.start, title: c.title }));
    chapterEdit.original = JSON.stringify(chapterEdit.list);
    $("#chaptersTitle").textContent = title;
    renderChapterEditor();
    const d = $<HTMLDialogElement>("#chaptersDialog");
    if (!d.open) d.showModal();
  } catch (e) { toast(errMsg(e), true); }
}

function initChapterEditor(): void {
  $("#chaptersSave").addEventListener("click", async (e) => {
    const btn = e.currentTarget as HTMLButtonElement;
    btn.disabled = true;
    try {
      await api("PUT", "/api/library/chapters", { path: chapterEdit.path, chapters: chapterEdit.list });
      toast(t("Kapitel gespeichert"));
      $<HTMLDialogElement>("#chaptersDialog").close();
    } catch (err) { toast(errMsg(err), true); btn.disabled = false; }
  });
  // the rules for new audiobooks (no book title, no track numbers) on the names in the list
  $("#chaptersTidy").addEventListener("click", async (e) => {
    const btn = e.currentTarget as HTMLButtonElement;
    if (chapterEdit.original !== JSON.stringify(chapterEdit.list) && !(await confirmBox(t("Deine Änderungen sind noch nicht gespeichert. Aufräumen speichert die Namen sofort und lädt die Liste neu."), t("Aufräumen")))) return;
    btn.disabled = true;
    try {
      const r = await api<{ changed: number }>("POST", "/api/library/chapters", { path: chapterEdit.path });
      toast(r.changed ? t("{n} Kapitelnamen aufgeräumt", { n: r.changed }) : t("Die Kapitelnamen sind schon sauber"));
      await openChapterEditor(chapterEdit.path, $("#chaptersTitle").textContent || "");
    } catch (err) { toast(errMsg(err), true); } finally { btn.disabled = false; }
  });
}

initChapterEditor();
