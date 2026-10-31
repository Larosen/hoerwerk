"use strict";

// ------------------------------------------------------------------------- //
// ntfy notifications (part of the settings dialog)
// ------------------------------------------------------------------------- //
interface NotifyConfig {
  url: string; topic: string; user: string; priority: number; app_url: string;
  token_set: boolean; password_set: boolean; events: Record<string, boolean>;
  configured: boolean; last_error: string | null; available: string[];
}

const NOTIFY_EVENTS: Record<string, string> = {
  filed: t("Hörbuch abgelegt"),
  review: t("Hörbuch braucht Prüfung"),
  duplicate: t("Schon in der Bibliothek"),
  failed: t("Fehler bei einem Hörbuch"),
  download_sent: t("Download an SABnzbd geschickt"),
  download_done: t("Download in SABnzbd fertig"),
  download_failed: t("Download in SABnzbd fehlgeschlagen"),
  abs_error: t("Audiobookshelf-Scan fehlgeschlagen"),
  plex_error: t("Plex-Scan fehlgeschlagen"),
};

function renderNotify(c: NotifyConfig): void {
  const f = $<HTMLFormElement>("#settingsForm");
  field(f, "ntfy_url").value = c.url;
  field(f, "ntfy_topic").value = c.topic;
  field(f, "ntfy_user").value = c.user;
  field<HTMLSelectElement>(f, "ntfy_priority").value = String(c.priority || 3);
  field(f, "ntfy_app_url").value = c.app_url;
  for (const [name, isSet] of [["ntfy_token", c.token_set], ["ntfy_password", c.password_set]] as const) {
    const input = field(f, name);
    input.value = "";
    input.placeholder = isSet ? t("gespeichert – leer lassen") : "";
  }
  $("#notifyEvents").replaceChildren(...c.available.map((name) => el("label", { class: "check" },
    el("input", { type: "checkbox", "data-event": name, checked: !!c.events[name] }),
    NOTIFY_EVENTS[name] || name)));
  $("#notifyState").textContent = c.last_error ? "✗ " + c.last_error
    : c.configured ? t("eingerichtet") : t("nicht eingerichtet");
}

async function loadNotify(): Promise<void> {
  try { renderNotify(await api<NotifyConfig>("GET", "/api/notify")); } catch (e) { $("#notifyState").textContent = errMsg(e); }
}

async function saveNotify(): Promise<void> {
  const f = $<HTMLFormElement>("#settingsForm");
  const events: Record<string, boolean> = {};
  for (const box of document.querySelectorAll<HTMLInputElement>("#notifyEvents input[data-event]")) {
    events[box.dataset.event || ""] = box.checked;
  }
  await api<NotifyConfig>("PUT", "/api/notify", {
    url: field(f, "ntfy_url").value, topic: field(f, "ntfy_topic").value, user: field(f, "ntfy_user").value,
    token: field(f, "ntfy_token").value, password: field(f, "ntfy_password").value,
    priority: Number(field<HTMLSelectElement>(f, "ntfy_priority").value), app_url: field(f, "ntfy_app_url").value,
    events,
  });
}

function initNotify(): void {
  $("#notifyTestBtn").addEventListener("click", async () => {
    try {
      await saveNotify();                 // test what is in the form right now
      const r = await api<{ ok: boolean; message: string }>("POST", "/api/notify/test");
      toast((r.ok ? "✓ " : "✗ ") + r.message, !r.ok);
      await loadNotify();
    } catch (e) { toast(errMsg(e), true); }
  });
  $("#settingsBtn").addEventListener("click", loadNotify);
}

initNotify();
