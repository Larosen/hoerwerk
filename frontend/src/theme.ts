"use strict";
// Colour scheme: "auto" follows the device, "light" (Papier), "dark" (Abend) and "oled" (Nacht, true black) are fixed.
// Where the choice is kept: in the account (works on every device) or – if the user wants – only on this device.
// Loaded in <head> so the page never flashes in the wrong colours.
type ThemeMode = "auto" | "light" | "dark" | "oled";
const THEME_MODES: ThemeMode[] = ["auto", "light", "dark", "oled"];

const hwTheme = (() => {
  const KEY = "br:theme";                                        // a choice made for this device only
  const isMode = (v: unknown): v is ThemeMode => typeof v === "string" && (THEME_MODES as string[]).includes(v);
  const readDevice = (): ThemeMode | null => {
    try {
      const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? "null");
      return isMode(v) ? v : null;
    } catch { return null; }
  };
  const readAccount = (): ThemeMode => {
    const v = typeof ME !== "undefined" && ME ? (ME as { theme?: unknown }).theme : undefined;
    return isMode(v) ? v : "auto";
  };
  const apply = (mode: ThemeMode): void => {
    if (mode === "auto") document.documentElement.removeAttribute("data-theme");
    else document.documentElement.dataset.theme = mode;
    // the colour of the browser bar (Safari, installed app) follows the page background
    let meta = document.querySelector<HTMLMetaElement>('meta[name="theme-color"]:not([media])');
    if (!meta) {
      meta = document.createElement("meta");
      meta.name = "theme-color";
      document.head.append(meta);
    }
    const bg = getComputedStyle(document.documentElement).getPropertyValue("--bg").trim();
    if (bg) meta.content = bg;
  };
  const current = (): ThemeMode => readDevice() ?? readAccount();
  const refresh = (): void => apply(current());
  refresh();
  document.addEventListener("DOMContentLoaded", refresh);
  window.matchMedia?.("(prefers-color-scheme: dark)").addEventListener("change", refresh);
  return {
    /** the scheme in use right now */
    mode: current,
    /** where the choice is kept */
    scope: (): "device" | "account" => (readDevice() ? "device" : "account"),
    /** account = on every device (saved on the server), device = only here */
    async set(mode: ThemeMode, scope: "device" | "account"): Promise<void> {
      if (scope === "device") {
        try { localStorage.setItem(KEY, JSON.stringify(mode)); } catch { /* private mode: only this page view */ }
        apply(mode);
        return;
      }
      await api("PUT", "/api/auth/prefs", { theme: mode });
      if (typeof ME !== "undefined" && ME) (ME as { theme?: string }).theme = mode;
      try { localStorage.removeItem(KEY); } catch { /* ignore */ }
      apply(mode);
    },
    /** a device-only choice is dropped; the account's scheme applies again */
    forgetDevice(): void {
      try { localStorage.removeItem(KEY); } catch { /* ignore */ }
      apply(readAccount());
    },
  };
})();
