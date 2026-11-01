"use strict";
// ------------------------------------------------------------------------- //
// Own account (/account: password, devices, sign out) and – for admins – the user list (/settings/users)
// ------------------------------------------------------------------------- //
interface SessionInfo { id: string; created: number; last_seen: number; ip: string; agent: string }
interface UserInfo {
  id: number; name: string; role: "admin" | "user"; perms: Record<string, boolean>; quota: number | null;
  disabled: boolean; last_login: number | null;
}
const PERM_KEYS = ["see_all_requests", "auto_approve", "interactive_search"];

function deviceName(agent: string): string {
  const os = /iPhone|iPad/.test(agent) ? "iOS" : /Android/.test(agent) ? "Android" : /Mac OS/.test(agent) ? "macOS"
    : /Windows/.test(agent) ? "Windows" : /Linux/.test(agent) ? "Linux" : "";
  const browser = /Firefox\//.test(agent) ? "Firefox" : /Edg\//.test(agent) ? "Edge" : /Chrome\//.test(agent) ? "Chrome"
    : /Safari\//.test(agent) ? "Safari" : "";
  return [browser, os].filter(Boolean).join(" · ") || t("Unbekanntes Gerät");
}

async function openAccount(): Promise<void> {
  try {
    const me = await api<{ name: string; role: string; sessions: SessionInfo[]; session: string;
      connection?: { ip: string; peer: string; proxied: boolean; trusted: boolean; https: boolean } }>("GET", "/api/auth/me");
    const c = me.connection;
    if (c) {
      const fact = (k: string, v: string, bad = false): HTMLElement[] => [el("dt", {}, k), el("dd", { class: bad ? "bad" : "" }, v)];
      $("#whoami").replaceChildren(
        ...fact(t("Deine Adresse"), c.ip),
        ...fact(t("Verbindung"), c.proxied ? (c.trusted ? t("über Reverse-Proxy {peer} (vertrauenswürdig)", { peer: c.peer })
          : t("über {peer} – nicht in TRUSTED_PROXIES, die echte Adresse fehlt", { peer: c.peer })) : t("direkt"), c.proxied && !c.trusted),
        ...fact("HTTPS", c.https ? t("ja") : t("nein")));
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
      s.id === me.session ? el("span", { class: "pill" }, t("dieses Gerät"))
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
  box.replaceChildren(el("p", { class: "muted small" }, t("Schützt dein Konto, auch wenn jemand dein Passwort kennt: beim Anmelden zusätzlich ein Code aus einer Authenticator-App (z. B. Aegis, 2FAS, Google oder Microsoft Authenticator).")),
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
  try {
    const res = await api<{ entries: { time: number; user: string; ip: string; action: string; detail: string }[] }>("GET", "/api/audit");
    $("#auditList").replaceChildren(...(res.entries.length ? res.entries.map((e) => el("li", {},
      el("span", { class: "muted small" }, new Date(e.time * 1000).toLocaleString(LANG, { dateStyle: "short", timeStyle: "short" })),
      el("b", {}, e.user), el("code", {}, e.action), e.detail ? el("span", { class: "muted small" }, e.detail) : null,
      el("span", { class: "muted small" }, e.ip))) : [el("li", { class: "muted" }, t("Noch nichts."))]));
  } catch (e) { toast(errMsg(e), true); }
}

// -- users (admin) ------------------------------------------------------------
function randomPassword(): string {
  const chars = "abcdefghjkmnpqrstuvwxyzABCDEFGHJKMNPQRSTUVWXYZ23456789";
  const bytes = crypto.getRandomValues(new Uint8Array(14));
  return Array.from(bytes, (b) => chars[b % chars.length]).join("").replace(/(.{4})(?=.)/g, "$1-");
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
  const quota = el("input", { type: "number", min: "0", max: "1000", class: "quota", placeholder: "–", "aria-label": t("Kontingent") });
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
    el("td", {}, el("b", {}, u.name), isMe ? el("span", { class: "muted small" }, " · " + t("du")) : null),
    el("td", {}, role),
    ...PERM_KEYS.map(perm),
    el("td", {}, quota),
    el("td", { class: "c-check" }, off),
    el("td", { class: "c-check" }, twoFa),
    el("td", { class: "muted small" }, u.last_login ? fmtAgo(u.last_login) : t("noch nie")),
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

async function loadUsers(): Promise<void> {
  try {
    const res = await api<{ users: UserInfo[]; default_quota: number }>("GET", "/api/users");
    $("#defaultQuota").textContent = String(res.default_quota);
    $("#userRows").replaceChildren(...res.users.map(userRow));
  } catch (e) { toast(errMsg(e), true); }
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

function initAccount(): void {
  if (typeof ME === "undefined" || !ME) return;
  $("#accountName").textContent = ME.name;
  $("#pwSave").addEventListener("click", changePassword);
  $("#endOthers").addEventListener("click", async () => {
    try { const r = await api<{ ended: number }>("POST", "/api/auth/sessions/end", {}); toast(t("{n} Geräte abgemeldet", { n: r.ended })); openAccount(); }
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
}

initAccount();
