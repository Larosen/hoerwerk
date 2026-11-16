"use strict";
// Settings → General: export and import of the settings and sources (for a move to a new server).

function initTransfer(): void {
  const keys = document.getElementById("exportKeys");
  keys?.addEventListener("click", (e) => {
    // with the API keys the file is as sensitive as a password: ask once
    if (!window.confirm(t("Die Datei enthält dann die API-Schlüssel deiner Indexer und von SABnzbd. Wie ein Passwort aufbewahren – fortfahren?"))) e.preventDefault();
  });
  const pick = document.getElementById("importFile") as HTMLInputElement | null;
  pick?.addEventListener("change", async () => {
    const file = pick.files?.[0];
    pick.value = "";
    if (!file) return;
    if (file.size > 3 * 1024 * 1024) { toast(t("Die Datei ist zu groß"), true); return; }
    let data: unknown;
    try { data = JSON.parse(await file.text()); } catch { toast(t("Das ist keine Hörwerk-Exportdatei"), true); return; }
    if (!(await confirmBox(t("Die Einstellungen und Quellen aus „{name}“ übernehmen? Gleiche Einstellungen werden überschrieben.", { name: file.name }), t("Importieren")))) return;
    try {
      const res = await api<{ settings: number; indexers: number }>("POST", "/api/settings/import", data);
      toast(t("Übernommen: {n} Einstellungen, {m} Indexer", { n: res.settings, m: res.indexers }));
      window.setTimeout(() => window.location.reload(), 1200);
    } catch (e) { toast(errMsg(e), true); }
  });
}

initTransfer();
