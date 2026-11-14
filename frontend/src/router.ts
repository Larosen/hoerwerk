"use strict";
// ------------------------------------------------------------------------- //
// Pages with their own address (/search, /settings/health …): the browser's back and
// forward buttons work, every page can be bookmarked or opened in a new tab.
// The server only hands out the page shell to signed-in users (and /settings only to admins);
// every piece of data is checked again by the API.
// ------------------------------------------------------------------------- //
interface Route { view: string; section: string }

const VIEWS = ["books", "search", "discover", "requests", "settings", "account", "stats", "more"];
const VIEW_ELEMENTS: Record<string, string> = {
  books: "#viewBooks", search: "#viewSearch", discover: "#viewDiscover", requests: "#viewRequests",
  settings: "#viewSettings", account: "#viewAccount", stats: "#viewStats", more: "#viewMore",
};
/** The main tabs – the last one used is where "/" leads. */
const MAIN_VIEWS = ["books", "search", "discover", "requests"];
const VIEW_TITLES: Record<string, () => string> = {
  books: () => t("Hörbücher"), search: () => t("Suchen"), discover: () => t("Entdecken"), requests: () => t("Anfragen"),
  settings: () => t("Einstellungen"), account: () => t("Konto"), stats: () => t("Statistik"), more: () => t("Mehr"),
};

const router = { current: null as Route | null, ready: false };

function settingsSections(): string[] {
  return [...document.querySelectorAll<HTMLElement>("#settingsNav [data-section]")].map((a) => a.dataset.section || "");
}

/** Everyone starts with what is good to hear – the inbox and library are one tab further for admins. */
function defaultView(): string {
  return "discover";
}

/** Which page an address means – anything unknown or not allowed leads to the start page. */
function parseRoute(path: string): Route {
  const [view = "", section = ""] = path.split("/").filter(Boolean);
  if (!VIEWS.includes(view)) return { view: defaultView(), section: "" };
  if (!IS_ADMIN && (view === "books" || view === "settings")) return { view: defaultView(), section: "" };
  // "Mehr" is the phone's menu for what does not fit into the tab bar – the sidebar of wide screens shows all of it
  if (view === "more" && window.matchMedia("(min-width: 780px)").matches) return { view: defaultView(), section: "" };
  if (view === "settings" && section === "stats") return { view: "stats", section: "" };    // where it used to be
  if (view === "settings") return { view, section: settingsSections().includes(section) ? section : "" };
  // /stats/7: one account in detail – admins only (everyone else has their own on /stats)
  if (view === "stats") return { view, section: IS_ADMIN && /^\d{1,9}$/.test(section) ? section : "" };
  return { view, section: "" };
}

function routePath(r: Route): string {
  return "/" + r.view + (r.section ? "/" + r.section : "");
}

function showRoute(r: Route, push: boolean): void {
  const path = routePath(r);
  if (location.pathname !== path) {
    if (push) history.pushState(null, "", path);
    else history.replaceState(null, "", path);
  }
  const previous = router.current;
  router.current = r;
  for (const [view, sel] of Object.entries(VIEW_ELEMENTS)) $(sel).hidden = view !== r.view;
  for (const a of document.querySelectorAll<HTMLElement>(".topbar [data-view]")) {
    // Protokoll lives inside the settings but has its own entry; "Mehr" stands for everything behind it on the phone
    const here = a.dataset.view === r.view && (!a.dataset.section || a.dataset.section === r.section)
      && (!a.dataset.notSection || a.dataset.notSection !== r.section);
    const behindMore = a.classList.contains("more-tab") && ["more", "stats", "settings", "account"].includes(r.view);
    if (here || behindMore) a.setAttribute("aria-current", "page"); else a.removeAttribute("aria-current");
  }
  if (MAIN_VIEWS.includes(r.view)) store.set("view", r.view);
  document.title = `${VIEW_TITLES[r.view]()} · Hörwerk`;          // settings.ts names the area on top
  if (r.view === "books") pollBooks(true);
  if (!previous || previous.view !== r.view) {
    document.dispatchEvent(new CustomEvent<string>("viewchange", { detail: r.view }));
  }
  document.dispatchEvent(new CustomEvent<{ route: Route; previous: Route | null }>("routechange", { detail: { route: r, previous } }));
  if (push) window.scrollTo(0, 0);
}

function navigate(path: string, push = true): void {
  showRoute(parseRoute(path), push);
}

/** Switch the main tab (kept for the scripts that jump between pages, e.g. "Suchen" on a suggestion). */
function setView(view: string): void {
  navigate("/" + view);
}

function isAppPath(path: string): boolean {
  return path === "/" || VIEWS.includes(path.split("/").filter(Boolean)[0] || "");
}

function initRouter(): void {
  // own links open as a page change; new tab, middle click & co. stay with the browser
  document.addEventListener("click", (e) => {
    // until every script has registered its listeners, a click is a normal page load – otherwise the page
    // would change before e.g. the settings listen, and their form would stay empty
    if (!router.ready) return;
    if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    const a = (e.target as Element | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
    if (!a || a.target || a.hasAttribute("download")) return;
    const url = new URL(a.href, location.href);
    if (url.origin !== location.origin || !isAppPath(url.pathname)) return;
    e.preventDefault();
    navigate(url.pathname);
  });
  window.addEventListener("popstate", () => { if (router.ready) navigate(location.pathname, false); });
  // after every script registered its listeners
  document.addEventListener("DOMContentLoaded", () => {
    router.ready = true;
    navigate(location.pathname === "/" ? "/" + defaultView() : location.pathname, false);
  });
}

initRouter();
