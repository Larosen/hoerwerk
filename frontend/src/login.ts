"use strict";
// ------------------------------------------------------------------------- //
// Login page: sign in, or create the first admin account
// ------------------------------------------------------------------------- //
interface AuthStatus { setup: boolean; setup_code: boolean; user: { name: string } | null }

function showError(text: string): void {
  const box = $("#loginError");
  box.textContent = text;
  box.hidden = !text;
}

let ticket = "";

/** Where to go after signing in: the page that was asked for – only own pages, never another site. */
function nextPage(): string {
  const next = new URLSearchParams(location.search).get("next") || "";
  return /^\/(books|search|discover|requests|account|settings(\/[a-z]+)?)$/.test(next) ? next : "/";
}

async function submitForm(form: HTMLFormElement, url: string, body: Record<string, unknown>): Promise<void> {
  const btn = form.querySelector<HTMLButtonElement>("button[type=submit]");
  if (btn) btn.disabled = true;
  showError("");
  try {
    const res = await api<{ ok: boolean; totp?: boolean; ticket?: string }>("POST", url, body);
    if (res.totp && res.ticket) {
      // password right – now the code from the authenticator app
      ticket = res.ticket;
      form.hidden = true;
      const totp = $<HTMLFormElement>("#totpForm");
      totp.hidden = false;
      field(totp, "code").focus();
      return;
    }
    location.replace(nextPage());
  } catch (e) {
    showError(errMsg(e));
    const pw = form.querySelector<HTMLInputElement>("input[type=password]");
    if (pw) { pw.value = ""; pw.focus(); }
  } finally { if (btn) btn.disabled = false; }
}

async function initLogin(): Promise<void> {
  let st: AuthStatus;
  try { st = await api<AuthStatus>("GET", "/api/auth/status"); } catch (e) { showError(errMsg(e)); return; }
  if (st.user) { location.replace(nextPage()); return; }
  const login = $<HTMLFormElement>("#loginForm");
  const setup = $<HTMLFormElement>("#setupForm");
  login.hidden = st.setup;
  setup.hidden = !st.setup;
  $("#setupCodeRow").hidden = !st.setup_code;
  if (st.setup) field(setup, "name").focus(); else field(login, "name").focus();

  login.addEventListener("submit", (e) => {
    e.preventDefault();
    submitForm(login, "/api/auth/login", {
      name: field(login, "name").value, password: field(login, "password").value, remember: field(login, "remember").checked,
    });
  });
  const totp = $<HTMLFormElement>("#totpForm");
  totp.addEventListener("submit", (e) => {
    e.preventDefault();
    submitForm(totp, "/api/auth/login/totp", { ticket, code: field(totp, "code").value.replace(/\s/g, "") });
  });
  setup.addEventListener("submit", (e) => {
    e.preventDefault();
    if (field(setup, "password").value !== field(setup, "password2").value) { showError(t("Die Passwörter stimmen nicht überein")); return; }
    submitForm(setup, "/api/auth/setup", {
      name: field(setup, "name").value, password: field(setup, "password").value, code: field(setup, "code").value,
    });
  });
}

initLogin();
