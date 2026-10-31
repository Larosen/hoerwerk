"use strict";
// ------------------------------------------------------------------------- //
// Settings dialog (the audiobook views live in books.ts and search.ts)
// ------------------------------------------------------------------------- //
const state = {
  settings: {} as Settings,
};

async function loadSettings(): Promise<void> {
  state.settings = await api<Settings>("GET", "/api/settings");
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
    toast(t("Einstellungen gespeichert"));
  } catch (e) { toast(errMsg(e), true); }
}

function init(): void {
  $("#settingsBtn").addEventListener("click", openSettings);
  const settingsDialog = $<HTMLDialogElement>("#settingsDialog");
  settingsDialog.addEventListener("close", () => { if (settingsDialog.returnValue === "save") saveSettings(); });
  loadSettings().catch((e) => toast(errMsg(e), true));
}

init();
