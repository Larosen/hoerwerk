"use strict";
// ------------------------------------------------------------------------- //
// Einstellungen → Downloads (/settings/transfers): what SABnzbd loads in the audiobook category, and its
// history – longer than the old panel in the search, with tidying up (only for admins, like the whole page).
// ------------------------------------------------------------------------- //
const downloads = { timer: undefined as number | undefined, data: null as Downloads | null };
const DOWNLOAD_HISTORY = 100;

function downloadsVisible(): boolean {
  return router.current?.view === "settings" && router.current.section === "transfers";
}

async function clearDownloads(ids: string[] = []): Promise<void> {
  if (!ids.length && !(await confirmBox(t("Alle fertigen und fehlgeschlagenen Downloads aus dem SABnzbd-Verlauf nehmen? Die Dateien bleiben liegen."), t("Aufräumen")))) return;
  try {
    const out = await api<{ removed: number; kept: number }>("POST", "/api/downloads/clear", { ids });
    toast(plural(out.removed, t("1 Eintrag aufgeräumt"), t("{n} Einträge aufgeräumt", { n: out.removed })) +
      (out.kept ? " · " + t("{n} bleiben, bis Hörwerk sie übernommen hat", { n: out.kept }) : ""));
  } catch (e) { toast(errMsg(e), true); }
  pollDownloads(true);
}

function renderDownloads(): void {
  const data = downloads.data;
  if (!data) return;
  const hide = $<HTMLInputElement>("#dlHideDone").checked;
  const list = $("#dlList");
  list.replaceChildren();
  $("#dlSpeed").textContent = data.speed && data.speed !== "0 " ? `${data.speed}B/s${data.paused ? " · " + t("pausiert") : ""}` : data.paused ? t("pausiert") : "";
  const items = (data.items || []).filter((d) => !(hide && d.done));
  const finished = (data.items || []).filter((d) => d.done).length;
  $<HTMLButtonElement>("#dlClear").disabled = !data.configured || !finished;
  if (!data.configured) list.append(el("li", { class: "empty" }, t("SABnzbd ist nicht eingerichtet.")));
  else if (data.error) list.append(el("li", { class: "empty err" }, data.error));
  else if (!items.length) list.append(el("li", { class: "empty" }, data.items.length ? t("Nichts läuft gerade.") : t("Keine Downloads in dieser Kategorie.")));
  for (const d of items) {
    const pct = Math.round((d.progress || 0) * 100);
    const failed = d.status === "Failed";
    const remove = d.done && d.id ? el("button", { type: "button", class: "icon-btn", title: t("Aus dem SABnzbd-Verlauf nehmen (Dateien bleiben)") }, icon("x")) : null;
    remove?.addEventListener("click", () => clearDownloads([d.id || ""]));
    list.append(el("li", { class: "job " + (failed ? "error" : d.done ? "done" : "running") },
      el("div", { class: "job-top" },
        el("span", { class: "job-name", title: d.name }, d.name),
        el("span", { class: "row gap" },
          el("span", { class: "badge " + (failed ? "error" : d.done ? "done" : "running") },
            d.done ? (failed ? t("fehlgeschlagen") : t("fertig → Eingang")) : `${d.status} ${pct}%`),
          remove)),
      d.error ? el("div", { class: "job-msg" }, d.error)
        : el("div", { class: "job-msg" }, [d.size, d.left && !d.done ? t("noch {time}", { time: d.left }) : "", d.finished ? fmtAgo(d.finished) : ""].filter(Boolean).join(" · ")),
      !d.done ? el("div", { class: "bar" }, el("i", { style: `width:${pct}%` })) : null));
  }
}

async function pollDownloads(force = false): Promise<void> {
  clearTimeout(downloads.timer);
  if (!downloadsVisible() && !force) return;
  try { downloads.data = await api<Downloads>("GET", `/api/downloads?limit=${DOWNLOAD_HISTORY}`); }
  catch { downloads.data = { items: [], error: t("Server nicht erreichbar") }; }
  renderDownloads();
  const busy = (downloads.data.items || []).some((d) => !d.done);
  if (downloadsVisible()) downloads.timer = setTimeout(pollDownloads, busy ? 3000 : 15000);
}

function initDownloads(): void {
  if (!IS_ADMIN) return;
  const hide = $<HTMLInputElement>("#dlHideDone");
  hide.checked = store.get("dlHideDone", false);
  hide.addEventListener("change", () => { store.set("dlHideDone", hide.checked); renderDownloads(); });
  $("#dlClear").addEventListener("click", () => clearDownloads());
}

initDownloads();
