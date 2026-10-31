"use strict";
// ------------------------------------------------------------------------- //
// Own account (password, devices, sign out) and – for admins – the user list
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
    const me = await api<{ name: string; role: string; sessions: SessionInfo[]; session: string }>("GET", "/api/auth/me");
    $("#accountTitle").textContent = `${me.name} · ${me.role === "admin" ? t("Admin") : t("Nutzer")}`;
    $("#sessionList").replaceChildren(...me.sessions.map((s) => el("li", {},
      el("span", {}, el("b", {}, deviceName(s.agent)), " ", el("span", { class: "muted small" }, `${s.ip} · ${fmtAgo(s.last_seen)}`)),
      s.id === me.session ? el("span", { class: "pill" }, t("dieses Gerät"))
        : el("button", { type: "button", class: "btn small ghost", onclick: async () => {
          await api("POST", "/api/auth/sessions/end", { id: s.id }); openAccount();
        } }, t("Abmelden")))));
    const d = $<HTMLDialogElement>("#accountDialog");
    if (!d.open) d.showModal();
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
  return el("tr", {},
    el("td", {}, el("b", {}, u.name), isMe ? el("span", { class: "muted small" }, " · " + t("du")) : null),
    el("td", {}, role),
    ...PERM_KEYS.map(perm),
    el("td", {}, quota),
    el("td", { class: "c-check" }, off),
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
  $("#accountBtn").addEventListener("click", openAccount);
  $("#pwSave").addEventListener("click", changePassword);
  $("#endOthers").addEventListener("click", async () => {
    try { const r = await api<{ ended: number }>("POST", "/api/auth/sessions/end", {}); toast(t("{n} Geräte abgemeldet", { n: r.ended })); openAccount(); }
    catch (e) { toast(errMsg(e), true); }
  });
  $("#logoutBtn").addEventListener("click", logout);
  if (!IS_ADMIN) return;
  $("#usersBtn").addEventListener("click", () => { loadUsers(); $<HTMLDialogElement>("#usersDialog").showModal(); });
  $("#newUserPwGen").addEventListener("click", () => { $<HTMLInputElement>("#newUserPw").value = randomPassword(); });
  $("#newUserSave").addEventListener("click", createUser);
}

initAccount();
