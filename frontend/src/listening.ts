"use strict";
// ------------------------------------------------------------------------- //
// What the family listens to: link Audiobookshelf accounts and Plex profiles to Hörwerk accounts.
// Konto: everyone links their own Audiobookshelf account. Einstellungen → Konten: admins assign both.
// ------------------------------------------------------------------------- //
interface ListenLink { id: string; name: string }
interface ListenLinks { abs: ListenLink | null; plex: ListenLink | null }
interface ListenSummary { linked: string[]; seconds: number; books: number; finished: number; last: number | null; errors: string[] }
interface MyListening { available: { abs: boolean; plex: boolean }; links: ListenLinks; recent: ListenSummary | null }

function fmtListen(seconds: number): string {
  if (!seconds) return "–";
  const h = seconds / 3600;
  return h < 1 ? t("{n} min", { n: Math.max(1, Math.round(seconds / 60)) }) : `${h < 10 ? h.toLocaleString(LOCALE, { maximumFractionDigits: 1 }) : Math.round(h)} h`;
}

// -- Konto: my own links ------------------------------------------------------------------
async function openListening(): Promise<void> {
  let res: MyListening;
  try { res = await api<MyListening>("GET", "/api/me/listening"); } catch { return; }
  const fs = $("#listenFieldset");
  fs.hidden = !res.available.abs && !res.available.plex;
  if (fs.hidden) return;
  const box = $("#listenBox");
  const parts: HTMLElement[] = [];
  if (res.available.abs) {
    if (res.links.abs) {
      parts.push(el("div", { class: "listen-row" }, icon("check"),
        el("span", {}, t("Verbunden mit dem Audiobookshelf-Konto „{name}“", { name: res.links.abs.name })),
        el("button", { type: "button", class: "btn small ghost", onclick: () => unlinkMine("abs") }, t("Trennen"))));
    } else {
      const user = el("input", { type: "text", autocomplete: "off", spellcheck: "false", maxlength: "200" });
      const pw = el("input", { type: "password", autocomplete: "off", maxlength: "500" });
      const go = el("button", { type: "button", class: "btn small" }, t("Verbinden"));
      const submit = async (): Promise<void> => {
        go.disabled = true;
        try {
          await api("POST", "/api/me/listening/abs", { username: user.value, password: pw.value });
          toast(t("Audiobookshelf verbunden"));
          openListening();
        } catch (e) { toast(errMsg(e), true); go.disabled = false; }
        pw.value = "";
      };
      go.addEventListener("click", submit);
      pw.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); submit(); } });
      parts.push(el("div", { class: "listen-form" },
        el("b", {}, t("Audiobookshelf verbinden")),
        el("div", { class: "grid2" },
          el("label", {}, t("Benutzername in Audiobookshelf"), user),
          el("label", {}, t("Passwort in Audiobookshelf"), pw)),
        el("div", { class: "row gap" }, go,
          el("span", { class: "muted small" }, t("Das Passwort prüft nur, dass es dein Konto ist – Hörwerk speichert es nicht.")))));
    }
  }
  if (res.available.plex) {
    parts.push(res.links.plex
      ? el("div", { class: "listen-row" }, icon("check"), el("span", {}, t("Plex-Profil „{name}“", { name: res.links.plex.name })),
        el("button", { type: "button", class: "btn small ghost", onclick: () => unlinkMine("plex") }, t("Trennen")))
      : el("div", { class: "listen-row muted small" }, t("Hörst du über Plex? Dein Plex-Profil ordnet ein Admin unter Einstellungen → Konten zu.")));
  }
  const r = res.recent;
  if (r) {
    parts.push(el("p", { class: "muted small" }, r.seconds || r.books
      ? t("Letzte 30 Tage: {time} gehört, {books} – Danke fürs Zuhören!", { time: fmtListen(r.seconds),
        books: plural(r.books, t("1 Buch"), t("{n} Bücher", { n: r.books })) })
      : t("In den letzten 30 Tagen noch nichts gehört.")));
    if (r.errors.length) parts.push(el("p", { class: "err small" }, r.errors.join(" · ")));
  }
  box.replaceChildren(...parts);
}

async function unlinkMine(kind: "abs" | "plex"): Promise<void> {
  try { await api("DELETE", `/api/me/listening/${kind}`); openListening(); } catch (e) { toast(errMsg(e), true); }
}

// -- Einstellungen → Konten: who is who -------------------------------------------------------
interface ListenAccounts {
  available: { abs: boolean; plex: boolean }; abs: ListenLink[] | null; plex: ListenLink[] | null; errors: string[];
  users: { id: number; name: string; links: ListenLinks }[];
}

async function loadListenMap(): Promise<void> {
  let res: ListenAccounts;
  try { res = await api<ListenAccounts>("GET", "/api/listening/accounts"); } catch { return; }
  const fs = $("#listenMapFieldset");
  fs.hidden = !res.available.abs && !res.available.plex;
  if (fs.hidden) return;
  const errs = $("#listenMapErrors");
  errs.hidden = !res.errors.length;
  errs.textContent = res.errors.join(" · ");
  $("#listenMapAbsHead").hidden = !res.available.abs;
  $("#listenMapPlexHead").hidden = !res.available.plex;
  const picker = (u: ListenAccounts["users"][number], kind: "abs" | "plex"): HTMLElement => {
    const options = res[kind];
    const current = u.links[kind];
    const sel = el("select", { "aria-label": kind === "abs" ? t("Audiobookshelf-Konto von {name}", { name: u.name }) : t("Plex-Profil von {name}", { name: u.name }) },
      el("option", { value: "" }, "–"),
      ...(options || (current ? [current] : [])).map((o) => el("option", { value: o.id }, o.name)));
    sel.value = current?.id || "";
    sel.disabled = !options;
    sel.addEventListener("change", async () => {
      try {
        await api("PUT", "/api/listening/link", { user_id: u.id, kind, target_id: sel.value });
        toast(t("Gespeichert"));
      } catch (e) { toast(errMsg(e), true); }
      loadListenMap();                       // one account or profile belongs to one person: others may change
    });
    return sel;
  };
  $("#listenMapRows").replaceChildren(...res.users.map((u) => el("tr", {},
    el("td", {}, el("b", {}, u.name)),
    res.available.abs ? el("td", {}, picker(u, "abs")) : null,
    res.available.plex ? el("td", {}, picker(u, "plex")) : null)));
}

function initListening(): void {
  if (typeof ME === "undefined" || !ME) return;
  document.addEventListener("viewchange", (e) => { if ((e as CustomEvent<string>).detail === "account") openListening(); });
  if (!$("#viewAccount").hidden) openListening();
}

initListening();
