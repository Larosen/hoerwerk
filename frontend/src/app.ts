"use strict";
// ------------------------------------------------------------------------- //
// The settings form (pages under /settings: settings.ts shows one part at a time; all parts are saved together)
// ------------------------------------------------------------------------- //
const state = {
  settings: {} as Settings,
};

async function loadSettings(): Promise<void> {
  state.settings = await api<Settings>("GET", "/api/settings");
}

type SettingsInput = HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement;
function settingsInputs(): [string, SettingsInput][] {
  const form = $<HTMLFormElement>("#settingsForm");
  return Object.keys(state.settings).flatMap((k) => {
    const input = form.elements.namedItem(k);
    return input instanceof HTMLInputElement || input instanceof HTMLSelectElement || input instanceof HTMLTextAreaElement
      ? [[k, input] as [string, SettingsInput]] : [];
  });
}

/** Put the saved values into the form (when the settings pages are opened – not between two parts, so edits stay). */
async function fillSettings(): Promise<void> {
  try { await loadSettings(); } catch (e) { toast(errMsg(e), true); return; }
  for (const [k, input] of settingsInputs()) {
    const v = state.settings[k];
    if (input instanceof HTMLInputElement && input.type === "checkbox") input.checked = !!v; else input.value = String(v);
  }
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
    document.documentElement.classList.toggle("no-help", state.settings.show_help === false);
    toast(t("Einstellungen gespeichert"));
  } catch (e) { toast(errMsg(e), true); }
}

function init(): void {
  $("#settingsForm").addEventListener("submit", (e) => { e.preventDefault(); saveSettings(); });
  if (IS_ADMIN) loadSettings().catch((e) => toast(errMsg(e), true));
}

init();
