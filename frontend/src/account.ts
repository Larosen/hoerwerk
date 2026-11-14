"use strict";
// ------------------------------------------------------------------------- //
// Own account (/account: password, devices, sign out) and – for admins – the user list (/settings/users)
// ------------------------------------------------------------------------- //
interface SessionInfo { id: string; created: number; last_seen: number; ip: string; agent: string }
interface UserInfo {
  id: number; name: string; role: "admin" | "user"; perms: Record<string, boolean>; quota: number | null;
  disabled: boolean; last_login: number | null; last_seen?: number | null; origin?: string;
}
const PERM_KEYS = ["see_all_requests", "auto_approve", "interactive_search"];

function deviceName(agent: string): string {
  const os = /iPhone|iPad/.test(agent) ? "iOS" : /Android/.test(agent) ? "Android" : /Mac OS/.test(agent) ? "macOS"
    : /Windows/.test(agent) ? "Windows" : /Linux/.test(agent) ? "Linux" : "";
  const browser = /Firefox\//.test(agent) ? "Firefox" : /Edg\//.test(agent) ? "Edge" : /Chrome\//.test(agent) ? "Chrome"
    : /Safari\//.test(agent) ? "Safari" : "";
  return [browser, os].filter(Boolean).join(" · ") || t("Unbekanntes Gerät");
}

const THEME_CARDS: { mode: ThemeMode; name: () => string; hint: () => string }[] = [
  { mode: "auto", name: () => t("Automatisch"), hint: () => t("Folgt dem Gerät") },
  { mode: "light", name: () => t("Hell"), hint: () => t("Papier") },
  { mode: "dark", name: () => t("Dunkel"), hint: () => t("Abend") },
  { mode: "oled", name: () => t("OLED"), hint: () => t("Nacht – echtes Schwarz") },
];

/** Colour world: four cards with a preview, and whether the choice counts for the account or only for this device. */
function renderDisplay(): void {
  const mode = hwTheme.mode(), scope = hwTheme.scope();
  const save = async (m: ThemeMode, where: "device" | "account"): Promise<void> => {
    try { await hwTheme.set(m, where); renderDisplay(); } catch (e) { toast(errMsg(e), true); }
  };
  $("#themeCards").replaceChildren(...THEME_CARDS.map((c) => el("button", {
    type: "button", class: "themecard", "aria-pressed": String(c.mode === mode), onclick: () => { void save(c.mode, scope); },
  }, el("span", { class: "pv pv-" + c.mode }, el("i"), el("i"), el("i")),
  el("span", {}, el("b", {}, c.name()), el("small", {}, c.hint())))));
  const radio = (where: "device" | "account", title: string, hint: string): HTMLElement => el("button", {
    type: "button", class: "radio" + (scope === where ? " on" : ""), role: "radio", "aria-checked": String(scope === where),
    onclick: () => { void save(mode, where); },
  }, el("span", { class: "dot" }), el("span", {}, el("b", {}, title), el("span", { class: "d" }, hint)));
  $("#themeScope").replaceChildren(
    radio("account", t("Alle meine Geräte"), t("iPhone, iPad und Mac zeigen dieselbe Farbwelt – sie wird in deinem Konto gespeichert.")),
    radio("device", t("Nur dieses Gerät"), t("Zum Beispiel OLED auf dem iPhone und Hell am Mac.")));
}

/** How to put Hörwerk on the iPhone home screen – the steps only where they help (iPhone/iPad, not yet installed). */
function renderInstall(): void {
  const box = $("#installBox");
  const standalone = window.matchMedia("(display-mode: standalone)").matches || (navigator as { standalone?: boolean }).standalone === true;
  const ios = /iPhone|iPad|iPod/.test(navigator.userAgent) || (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);
  if (standalone) { box.replaceChildren(el("p", { class: "install-done" }, icon("check"), t("Hörwerk läuft gerade als App."))); return; }
  if (!ios) {
    box.replaceChildren(el("p", { class: "muted small" }, t("Hörwerk lässt sich als App auf den Home-Bildschirm legen: auf dem iPhone und iPad in Safari über Teilen → Zum Home-Bildschirm, in Chrome über das Menü → App installieren.")));
    return;
  }
  // **bold** marks the words to look for on screen – one sentence per step, so it translates as a whole
  const rich = (text: string): (string | HTMLElement)[] => text.split("**").map((part, i) => (i % 2 ? el("b", {}, part) : part));
  box.replaceChildren(
    el("p", { class: "muted small" }, t("So liegt Hörwerk auf dem Home-Bildschirm – ohne Adressleiste, wie eine App:")),
    el("ol", { class: "install-steps" },
      el("li", {}, ...rich(t("Hörwerk in **Safari** öffnen (nicht in einer anderen App)."))),
      el("li", {}, ...rich(t("Unten auf **Teilen** tippen – das Quadrat mit dem Pfeil nach oben."))),
      el("li", {}, ...rich(t("Nach unten wischen und **Zum Home-Bildschirm** wählen."))),
      el("li", {}, ...rich(t("Oben rechts auf **Hinzufügen** tippen.")))),
    el("p", { class: "muted small" }, t("Danach startet Hörwerk mit eigenem Symbol im Vollbild.")));
}

async function openAccount(): Promise<void> {
  renderDisplay();
  renderInstall();
  try {
    const me = await api<{ name: string; role: string; sessions: SessionInfo[]; session: string;
      connection?: { ip: string; peer: string; proxied: boolean; trusted: boolean; https: boolean } }>("GET", "/api/auth/me");
    const c = me.connection;
    if (c) {
      const fact = (k: string, v: string, bad = false): HTMLElement[] => [el("dt", {}, k), el("dd", { class: bad ? "bad" : "" }, v)];
      $("#whoami").replaceChildren(
        ...fact(t("Deine Adresse"), c.ip),
        ...fact(t("Verbindung"), c.proxied ? (c.trusted ? t("über Reverse-Proxy {peer} (vertrauenswürdig)", { peer: c.peer })
          : t("über {peer} – nicht in TRUSTED_PROXIES, die echte Adresse fehlt", { peer: c.peer })) : t("Direkt"), c.proxied && !c.trusted),
        ...fact("HTTPS", c.https ? t("Ja") : t("Nein")));
    }
    $("#accountTitle").textContent = `${me.name} · ${me.role === "admin" ? t("Admin") : t("Nutzer")}`;
    renderTotp(!!(me as { totp?: boolean }).totp);
    api<{ ntfy_topic: string; ntfy_server: boolean }>("GET", "/api/auth/prefs").then((p) => {
      $<HTMLInputElement>("#myTopic").value = p.ntfy_topic;
      $<HTMLInputElement>("#myTopic").disabled = !p.ntfy_server;
      $<HTMLInputElement>("#myTopic").title = p.ntfy_server ? "" : t("Der Admin hat noch keinen ntfy-Server eingerichtet");
    }).catch(() => { /* optional */ });
    $("#sessionList").replaceChildren(...me.sessions.map((s) => el("li", {},
      el("span", {}, el("b", {}, deviceName(s.agent)), " ", el("span", { class: "muted small" }, `${s.ip} · ${fmtAgo(s.last_seen)}`)),
      s.id === me.session ? el("span", { class: "pill" }, t("Dieses Gerät"))
        : el("button", { type: "button", class: "btn small ghost", onclick: async () => {
          await api("POST", "/api/auth/sessions/end", { id: s.id }); openAccount();
        } }, t("Abmelden")))));
  } catch (e) { toast(errMsg(e), true); }
}

async function changePassword(): Promise<void> {
  const old = $<HTMLInputElement>("#pwOld"), pw = $<HTMLInputElement>("#pwNew"), pw2 = $<HTMLInputElement>("#pwNew2");
  if (pw.value !== pw2.value) { toast(t("Die Passwörter stimmen nicht überein"), true); return; }
  try {
    await api("POST", "/api/auth/password", { old: old.value, new: pw.value });
    old.value = pw.value = pw2.value = "";
    toast(t("Passwort geändert – andere Geräte sind abgemeldet"));
    openAccount();
  } catch (e) { toast(errMsg(e), true); }
}

async function logout(): Promise<void> {
  try { await api("POST", "/api/auth/logout"); } catch { /* signed out anyway */ }
  location.replace("/login");
}

// -- second factor ------------------------------------------------------------
function renderTotp(active: boolean): void {
  const box = $("#totpBox");
  if (active) {
    const pw = el("input", { type: "password", placeholder: t("Passwort"), autocomplete: "current-password", maxlength: "256" });
    const code = el("input", { inputmode: "numeric", placeholder: t("Code"), maxlength: "7", autocomplete: "one-time-code", class: "code" });
    box.replaceChildren(el("p", { class: "ok-text" }, icon("check"), t("Aktiv – beim Anmelden fragt Hörwerk nach dem Code aus der App.")),
      el("div", { class: "row gap wrap" }, pw, code, el("button", { type: "button", class: "btn small", onclick: async () => {
        try { await api("POST", "/api/auth/totp/disable", { password: pw.value, code: code.value }); toast(t("Zweitfaktor ausgeschaltet")); renderTotp(false); }
        catch (e) { toast(errMsg(e), true); }
      } }, t("Ausschalten"))));
    return;
  }
  box.replaceChildren(el("p", { class: "muted small" }, t("Beim Anmelden zusätzlich ein Code aus einer Authenticator-App (z. B. Aegis, 2FAS oder Google Authenticator).")),
    el("button", { type: "button", class: "btn small", onclick: async () => {
      try {
        const s = await api<{ secret: string; uri: string }>("POST", "/api/auth/totp/setup");
        const code = el("input", { inputmode: "numeric", placeholder: t("Code aus der App"), maxlength: "7", autocomplete: "one-time-code", class: "code" });
        box.replaceChildren(
          el("p", { class: "small" }, t("In der Authenticator-App ein Konto hinzufügen – auf dem Handy über den Link, sonst den Schlüssel abtippen:")),
          el("p", {}, el("a", { href: s.uri }, t("In der App öffnen")), " · ", el("code", { class: "secret" }, s.secret.replace(/(.{4})/g, "$1 ").trim())),
          el("div", { class: "row gap" }, code, el("button", { type: "button", class: "btn small primary", onclick: async () => {
            try { await api("POST", "/api/auth/totp/confirm", { code: code.value.replace(/\s/g, "") }); toast(t("Zweitfaktor aktiv")); renderTotp(true); }
            catch (e) { toast(errMsg(e), true); }
          } }, t("Bestätigen"))));
        code.focus();
      } catch (e) { toast(errMsg(e), true); }
    } }, t("Einrichten")));
}

async function loadAudit(): Promise<void> {
  await loadList($("#auditList"), async () => {
    const res = await api<{ entries: { time: number; user: string; ip: string; action: string; detail: string }[] }>("GET", "/api/audit");
    $("#auditList").replaceChildren(...(res.entries.length ? res.entries.map((e) => el("li", {},
      el("span", { class: "muted small" }, new Date(e.time * 1000).toLocaleString(LANG, { dateStyle: "short", timeStyle: "short" })),
      el("b", {}, e.user), el("code", {}, e.action), e.detail ? el("span", { class: "muted small" }, e.detail) : null,
      el("span", { class: "muted small" }, e.ip))) : [el("li", { class: "muted" }, t("Noch nichts."))]));
  }, { count: 3, kind: "line" });
}

// -- users (admin) ------------------------------------------------------------
function randomPassword(): string {
  const chars = "abcdefghjkmnpqrstuvwxyzABCDEFGHJKMNPQRSTUVWXYZ23456789";
  for (;;) {                                  // until it meets every rule (nearly always the first one)
    const bytes = crypto.getRandomValues(new Uint8Array(16));
    const pw = Array.from(bytes, (b) => chars[b % chars.length]).join("").replace(/(.{4})(?=.)/g, "$1-");
    if (passwordRules(pw).every((r) => r.ok)) return pw;
  }
}

async function saveUser(id: number, changes: Record<string, unknown>): Promise<void> {
  try { await api("PUT", `/api/users/${id}`, changes); toast(t("Gespeichert")); }
  catch (e) { toast(errMsg(e), true); }
  loadUsers();
}

function userRow(u: UserInfo): HTMLTableRowElement {
  const role = el("select", {}, el("option", { value: "user" }, t("Nutzer")), el("option", { value: "admin" }, t("Admin")));
  role.value = u.role;
  role.addEventListener("change", () => saveUser(u.id, { role: role.value }));
  const perm = (key: string): HTMLTableCellElement => {
    const box = el("input", { type: "checkbox", "aria-label": key });
    box.checked = !!u.perms[key] || u.role === "admin";
    box.disabled = u.role === "admin";
    box.addEventListener("change", () => saveUser(u.id, { perms: { [key]: box.checked } }));
    return el("td", { class: "c-check" }, box);
  };
  const quota = el("input", { type: "number", min: "0", max: "1000", class: "quota", placeholder: "–", "aria-label": t("Monatslimit") });
  quota.value = u.quota === null ? "" : String(u.quota);
  quota.disabled = u.role === "admin";
  quota.addEventListener("change", () => saveUser(u.id, { quota: quota.value === "" ? null : Number(quota.value) }));
  const off = el("input", { type: "checkbox", "aria-label": t("Gesperrt") });
  off.checked = u.disabled;
  off.addEventListener("change", () => saveUser(u.id, { disabled: off.checked }));
  const isMe = typeof ME !== "undefined" && ME?.id === u.id;
  const twoFa = (u as UserInfo & { totp?: boolean }).totp
    ? el("button", { type: "button", class: "btn small ghost", title: t("Zweitfaktor zurücksetzen (z. B. Handy verloren)"), onclick: async () => {
      if (!(await confirmBox(t("Zweitfaktor von {name} ausschalten? Alle Geräte von {name} werden abgemeldet.", { name: u.name }), t("Ausschalten")))) return;
      saveUser(u.id, { totp: false });
    } }, icon("check"))
    : el("span", { class: "muted small" }, "–");
  return el("tr", {},
    el("td", {}, el("b", {}, u.name), isMe ? el("span", { class: "muted small" }, " · " + t("Du")) : null,
      u.origin ? el("div", { class: "muted small" }, t("Über Einladung „{note}“", { note: u.origin })) : null),
    el("td", {}, role),
    ...PERM_KEYS.map(perm),
    el("td", {}, quota),
    el("td", { class: "c-check" }, off),
    el("td", { class: "c-check" }, twoFa),
    el("td", { class: "muted small" }, (u.last_seen ?? u.last_login) ? fmtAgo(u.last_seen ?? u.last_login) : t("Noch nie")),
    el("td", { class: "c-act" },
      el("button", { type: "button", class: "btn small ghost", title: t("Neues Passwort setzen"), onclick: async () => {
        const pw = randomPassword();
        if (!(await confirmBox(t("Neues Passwort für {name}: {pw} – bitte weitergeben. Alle Geräte von {name} werden abgemeldet.", { name: u.name, pw }), t("Setzen")))) return;
        saveUser(u.id, { password: pw });
      } }, t("Passwort")),
      isMe ? null : el("button", { type: "button", class: "icon-btn", title: t("Konto löschen"), onclick: async () => {
        if (!(await confirmBox(t("Konto „{name}“ löschen?", { name: u.name }), t("Löschen")))) return;
        try { await api("DELETE", `/api/users/${u.id}`); } catch (e) { toast(errMsg(e), true); }
        loadUsers();
      } }, icon("x"))));
}

// -- invitations by link (Einstellungen → Konten) ------------------------------------
interface Invite {
  id: number; token: string; note: string; created: number; created_by: string; expires: number | null;
  max_uses: number; uses: number; quota: number | null; used_by: { name: string; time: number }[];
  perms: Record<string, boolean>;
  status: "open" | "used" | "expired" | "revoked";
}
const INVITE_STATUS: Record<Invite["status"], () => string> = {
  open: () => t("Offen"), used: () => t("Benutzt"), expired: () => t("Abgelaufen"), revoked: () => t("Widerrufen"),
};
interface InviteWithLink extends Invite { link: string }

function inviteLink(inv: Invite): string {
  return (inv as InviteWithLink).link;
}

/** Copy to the clipboard – over plain http (LAN) browsers refuse that, then the link is selected for copying by hand. */
async function copyInvite(inv: Invite, input: HTMLInputElement): Promise<void> {
  try {
    await navigator.clipboard.writeText(inviteLink(inv));
    toast(t("Link kopiert – jetzt z. B. in WhatsApp einfügen"));
  } catch {
    input.focus();
    input.select();
    toast(t("Link markiert – jetzt kopieren"));
  }
}

function inviteRow(inv: Invite): HTMLLIElement {
  const active = inv.status === "open";
  const date = (ts: number): string => new Date(ts * 1000).toLocaleDateString(LOCALE, { dateStyle: "medium" });
  const input = el("input", { type: "text", readonly: true, value: inviteLink(inv), "aria-label": t("Einladungslink"),
    onclick: (e: MouseEvent) => (e.target as HTMLInputElement).select() });
  const share = typeof navigator.share === "function" ? el("button", { type: "button", class: "btn small", onclick: () => {
    navigator.share({ title: "Hörwerk", text: t("Deine Einladung zu Hörwerk – Hörbücher entdecken und wünschen:"), url: inviteLink(inv) })
      .catch(() => { /* closed */ });
  } }, t("Teilen")) : null;
  return el("li", { class: active ? "" : "inactive" },
    el("div", { class: "invite-head" },
      el("b", {}, inv.note || t("Einladung")),
      el("span", { class: `rtag ${active ? "ok" : ""}` }, INVITE_STATUS[inv.status]()),
      el("span", { class: "muted small" }, [
        inv.expires ? t("Gültig bis {date}", { date: date(inv.expires) }) : t("Ohne Ablauf"),
        inv.max_uses === 0 ? t("Für beliebig viele Personen") : plural(inv.max_uses, t("Für 1 Person"), t("Für {n} Personen", { n: inv.max_uses })),
        inv.uses === 0 ? t("Noch niemand angemeldet") : plural(inv.uses, t("1 angemeldet"), t("{n} angemeldet", { n: inv.uses })),
        inv.used_by.length ? t("von {names}", { names: inv.used_by.map((u) => u.name).join(", ") }) : "",
      ].filter(Boolean).join(" · "))),
    el("span", { class: "muted small" }, t("Startrechte: {rights}", { rights: inviteRights(inv) })),
    active ? el("div", { class: "invite-link" }, input,
      el("button", { type: "button", class: "btn small primary", onclick: () => copyInvite(inv, input) }, t("Kopieren")), share) : null,
    el("div", { class: "row gap" },
      active ? el("button", { type: "button", class: "btn small ghost", onclick: async () => {
        if (!(await confirmBox(t("Einladung widerrufen? Der Link funktioniert dann nicht mehr."), t("Widerrufen")))) return;
        try { await api("POST", `/api/invites/${inv.id}/revoke`); } catch (e) { toast(errMsg(e), true); }
        loadInvites();
      } }, t("Widerrufen")) : el("button", { type: "button", class: "btn small ghost", onclick: async () => {
        try { await api("DELETE", `/api/invites/${inv.id}`); } catch (e) { toast(errMsg(e), true); }
        loadInvites();
      } }, t("Aus der Liste entfernen"))));
}

const PERM_NAMES: Record<string, () => string> = {
  auto_approve: () => t("ohne Freigabe"), see_all_requests: () => t("alle Anfragen sehen"),
  interactive_search: () => t("interaktive Suche"),
};

function inviteRights(inv: Invite): string {
  const on = Object.keys(PERM_NAMES).filter((p) => inv.perms?.[p]).map((p) => PERM_NAMES[p]());
  return on.length ? on.join(", ") : t("nur anfragen, mit Freigabe");
}

async function loadInvites(): Promise<void> {
  await loadList($("#inviteList"), async () => {
    const res = await api<{ invites: Invite[]; public_url: string }>("GET", "/api/invites");
    // links always point to the address from the internet – without it there are no links
    $("#invitePublic").hidden = !!res.public_url;
    $<HTMLButtonElement>("#inviteCreate").disabled = !res.public_url;
    $("#inviteList").replaceChildren(...res.invites.map(inviteRow));
  }, { count: 2, kind: "line" });
}

async function createInvite(): Promise<void> {
  const quota = $<HTMLInputElement>("#inviteQuota").value;
  try {
    const inv = await api<Invite>("POST", "/api/invites", {
      note: $<HTMLInputElement>("#inviteNote").value, days: Number($<HTMLSelectElement>("#inviteDays").value),
      uses: Number($<HTMLSelectElement>("#inviteUses").value), quota: quota === "" ? null : Number(quota),
      perms: Object.fromEntries([...document.querySelectorAll<HTMLInputElement>("#invitePerms input[data-perm]")]
        .map((box) => [box.dataset.perm || "", box.checked])),
    });
    $<HTMLInputElement>("#inviteNote").value = "";
    await loadInvites();
    const input = document.querySelector<HTMLInputElement>("#inviteList li:first-child input");
    if (input) copyInvite(inv, input);
  } catch (e) { toast(errMsg(e), true); }
}

/** Addresses Hörwerk blocked after failed logins – visible and liftable here, no terminal needed. */
async function loadBlocked(): Promise<void> {
  const list = $("#blockedList");
  let res: { blocked: { address: string; fails: number; until: number }[] };
  try { res = await api("GET", "/api/security/blocked"); } catch (e) { list.replaceChildren(el("li", { class: "err small" }, errMsg(e))); return; }
  if (!res.blocked.length) { list.replaceChildren(el("li", { class: "muted small" }, t("Gerade ist keine Adresse gesperrt."))); return; }
  list.replaceChildren(...res.blocked.map((b) => {
    const lift = el("button", { type: "button", class: "btn small" }, t("Freigeben"));
    lift.addEventListener("click", async () => {
      try { await api("DELETE", "/api/security/blocked", { address: b.address }); toast(t("{address} ist wieder frei", { address: b.address })); loadBlocked(); }
      catch (e) { toast(errMsg(e), true); }
    });
    const until = new Date(b.until * 1000).toLocaleTimeString(LOCALE, { hour: "2-digit", minute: "2-digit" });
    return el("li", {}, el("code", {}, b.address),
      el("span", { class: "muted small" }, t("{n} Fehlversuche · frei ab {time}", { n: b.fails, time: until })), lift);
  }));
}

async function loadUsers(): Promise<void> {
  void loadInvites();
  void loadBlocked();
  await loadList($("#userRows"), async () => {
    const res = await api<{ users: UserInfo[]; default_quota: number }>("GET", "/api/users");
    $("#defaultQuota").textContent = String(res.default_quota);
    $("#userRows").replaceChildren(...res.users.map(userRow));
  }, { count: 3, kind: "line" });
}

async function createUser(): Promise<void> {
  const name = $<HTMLInputElement>("#newUserName"), pw = $<HTMLInputElement>("#newUserPw");
  const quota = $<HTMLInputElement>("#newUserQuota").value;
  try {
    await api("POST", "/api/users", { name: name.value, password: pw.value, role: $<HTMLSelectElement>("#newUserRole").value,
      quota: quota === "" ? null : Number(quota) });
    toast(t("Konto „{name}“ angelegt", { name: name.value }));
    name.value = pw.value = "";
    loadUsers();
  } catch (e) { toast(errMsg(e), true); }
}

/** "Protokoll" in the phone menu shows how many errors the last 24 hours had. */
async function refreshMoreBadge(): Promise<void> {
  if (!IS_ADMIN) return;
  const badge = $("#moreLogBadge");
  try {
    const res = await api<{ entries: { time: number }[] }>("GET", "/api/logs?after=0&level=error");
    const since = Date.now() / 1000 - 86400;
    const n = res.entries.filter((e) => e.time >= since).length;
    badge.hidden = n === 0;
    badge.textContent = plural(n, t("1 Fehler"), t("{n} Fehler", { n }));
  } catch { badge.hidden = true; }
}

function initAccount(): void {
  if (typeof ME === "undefined" || !ME) return;
  $("#accountName").textContent = ME.name;
  // phone menu "Mehr": who is signed in, sign out, and a red note when something failed lately
  $("#moreAccount").replaceChildren(ME.name, el("small", {}, `${t("Konto")} · ${IS_ADMIN ? t("Admin") : t("Nutzer")}`));
  $("#moreLogout").addEventListener("click", logout);
  document.addEventListener("viewchange", (e) => { if ((e as CustomEvent<string>).detail === "more") void refreshMoreBadge(); });
  $("#pwSave").addEventListener("click", changePassword);
  attachPasswordRules($<HTMLInputElement>("#pwNew"), () => ME?.name || "");
  $("#endOthers").addEventListener("click", async () => {
    try { const r = await api<{ ended: number }>("POST", "/api/auth/sessions/end", {}); toast(plural(r.ended, t("1 Gerät abgemeldet"), t("{n} Geräte abgemeldet", { n: r.ended }))); openAccount(); }
    catch (e) { toast(errMsg(e), true); }
  });
  $("#logoutBtn").addEventListener("click", logout);
  $("#myTopicTest").addEventListener("click", async () => {
    try {
      const r = await api<{ ok: boolean; message: string }>("POST", "/api/auth/prefs/test", { ntfy_topic: $<HTMLInputElement>("#myTopic").value });
      toast((r.ok ? "✓ " : "✗ ") + r.message, !r.ok);
    } catch (e) { toast(errMsg(e), true); }
  });
  $("#myTopicSave").addEventListener("click", async () => {
    try { await api("PUT", "/api/auth/prefs", { ntfy_topic: $<HTMLInputElement>("#myTopic").value }); toast(t("Gespeichert")); }
    catch (e) { toast(errMsg(e), true); }
  });
  document.addEventListener("viewchange", (e) => { if ((e as CustomEvent<string>).detail === "account") openAccount(); });
  // admins that still need a second factor (setting): straight to the account page
  if (IS_ADMIN && ME && !(ME as Me & { totp?: boolean }).totp) {
    api("GET", "/api/sources").catch((e) => { if (/Zweitfaktor|second factor/i.test(errMsg(e))) navigate("/account"); });
  }
  if (!IS_ADMIN) return;
  $(".audit-box").addEventListener("toggle", (e) => { if ((e.target as HTMLDetailsElement).open) loadAudit(); });
  $("#newUserPwGen").addEventListener("click", () => { $<HTMLInputElement>("#newUserPw").value = randomPassword(); });
  $("#newUserSave").addEventListener("click", createUser);
  $("#inviteCreate").addEventListener("click", createInvite);
  // how many people may sign up through one link: 1 … 20, 30, or as many as like
  $("#inviteUses").replaceChildren(
    ...[...Array.from({ length: 20 }, (_, i) => i + 1), 30].map((n) =>
      el("option", { value: String(n) }, n === 1 ? t("1 Person") : t("{n} Personen", { n }))),
    el("option", { value: "0" }, t("Unbegrenzt")));
  // security: admins without a second factor only reach their own account (saved at once)
  const totpRequired = $<HTMLInputElement>("#adminTotpRequired");
  api<Settings>("GET", "/api/settings").then((s) => { totpRequired.checked = !!s.admin_totp_required; }).catch(() => { /* stays off */ });
  totpRequired.addEventListener("change", async () => {
    try {
      await api("PUT", "/api/settings", { admin_totp_required: totpRequired.checked });
      toast(totpRequired.checked ? t("Admins brauchen jetzt einen Zweitfaktor") : t("Zweitfaktor für Admins ist wieder freiwillig"));
    } catch (e) { totpRequired.checked = !totpRequired.checked; toast(errMsg(e), true); }
  });
  attachPasswordRules($<HTMLInputElement>("#newUserPw"), () => $<HTMLInputElement>("#newUserName").value);
}

initAccount();
