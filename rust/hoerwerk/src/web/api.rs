//! The API: the route table comes from contract/routes.json (method, path, who may call it). Every route exists
//! with its access rule from the start; those not built in Rust yet answer 501 – after the same checks.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::body::Bytes;
use axum::http::{Method, StatusCode};
use serde_json::{Map, Value, json};

use super::{Reply, Req, error, json_reply};
use crate::accounts::{SESSION_LONG, me};
use crate::app::App;
use crate::text::{self, t};

type Handler = fn(Arc<App>, Req, HashMap<String, String>) -> Pin<Box<dyn Future<Output = Reply> + Send>>;

pub struct Route {
    pub method: Method,
    pub parts: Vec<String>,
    pub access: String,
    pub raw_body: bool,
    pub handler: Handler,
}

macro_rules! h {
    ($f:path) => {
        |app, req, params| Box::pin($f(app, req, params))
    };
}

fn handler_for(method: &str, path: &str) -> Handler {
    match (method, path) {
        ("GET", "/api/health") => h!(health),
        ("GET", "/api/auth/status") => h!(auth_status),
        ("POST", "/api/auth/setup") => h!(setup),
        ("POST", "/api/auth/login") => h!(login),
        ("POST", "/api/auth/logout") => h!(logout),
        ("GET", "/api/auth/me") => h!(auth_me),
        ("POST", "/api/auth/password") => h!(password),
        ("POST", "/api/auth/sessions/end") => h!(sessions_end),
        ("GET", "/api/auth/prefs") => h!(prefs_get),
        ("PUT", "/api/auth/prefs") => h!(prefs_put),
        ("GET", "/api/users") => h!(users_list),
        ("POST", "/api/users") => h!(users_create),
        ("PUT", "/api/users/{uid}") => h!(users_update),
        ("DELETE", "/api/users/{uid}") => h!(users_delete),
        ("GET", "/api/audit") => h!(audit),
        ("GET", "/api/settings") => h!(settings_get),
        ("PUT", "/api/settings") => h!(settings_put),
        ("GET", "/api/logs") => h!(logs),
        ("GET", "/api/events") => h!(events),
        ("GET", "/api/system") => h!(system),
        ("GET", "/api/invites") => h!(invites_list),
        ("POST", "/api/invites") => h!(invites_create),
        ("POST", "/api/invites/{iid}/revoke") => h!(invites_revoke),
        ("DELETE", "/api/invites/{iid}") => h!(invites_delete),
        ("GET", "/api/invite/{token}") => h!(invite_check),
        ("POST", "/api/invite/{token}") => h!(invite_accept),
        ("GET", "/api/sources") => h!(super::requests::sources_get),
        ("PUT", "/api/sources") => h!(super::requests::sources_put),
        ("POST", "/api/sources/test") => h!(super::requests::sources_test),
        ("GET", "/api/downloads") => h!(super::requests::downloads),
        ("POST", "/api/downloads/clear") => h!(super::requests::downloads_clear),
        ("GET", "/api/requests") => h!(super::requests::list),
        ("POST", "/api/requests") => h!(super::requests::create),
        ("GET", "/api/requests/keys") => h!(super::requests::keys),
        ("POST", "/api/requests/series") => h!(super::requests::create_series),
        ("GET", "/api/requests/{rid}") => h!(super::requests::one),
        ("DELETE", "/api/requests/{rid}") => h!(super::requests::withdraw),
        ("POST", "/api/requests/{rid}/approve") => h!(super::requests::approve),
        ("POST", "/api/requests/{rid}/decline") => h!(super::requests::decline),
        ("POST", "/api/requests/{rid}/retry") => h!(super::requests::retry),
        ("POST", "/api/requests/{rid}/releases") => h!(super::requests::releases),
        ("POST", "/api/requests/{rid}/grab") => h!(super::requests::grab),
        ("POST", "/api/requests/{rid}/block") => h!(super::requests::block),
        ("GET", "/api/stats") => h!(super::requests::stats),
        ("GET", "/api/stats/accounts/{uid}") => h!(super::requests::stats_account),
        ("GET", "/api/notify") => h!(super::requests::notify_get),
        ("PUT", "/api/notify") => h!(super::requests::notify_put),
        ("POST", "/api/notify/test") => h!(super::requests::notify_test),
        ("POST", "/api/auth/prefs/test") => h!(super::requests::prefs_test),
        ("GET", "/api/library") => h!(super::library::stats),
        ("GET", "/api/library/books") => h!(super::library::books),
        ("GET", "/api/library/cover") => h!(super::library::cover),
        ("POST", "/api/library/scan") => h!(super::library::scan),
        ("GET", "/api/library/unconverted") => h!(super::library::unconverted),
        ("GET", "/api/library/tracks") => h!(super::library::tracks),
        ("GET", "/api/library/cleanup") => h!(super::library::cleanup_preview),
        ("POST", "/api/library/cleanup") => h!(super::library::cleanup_run),
        ("GET", "/api/audiobooks") => h!(super::audiobooks::status),
        ("DELETE", "/api/audiobooks") => h!(super::audiobooks::clear),
        ("GET", "/api/audiobooks/paths") => h!(super::audiobooks::paths),
        ("POST", "/api/audiobooks/scan") => h!(super::audiobooks::scan),
        ("GET", "/api/audiobooks/{job_id}") => h!(super::audiobooks::detail),
        ("DELETE", "/api/audiobooks/{job_id}") => h!(super::audiobooks::remove),
        ("POST", "/api/audiobooks/{job_id}/search") => h!(super::audiobooks::search),
        ("POST", "/api/audiobooks/{job_id}/link") => h!(super::audiobooks::link),
        ("POST", "/api/audiobooks/{job_id}/duplicate") => h!(super::audiobooks::duplicate),
        ("POST", "/api/audiobooks/{job_id}/reopen") => h!(super::audiobooks::reopen),
        ("POST", "/api/audiobooks/{job_id}/choose") => h!(super::audiobooks::choose),
        ("POST", "/api/audiobooks/{job_id}/manual") => h!(super::audiobooks::manual),
        ("POST", "/api/audiobooks/{job_id}/retry") => h!(super::audiobooks::retry),
        ("POST", "/api/audiobooks/{job_id}/cancel") => h!(super::audiobooks::cancel),
        ("POST", "/api/audiobooks/{job_id}/delete-source") => h!(super::audiobooks::delete_source),
        ("POST", "/api/audiobooks/{job_id}/ignore") => h!(super::audiobooks::ignore),
        ("POST", "/api/inbox/unignore") => h!(super::audiobooks::unignore),
        ("POST", "/api/library/convert") => h!(super::audiobooks::convert),
        ("POST", "/api/upload") => h!(super::upload::start),
        ("PUT", "/api/upload/{uid}") => h!(super::upload::put_file),
        ("POST", "/api/upload/{uid}/finish") => h!(super::upload::finish),
        ("DELETE", "/api/upload/{uid}") => h!(super::upload::cancel),
        ("POST", "/api/auth/login/totp") => h!(login_totp),
        ("POST", "/api/auth/totp/setup") => h!(totp_setup),
        ("POST", "/api/auth/totp/confirm") => h!(totp_confirm),
        ("POST", "/api/auth/totp/disable") => h!(totp_disable),
        _ => h!(not_yet),
    }
}

fn routes() -> &'static Vec<Route> {
    static ROUTES: OnceLock<Vec<Route>> = OnceLock::new();
    ROUTES.get_or_init(|| {
        let spec: Value = serde_json::from_str(include_str!("../../../../contract/routes.json")).expect("routes.json");
        spec["routes"]
            .as_array()
            .expect("routes")
            .iter()
            .map(|r| {
                let (m, p) = (r["method"].as_str().unwrap_or("GET"), r["path"].as_str().unwrap_or("/"));
                Route {
                    method: Method::from_bytes(m.as_bytes()).expect("method"),
                    parts: p.split('/').filter(|x| !x.is_empty()).map(String::from).collect(),
                    access: r["access"].as_str().unwrap_or("admin").to_string(),
                    raw_body: r["raw_body"].as_bool().unwrap_or(false),
                    handler: handler_for(m, p),
                }
            })
            .collect()
    })
}

/// The route and its path parameters (``{uid}`` → value).
pub fn find(method: &Method, path: &str) -> Option<(&'static Route, HashMap<String, String>)> {
    let parts: Vec<&str> = path.split('/').filter(|x| !x.is_empty()).collect();
    'routes: for r in routes() {
        if r.method != method || r.parts.len() != parts.len() {
            continue;
        }
        let mut params = HashMap::new();
        for (want, got) in r.parts.iter().zip(&parts) {
            if let Some(name) = want.strip_prefix('{').and_then(|w| w.strip_suffix('}')) {
                params.insert(name.to_string(), (*got).to_string());
            } else if want != got {
                continue 'routes;
            }
        }
        return Some((r, params));
    }
    None
}

async fn not_yet(_: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    error(
        501,
        format!("{} {} – im Rust-Backend noch nicht umgesetzt", req.method, req.path),
    )
}

fn cookie(req: &Req, token: &str, persistent: bool) -> String {
    let mut parts = vec![
        format!("{}={token}", super::COOKIE),
        "Path=/".into(),
        "HttpOnly".into(),
        "SameSite=Lax".into(),
    ];
    if req.https {
        parts.push("Secure".into());
    }
    if persistent {
        parts.push(format!("Max-Age={}", SESSION_LONG as i64));
    }
    parts.join("; ")
}

fn with_cookie(v: Value, cookie: String) -> Reply {
    Reply::Json(StatusCode::OK, v, vec![("set-cookie".into(), cookie)])
}

fn id_param(params: &HashMap<String, String>, key: &str) -> Option<i64> {
    params.get(key).and_then(|v| v.parse().ok())
}

fn home(req: &Req) -> bool {
    !req.proxied
        && req.ip.parse::<std::net::IpAddr>().is_ok_and(|ip| match ip {
            std::net::IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
            std::net::IpAddr::V6(v6) => {
                v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80
            }
        })
}

// -- system ---------------------------------------------------------------------------
async fn health(_: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    if req.user.is_some() {
        json_reply(json!({"ok": true, "version": crate::settings::version()}))
    } else {
        json_reply(json!({"ok": true}))
    }
}

async fn system(_: Arc<App>, _: Req, _: HashMap<String, String>) -> Reply {
    json_reply(
        json!({"version": crate::settings::version(), "backend": "rust", "ffmpeg": "",
                      "sandbox": {"mode": "auto", "landlock_abi": 0, "active": false, "network_blocked": false},
                      "queues": {"encode": {"workers": 1, "running": 0, "waiting": 0}, "io": {"workers": 2, "running": 0, "waiting": 0}}}),
    )
}

async fn logs(_: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let after = req.query.get("after").and_then(|v| v.parse().ok()).unwrap_or(0);
    let level = req.query.get("level").map(String::as_str).unwrap_or("info");
    let (entries, last) = text::since(after, level, 1000);
    json_reply(json!({"entries": entries, "last": last}))
}

async fn settings_get(app: Arc<App>, _: Req, _: HashMap<String, String>) -> Reply {
    json_reply(Value::Object(app.settings()))
}

async fn settings_put(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let data = req.body.as_object().cloned().unwrap_or_default();
    let mut clean = match app.settings.clean(&data) {
        Ok(c) => c,
        Err(key) => return error(400, t("Ungültiger Wert für {key}", &[("key", &key)])),
    };
    if let Some(Value::String(raw)) = clean.get("public_url").cloned() {
        let url = crate::settings::public_url(&raw);
        if !raw.trim().is_empty() && url.is_empty() {
            return error(
                400,
                t(
                    "Öffentliche Adresse: bitte so angeben: https://hoerwerk.example.de",
                    &[],
                ),
            );
        }
        clean.insert("public_url".into(), json!(url));
    }
    match app.settings.update(&app.db, clean) {
        Ok(s) => {
            if let Some(lang) = s.get("language").and_then(Value::as_str) {
                text::set_language(lang);
            }
            json_reply(Value::Object(s))
        }
        Err(e) => error(500, e.to_string()),
    }
}

async fn events(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let wanted = req
        .query
        .get("topics")
        .cloned()
        .unwrap_or_else(|| app.topics().join(","));
    let topics: Vec<String> = wanted
        .split(',')
        .filter(|x| app.topics().contains(x))
        .map(String::from)
        .collect();
    if topics.is_empty() {
        return error(400, t("Unbekanntes Thema", &[]));
    }
    if !app.sse_open() {
        return error(503, t("Zu viele offene Verbindungen", &[]));
    }
    Reply::Stream(Box::new(Box::pin(live_stream(app, topics))))
}

struct SseGuard(Arc<App>);

impl Drop for SseGuard {
    fn drop(&mut self) {
        self.0.sse_clients.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// First the full state of each topic, then every change (at most every 0.5 s), a heartbeat every 15 s;
/// ends when Hörwerk shuts down.
fn live_stream(
    app: Arc<App>,
    topics: Vec<String>,
) -> impl futures_util::Stream<Item = Result<Bytes, std::convert::Infallible>> {
    let guard = SseGuard(app.clone());
    let shutdown = app.shutdown.subscribe();
    let mut stop = app.shutdown.subscribe();
    futures_util::stream::unfold(
        (app, topics, HashMap::<String, u64>::new(), true, guard),
        move |(app, topics, mut seen, first, guard)| {
            let mut shutdown = shutdown.clone();
            async move {
                if *shutdown.borrow() {
                    return None;
                }
                if first {
                    let mut out = String::from("retry: 3000\n\n");
                    for topic in &topics {
                        seen.insert(topic.clone(), app.version(topic));
                        out.push_str(&format!("event: {topic}\ndata: {}\n\n", app.snapshot(topic)));
                    }
                    return Some((Ok(Bytes::from(out)), (app, topics, seen, false, guard)));
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
                let changed: Vec<String> = topics
                    .iter()
                    .filter(|x| seen.get(*x) != Some(&app.version(x)))
                    .cloned()
                    .collect();
                if changed.is_empty() {
                    tokio::select! {
                        _ = app.changed.notified() => {}
                        _ = tokio::time::sleep(Duration::from_secs(15)) => {
                            return Some((Ok(Bytes::from_static(b": ping\n\n")), (app, topics, seen, false, guard)));
                        }
                        _ = shutdown.changed() => return None,
                    }
                }
                let mut out = String::new();
                for topic in &topics {
                    let v = app.version(topic);
                    if seen.get(topic) != Some(&v) {
                        seen.insert(topic.clone(), v);
                        out.push_str(&format!("event: {topic}\ndata: {}\n\n", app.snapshot(topic)));
                    }
                }
                Some((
                    Ok(Bytes::from(if out.is_empty() { ": ping\n\n".to_string() } else { out })),
                    (app, topics, seen, false, guard),
                ))
            }
        },
    )
    .take_while(move |_| {
        let stopped = *stop.borrow_and_update();
        std::future::ready(!stopped)
    })
}

use futures_util::StreamExt;

// -- accounts -------------------------------------------------------------------------
async fn auth_status(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let setup = app.accounts.count(&app.db) == 0;
    json_reply(json!({"setup": setup, "setup_code": setup && !home(&req), "user": req.user.as_ref().map(me)}))
}

async fn setup(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    if app.accounts.count(&app.db) > 0 {
        return error(409, t("Es gibt schon ein Konto – bitte anmelden", &[]));
    }
    if !home(&req) && req.str("code").trim().to_uppercase() != app.setup_code {
        tokio::time::sleep(Duration::from_secs(1)).await;
        return error(403, t("Einrichtungs-Code falsch (steht im Container-Log)", &[]));
    }
    match app.accounts.create(
        &app.db,
        &req.str("name"),
        &req.str("password"),
        "admin",
        &json!({}),
        &Value::Null,
    ) {
        Ok(user) => {
            let token =
                app.accounts
                    .start_session(&app.db, user["id"].as_i64().unwrap_or(0), true, &req.ip, &req.agent);
            with_cookie(json!({"ok": true, "user": me(&user)}), cookie(&req, &token, true))
        }
        Err(e) => error(400, e),
    }
}

async fn login(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let name = req.str("name");
    let delay = app.accounts.delay_for(&name);
    if delay > 0.0 {
        tokio::time::sleep(Duration::from_secs_f64(delay)).await;
    }
    let password = req.str("password");
    let (app2, ip) = (app.clone(), req.ip.clone());
    // scrypt takes a moment of CPU: not on the async threads
    let result = tokio::task::spawn_blocking(move || app2.accounts.authenticate(&app2.db, &name, &password, &ip))
        .await
        .unwrap_or_else(|_| {
            Err(crate::accounts::AuthError {
                message: "intern".into(),
                status: 500,
                reason: "panic",
            })
        });
    match result {
        Ok(user) => {
            let persistent = req.get("remember").as_bool().unwrap_or(false);
            if user["totp"] == true {
                let ticket = new_ticket(&app, user["id"].as_i64().unwrap_or(0), persistent);
                return json_reply(json!({"ok": false, "totp": true, "ticket": ticket}));
            }
            let token = app.accounts.start_session(
                &app.db,
                user["id"].as_i64().unwrap_or(0),
                persistent,
                &req.ip,
                &req.agent,
            );
            with_cookie(json!({"ok": true, "user": me(&user)}), cookie(&req, &token, persistent))
        }
        Err(e) => error(e.status, e.message),
    }
}

fn new_ticket(app: &App, uid: i64, persistent: bool) -> String {
    use base64::Engine;
    use rand::RngCore;
    let mut b = [0u8; 24];
    rand::rng().fill_bytes(&mut b);
    let ticket = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
    let mut totp = app.totp.lock().unwrap_or_else(|p| p.into_inner());
    let now = text::now();
    totp.tickets.retain(|_, v| v.2 >= now);
    totp.tickets.insert(ticket.clone(), (uid, persistent, now + 300.0, 0));
    ticket
}

/// Second step of a login: the code for the ticket from step one (5 tries, 5 minutes).
async fn login_totp(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    if app.accounts.blocked(&req.ip) {
        return error(
            429,
            t("Zu viele Fehlversuche – bitte in 15 Minuten erneut versuchen", &[]),
        );
    }
    let tid = req.str("ticket");
    let ticket = {
        let mut st = app.totp.lock().unwrap_or_else(|p| p.into_inner());
        let valid = st.tickets.get(&tid).is_some_and(|x| x.2 >= text::now() && x.3 < 5);
        if !valid {
            st.tickets.remove(&tid);
            None
        } else {
            let x = st.tickets.get_mut(&tid).expect("checked");
            x.3 += 1;
            Some((x.0, x.1))
        }
    };
    let Some((uid, persistent)) = ticket else {
        return error(401, t("Anmeldung abgelaufen – bitte neu anmelden", &[]));
    };
    let user = app.accounts.get(&app.db, uid);
    let good = user.as_ref().is_some_and(|u| u["disabled"] != true) && crate::totp::check(&app, uid, &req.str("code"));
    if !good {
        app.accounts
            .fail(&req.ip, user.as_ref().and_then(|u| u["name"].as_str()).unwrap_or(""));
        return error(401, t("Code falsch", &[]));
    }
    app.totp.lock().unwrap_or_else(|p| p.into_inner()).tickets.remove(&tid);
    let user = user.unwrap_or(Value::Null);
    let token = app
        .accounts
        .start_session(&app.db, uid, persistent, &req.ip, &req.agent);
    with_cookie(json!({"ok": true, "user": me(&user)}), cookie(&req, &token, persistent))
}

/// A new secret for the authenticator app – active only after a correct code (confirm).
async fn totp_setup(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let secret = crate::totp::secret();
    app.totp
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .pending
        .insert(req.user_id(), secret.clone());
    json_reply(json!({"secret": secret, "uri": crate::totp::uri(&secret, &req.user_name())}))
}

async fn totp_confirm(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let uid = req.user_id();
    let pending = app
        .totp
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .pending
        .get(&uid)
        .cloned();
    match pending {
        Some(secret) if crate::totp::ok(&app, uid, &secret, &req.str("code")) => {
            let _ = app
                .db
                .lock()
                .execute("UPDATE users SET totp=? WHERE id=?", rusqlite::params![secret, uid]);
            app.totp.lock().unwrap_or_else(|p| p.into_inner()).pending.remove(&uid);
            json_reply(json!({"ok": true}))
        }
        _ => error(
            400,
            t(
                "Code falsch – Uhrzeit des Handys prüfen und den aktuellen Code eingeben",
                &[],
            ),
        ),
    }
}

async fn totp_disable(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let (a, name, password, ip) = (app.clone(), req.user_name(), req.str("password"), req.ip.clone());
    let checked = tokio::task::spawn_blocking(move || a.accounts.authenticate(&a.db, &name, &password, &ip)).await;
    match checked {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            return error(
                if e.status == 429 { 429 } else { 403 },
                t("Das bisherige Passwort stimmt nicht", &[]),
            );
        }
        Err(_) => return error(500, "intern".to_string()),
    }
    if !crate::totp::check(&app, req.user_id(), &req.str("code")) {
        return error(403, t("Code falsch", &[]));
    }
    let _ = app
        .db
        .lock()
        .execute("UPDATE users SET totp=NULL WHERE id=?", [req.user_id()]);
    json_reply(json!({"ok": true}))
}

async fn logout(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    app.accounts.end_session(&app.db, &req.token);
    let gone = format!(
        "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{}",
        super::COOKIE,
        if req.https { "; Secure" } else { "" }
    );
    with_cookie(json!({"ok": true}), gone)
}

async fn auth_me(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let user = req.user.clone().unwrap_or(Value::Null);
    let mut out = me(&user);
    out["sessions"] = json!(app.accounts.sessions(&app.db, req.user_id()));
    out["session"] = user["session"].clone();
    out["connection"] = json!({"ip": req.ip, "peer": req.conn["peer"], "proxied": req.conn["proxied"],
                               "trusted": req.conn["trusted"], "https": req.https});
    json_reply(out)
}

async fn password(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    if let Err(e) = app
        .accounts
        .authenticate(&app.db, &req.user_name(), &req.str("old"), &req.ip)
    {
        return error(
            if e.status == 429 { 429 } else { 403 },
            t("Das bisherige Passwort stimmt nicht", &[]),
        );
    }
    if let Err(e) = app.accounts.set_password(&app.db, req.user_id(), &req.str("new")) {
        return error(400, e);
    }
    let token = app
        .accounts
        .start_session(&app.db, req.user_id(), true, &req.ip, &req.agent);
    with_cookie(json!({"ok": true}), cookie(&req, &token, true))
}

async fn sessions_end(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let sid = req.str("id");
    let n = if sid.is_empty() {
        let keep = req
            .user
            .as_ref()
            .and_then(|u| u["session"].as_str())
            .unwrap_or("")
            .to_string();
        app.accounts.end_sessions(&app.db, req.user_id(), &keep)
    } else {
        usize::from(app.accounts.end_session_id(&app.db, req.user_id(), &sid))
    };
    json_reply(json!({"ok": true, "ended": n}))
}

fn prefs(app: &App, uid: i64) -> Map<String, Value> {
    app.db
        .lock()
        .query_row("SELECT prefs FROM users WHERE id=?", [uid], |r| r.get::<_, String>(0))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

async fn prefs_get(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let p = prefs(&app, req.user_id());
    let server = app
        .db
        .get("ntfy")
        .is_some_and(|n| n["url"].as_str().is_some_and(|u| !u.is_empty()));
    json_reply(json!({"ntfy_topic": p.get("ntfy_topic").cloned().unwrap_or(json!("")), "ntfy_server": server}))
}

async fn prefs_put(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let topic = req.str("ntfy_topic").trim().to_string();
    if !topic.is_empty()
        && (topic.len() > 64 || !topic.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    {
        return error(400, t("Topic: nur Buchstaben, Ziffern, - und _ (max. 64 Zeichen)", &[]));
    }
    let mut p = prefs(&app, req.user_id());
    p.insert("ntfy_topic".into(), json!(topic));
    let _ = app.db.lock().execute(
        "UPDATE users SET prefs=? WHERE id=?",
        rusqlite::params![Value::Object(p).to_string(), req.user_id()],
    );
    json_reply(json!({"ok": true, "ntfy_topic": topic}))
}

async fn users_list(app: Arc<App>, _: Req, _: HashMap<String, String>) -> Reply {
    let quota = app.settings().get("request_quota").cloned().unwrap_or(json!(5));
    json_reply(json!({"users": app.accounts.list(&app.db), "default_quota": quota}))
}

async fn users_create(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let role = {
        let r = req.str("role");
        if r.is_empty() { "user".to_string() } else { r }
    };
    match app.accounts.create(
        &app.db,
        &req.str("name"),
        &req.str("password"),
        &role,
        req.get("perms"),
        req.get("quota"),
    ) {
        Ok(user) => json_reply(user),
        Err(e) => error(400, e),
    }
}

async fn users_update(app: Arc<App>, req: Req, params: HashMap<String, String>) -> Reply {
    let Some(uid) = id_param(&params, "uid") else {
        return error(404, t("Nicht gefunden", &[]));
    };
    let mut changes = Map::new();
    for k in ["name", "role", "perms", "quota", "disabled"] {
        if let Some(v) = req.body.get(k) {
            changes.insert(k.into(), v.clone());
        }
    }
    let user = match app.accounts.update(&app.db, uid, &changes) {
        Ok(Some(u)) => u,
        Ok(None) => return error(404, t("Nicht gefunden", &[])),
        Err(e) => return error(400, e),
    };
    if req.get("totp") == &Value::Bool(false) {
        // lost phone: the admin switches the factor off (and every session of the account ends)
        let _ = app.db.lock().execute("UPDATE users SET totp=NULL WHERE id=?", [uid]);
        app.accounts.end_sessions(&app.db, uid, "");
    }
    let password = req.str("password");
    if !password.is_empty() && req.get("password") != &Value::Bool(false) {
        let a = app.clone();
        let set = tokio::task::spawn_blocking(move || a.accounts.set_password(&a.db, uid, &password)).await;
        match set {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return error(400, e),
            Err(_) => return error(500, "intern".to_string()),
        }
    }
    json_reply(user)
}

async fn users_delete(app: Arc<App>, _: Req, params: HashMap<String, String>) -> Reply {
    let Some(uid) = id_param(&params, "uid") else {
        return error(404, t("Nicht gefunden", &[]));
    };
    match app.accounts.delete(&app.db, uid) {
        Ok(true) => json_reply(json!({"ok": true})),
        Ok(false) => error(404, t("Nicht gefunden", &[])),
        Err(e) => error(400, e),
    }
}

async fn audit(app: Arc<App>, _: Req, _: HashMap<String, String>) -> Reply {
    let c = app.db.lock();
    let mut st = c
        .prepare("SELECT time, user, ip, action, detail FROM audit ORDER BY id DESC LIMIT 300")
        .expect("valid SQL");
    let entries: Vec<Value> = st
        .query_map([], |r| {
            Ok(
                json!({"time": r.get::<_, f64>(0)?, "user": r.get::<_, String>(1)?, "ip": r.get::<_, String>(2)?,
                      "action": r.get::<_, String>(3)?, "detail": r.get::<_, String>(4)?}),
            )
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default();
    json_reply(json!({"entries": entries}))
}

// -- invitations ------------------------------------------------------------------------
fn invite_base(app: &App, req: &Req) -> String {
    let url = app
        .settings()
        .get("public_url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if url.is_empty() && req.https && req.conn["trusted"] == true {
        let taken = crate::settings::public_url(&format!("https://{}", req.conn["host"].as_str().unwrap_or("")));
        if !taken.is_empty() {
            let mut change = Map::new();
            change.insert("public_url".into(), json!(taken));
            let _ = app.settings.update(&app.db, change);
            return taken;
        }
    }
    url
}

fn with_link(mut invite: Value, base: &str) -> Value {
    invite["link"] = json!(if base.is_empty() {
        String::new()
    } else {
        format!("{base}/invite/{}", invite["token"].as_str().unwrap_or(""))
    });
    invite
}

async fn invites_list(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let base = invite_base(&app, &req);
    let list: Vec<Value> = crate::invites::list(&app.db)
        .into_iter()
        .map(|i| with_link(i, &base))
        .collect();
    json_reply(json!({"invites": list, "public_url": base}))
}

async fn invites_create(app: Arc<App>, req: Req, _: HashMap<String, String>) -> Reply {
    let base = invite_base(&app, &req);
    if base.is_empty() {
        return error(
            400,
            t(
                "Erst die öffentliche Adresse eintragen (Einstellungen → Allgemein), z. B. https://hoerwerk.example.de – Einladungslinks zeigen immer dorthin.",
                &[],
            ),
        );
    }
    match crate::invites::create(&app.db, &req.user_name(), &req.body) {
        Ok(i) => json_reply(with_link(i, &base)),
        Err(e) => error(400, e),
    }
}

async fn invites_revoke(app: Arc<App>, _: Req, params: HashMap<String, String>) -> Reply {
    match id_param(&params, "iid").and_then(|id| crate::invites::revoke(&app.db, id)) {
        Some(i) => json_reply(i),
        None => error(404, t("Nicht gefunden", &[])),
    }
}

async fn invites_delete(app: Arc<App>, _: Req, params: HashMap<String, String>) -> Reply {
    if id_param(&params, "iid").is_some_and(|id| crate::invites::delete(&app.db, id)) {
        json_reply(json!({"ok": true}))
    } else {
        error(404, t("Nicht gefunden", &[]))
    }
}

async fn refuse_invite(app: &App, ip: &str, message: String, name: &str) -> Reply {
    app.accounts.fail(ip, name);
    tokio::time::sleep(Duration::from_secs(1)).await;
    error(404, message)
}

async fn invite_check(app: Arc<App>, req: Req, params: HashMap<String, String>) -> Reply {
    match crate::invites::check(&app.db, params.get("token").map(String::as_str).unwrap_or("")) {
        Ok(i) => json_reply(json!({"ok": true, "note": i["note"], "expires": i["expires"]})),
        Err(e) => refuse_invite(&app, &req.ip, e, "").await,
    }
}

async fn invite_accept(app: Arc<App>, req: Req, params: HashMap<String, String>) -> Reply {
    let name = req.str("name");
    let token = params.get("token").cloned().unwrap_or_default();
    match crate::invites::accept(&app.db, &app.accounts, &token, &name, &req.str("password")) {
        Ok(user) => {
            app.audit(
                user["name"].as_str().unwrap_or(""),
                &req.ip,
                "invite_accepted",
                user["name"].as_str().unwrap_or(""),
            );
            let session =
                app.accounts
                    .start_session(&app.db, user["id"].as_i64().unwrap_or(0), true, &req.ip, &req.agent);
            with_cookie(json!({"ok": true, "user": me(&user)}), cookie(&req, &session, true))
        }
        Err((true, e)) => refuse_invite(&app, &req.ip, e, &name).await,
        Err((false, e)) => error(400, e),
    }
}
