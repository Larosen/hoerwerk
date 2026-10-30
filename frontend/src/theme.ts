// Colour scheme: "auto" follows the system, "light"/"dark" are fixed. Loaded in <head> so the page
// never flashes in the wrong colours.
(() => {
  type Mode = "auto" | "light" | "dark";
  // runs before core.js: its own lookup in the catalog of /i18n.js
  const t = (text: string): string => (typeof I18N !== "undefined" && I18N.text[text]) || text;
  const KEY = "br:theme";
  const MODES: Record<Mode, [string, string]> = { auto: ["contrast", t("Automatisch (wie System)")], light: ["sun", t("Hell")], dark: ["moon", t("Dunkel")] };
  const read = (): Mode => {
    try {
      const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? "null");
      return typeof v === "string" && v in MODES ? v as Mode : "auto";
    } catch { return "auto"; }
  };
  const apply = (mode: Mode): void => {
    if (mode === "auto") document.documentElement.removeAttribute("data-theme");
    else document.documentElement.dataset.theme = mode;
    const btn = document.getElementById("themeBtn");
    if (btn) {
      btn.querySelector("use")?.setAttribute("href", "#i-" + MODES[mode][0]);
      btn.title = t("Design: {mode} – klicken zum Wechseln").replace("{mode}", MODES[mode][1]);
      btn.setAttribute("aria-label", btn.title);
    }
  };
  let current = read();
  apply(current);
  document.addEventListener("DOMContentLoaded", () => {
    const btn = document.getElementById("themeBtn");
    if (!btn) return;
    apply(current);
    btn.addEventListener("click", () => {
      const order = Object.keys(MODES) as Mode[];
      current = order[(order.indexOf(current) + 1) % order.length];
      try { localStorage.setItem(KEY, JSON.stringify(current)); } catch { /* private mode: only this page view */ }
      apply(current);
    });
  });
})();
