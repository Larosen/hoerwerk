//! HTTP: every request goes through ``dispatch``, always with the same checks in the same order:
//! host name, pages and static files, CSRF, route, login, rights, body size.

pub mod api;
pub mod audiobooks;
pub mod discover;
pub mod follows;
pub mod hosts;
pub mod library;
pub mod listening;
pub mod requests;
pub mod search;
pub mod system;
pub mod upload;

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, Response, StatusCode, header};
use futures_util::{FutureExt, Stream};
use http_body_util::{BodyExt, Limited};
use serde_json::{Value, json};

use crate::app::App;
use crate::text::t;

pub const MAX_BODY: usize = 4 * 1024 * 1024;
pub const MAX_UPLOAD: usize = 4 << 30;
pub const COOKIE: &str = "hw_session";
const CSRF_HEADER: &str = "x-requested-with";
const CSRF_VALUE: &str = "Hoerwerk";
/// Connections served at the same time; beyond that a short 503 (the server stays responsive).
pub const MAX_CONCURRENT: usize = 96;
const SECURITY_HEADERS: [(&str, &str); 6] = [
    ("x-content-type-options", "nosniff"),
    ("x-frame-options", "DENY"),
    ("referrer-policy", "no-referrer"),
    ("cross-origin-opener-policy", "same-origin"),
    ("cross-origin-resource-policy", "same-origin"),
    (
        "content-security-policy",
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
    ),
];
const PUBLIC_STATIC: [&str; 10] = [
    "style.css",
    "icon.svg",
    "icon.png",
    "theme.js",
    "core.js",
    "login.js",
    "manifest.webmanifest",
    "apple-touch-icon.png",
    "icon-192.png",
    "icon-512.png",
];
const APP_PAGES: [(&str, &str); 8] = [
    ("books", "admin"),
    ("search", "user"),
    ("discover", "user"),
    ("requests", "user"),
    ("account", "user"),
    ("settings", "admin"),
    ("stats", "user"),
    ("more", "user"),
];
const SETTINGS_SECTIONS: [&str; 12] = [
    "health",
    "general",
    "library",
    "convert",
    "downloads",
    "services",
    "discover",
    "notify",
    "users",
    "stats",
    "logs",
    "transfers",
];

/// A finished answer of a handler.
pub enum Reply {
    Json(StatusCode, Value, Vec<(String, String)>),
    Raw(StatusCode, &'static str, Bytes, Vec<(String, String)>),
    Stream(Box<dyn Stream<Item = Result<Bytes, Infallible>> + Send + Unpin>),
}

pub fn json_reply(v: Value) -> Reply {
    Reply::Json(StatusCode::OK, v, vec![])
}

pub fn error(status: u16, message: impl Into<String>) -> Reply {
    Reply::Json(
        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
        json!({"error": message.into()}),
        vec![],
    )
}

/// What a handler sees of a request.
pub struct Req {
    pub method: Method,
    pub path: String,
    pub query: HashMap<String, String>,
    pub body: Value,
    pub raw: Option<Body>,
    pub length: usize,
    pub user: Option<Value>,
    pub ip: String,
    pub https: bool,
    pub token: String,
    pub agent: String,
    pub proxied: bool,
    pub conn: Value,
}

impl Req {
    pub fn get(&self, key: &str) -> &Value {
        self.body.get(key).unwrap_or(&Value::Null)
    }

    pub fn str(&self, key: &str) -> String {
        match self.get(key) {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        }
    }

    pub fn user_id(&self) -> i64 {
        self.user.as_ref().and_then(|u| u["id"].as_i64()).unwrap_or(0)
    }

    pub fn user_name(&self) -> String {
        self.user
            .as_ref()
            .and_then(|u| u["name"].as_str())
            .unwrap_or("")
            .to_string()
    }
}

fn header_str<'a>(h: &'a HeaderMap, name: &str) -> &'a str {
    h.get(name).and_then(|v| v.to_str().ok()).unwrap_or("")
}

fn parse_ip(text: &str) -> Option<IpAddr> {
    let ip: IpAddr = text.trim().trim_matches(['[', ']']).split('%').next()?.parse().ok()?;
    Some(match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        v4 => v4,
    })
}

pub struct Conn {
    pub peer: IpAddr,
    pub trusted: bool,
    pub forwarded: bool,
    pub ip: String,
}

/// The real client: behind a trusted proxy the right-most X-Forwarded-For entry it did not add itself.
fn client(app: &App, peer: SocketAddr, h: &HeaderMap) -> Conn {
    let peer_ip = parse_ip(&peer.ip().to_string()).unwrap_or(peer.ip());
    let trusted_net = |ip: &IpAddr| app.cfg.trusted_proxies.iter().any(|n| n.contains(ip));
    let trusted = trusted_net(&peer_ip);
    let forwarded = ["x-forwarded-for", "forwarded", "x-real-ip"]
        .iter()
        .any(|k| !header_str(h, k).is_empty());
    if !trusted {
        // a proxy in the own network that is not trusted: the visitors' real addresses get lost – say so
        let private = match peer_ip {
            IpAddr::V4(v4) => {
                v4.is_private() || v4.is_link_local() || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1]))
            }
            IpAddr::V6(v6) => (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80,
        };
        if forwarded && private && !peer_ip.is_loopback() {
            app.note_untrusted(&peer_ip.to_string());
        }
        return Conn {
            peer: peer_ip,
            trusted,
            forwarded,
            ip: peer_ip.to_string(),
        };
    }
    let chain: Vec<Option<IpAddr>> = header_str(h, "x-forwarded-for")
        .split(',')
        .filter(|p| !p.trim().is_empty())
        .map(parse_ip)
        .collect();
    for ip in chain.iter().rev() {
        match ip {
            None => break,
            Some(ip) if !trusted_net(ip) => {
                return Conn {
                    peer: peer_ip,
                    trusted,
                    forwarded,
                    ip: ip.to_string(),
                };
            }
            _ => {}
        }
    }
    let real = parse_ip(header_str(h, "x-real-ip"));
    let ip = real.or(chain.first().copied().flatten()).unwrap_or(peer_ip);
    Conn {
        peer: peer_ip,
        trusted,
        forwarded,
        ip: ip.to_string(),
    }
}

fn is_https(app: &App, conn: &Conn, h: &HeaderMap) -> bool {
    match app.cfg.cookie_secure.as_str() {
        "true" => true,
        "false" => false,
        _ => {
            conn.trusted
                && header_str(h, "x-forwarded-proto")
                    .split(',')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .eq_ignore_ascii_case("https")
        }
    }
}

fn cookie_token(h: &HeaderMap) -> String {
    for part in header_str(h, "cookie").split(';') {
        if let Some((k, v)) = part.trim().split_once('=')
            && k == COOKIE
        {
            return v.trim().to_string();
        }
    }
    String::new()
}

/// Changing requests must come from our own page: custom header (needs a CORS preflight a foreign page can't
/// get), Sec-Fetch-Site and Origin where the browser sends them.
fn same_origin(h: &HeaderMap) -> bool {
    if header_str(h, CSRF_HEADER) != CSRF_VALUE {
        return false;
    }
    let site = header_str(h, "sec-fetch-site");
    if !site.is_empty() && site != "same-origin" && site != "none" {
        return false;
    }
    let origin = header_str(h, "origin");
    if !origin.is_empty() && origin != "null" {
        let netloc = origin
            .split_once("://")
            .map(|x| x.1)
            .unwrap_or("")
            .split('/')
            .next()
            .unwrap_or("");
        return netloc.eq_ignore_ascii_case(header_str(h, "host"));
    }
    true
}

pub fn page_access(path: &str) -> Option<&'static str> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let (_, access) = APP_PAGES.iter().find(|(name, _)| parts.first() == Some(name))?;
    match (parts[0], parts.len()) {
        ("settings", 2) if !SETTINGS_SECTIONS.contains(&parts[1]) => None,
        ("settings", n) if n > 2 => None,
        ("settings", _) => Some(access),
        ("stats", 2) => (parts[1].len() < 10 && parts[1].chars().all(|c| c.is_ascii_digit())).then_some("user"),
        (_, n) if n > 1 => None,
        _ => Some(access),
    }
}

fn respond(status: StatusCode, ctype: &str, body: Bytes, extra: &[(String, String)], close: bool) -> Response<Body> {
    let mut r = Response::builder().status(status);
    let h = r.headers_mut().expect("fresh builder");
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(ctype).unwrap_or(HeaderValue::from_static("text/plain")),
    );
    if !extra.iter().any(|(k, _)| k.eq_ignore_ascii_case("cache-control")) {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    for (k, v) in SECURITY_HEADERS {
        h.insert(k, HeaderValue::from_static(v));
    }
    for (k, v) in extra {
        if let (Ok(name), Ok(value)) = (
            axum::http::HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(v),
        ) {
            h.append(name, value);
        }
    }
    if close {
        h.insert(header::CONNECTION, HeaderValue::from_static("close"));
    }
    r.body(Body::from(body)).expect("valid response")
}

fn json_response(status: StatusCode, v: &Value, extra: &[(String, String)], close: bool) -> Response<Body> {
    respond(
        status,
        "application/json; charset=utf-8",
        Bytes::from(v.to_string()),
        extra,
        close,
    )
}

fn simple(status: u16, message: &str) -> Response<Body> {
    json_response(
        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
        &json!({"error": message}),
        &[],
        false,
    )
}

fn static_file(app: &App, rel: &str) -> Response<Body> {
    let decoded = percent_encoding::percent_decode_str(rel)
        .decode_utf8_lossy()
        .into_owned();
    let root = app
        .cfg
        .static_dir
        .canonicalize()
        .unwrap_or_else(|_| app.cfg.static_dir.clone());
    let full = match root.join(&decoded).canonicalize() {
        Ok(p) if p.starts_with(&root) && p.is_file() => p,
        _ => return simple(404, &t("Nicht gefunden", &[])),
    };
    let mut ctype = mime_guess::from_path(&full)
        .first_or_octet_stream()
        .essence_str()
        .to_string();
    if ctype.starts_with("text/") || ctype.ends_with("javascript") {
        ctype.push_str("; charset=utf-8");
    }
    match std::fs::read(&full) {
        Ok(data) => respond(StatusCode::OK, &ctype, Bytes::from(data), &[], false),
        Err(_) => simple(404, &t("Nicht gefunden", &[])),
    }
}

fn redirect(to: &str) -> Response<Body> {
    respond(
        StatusCode::SEE_OTHER,
        "text/plain",
        Bytes::new(),
        &[("location".into(), to.into())],
        false,
    )
}

/// Counts requests in flight (overall) – beyond ``MAX_CONCURRENT`` a short 503.
pub struct Slots(AtomicUsize);

impl Slots {
    pub fn new() -> Self {
        Slots(AtomicUsize::new(0))
    }
}

impl Default for Slots {
    fn default() -> Self {
        Self::new()
    }
}

struct SlotGuard<'a>(&'a AtomicUsize);

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub async fn dispatch(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
) -> Response<Body> {
    if app.slots.0.fetch_add(1, Ordering::SeqCst) >= MAX_CONCURRENT {
        app.slots.0.fetch_sub(1, Ordering::SeqCst);
        let mut r = respond(
            StatusCode::SERVICE_UNAVAILABLE,
            "text/plain; charset=utf-8",
            Bytes::from("Zu viele Verbindungen – gleich nochmal"),
            &[("retry-after".into(), "2".into())],
            true,
        );
        r.headers_mut().remove(header::CACHE_CONTROL);
        return r;
    }
    let _slot = SlotGuard(&app.slots.0);
    let (method, path) = (req.method().clone(), req.uri().path().to_string());
    match std::panic::AssertUnwindSafe(handle(app.clone(), peer, req))
        .catch_unwind()
        .await
    {
        Ok(r) => r,
        Err(p) => internal_error(&method, &path, p.as_ref()),
    }
}

/// Unexpected errors: everything into the log, only a number to the browser – no error text, paths, SQL or keys
/// leave the server. The number finds the entry in the log.
fn internal_error(method: &Method, path: &str, panic: &(dyn std::any::Any + Send)) -> Response<Body> {
    let rid = hex::encode(rand::random::<[u8; 4]>());
    let why = panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".into());
    crate::text::error("web", &format!("Fehler {rid} in {method} {path}: {why}"));
    let body = json!({"error": t("Interner Fehler (Nr. {id}) – Einzelheiten stehen im Protokoll", &[("id", &rid)]),
                      "request_id": rid});
    respond(
        StatusCode::INTERNAL_SERVER_ERROR,
        "application/json",
        Bytes::from(body.to_string()),
        &[],
        true,
    )
}

async fn handle(app: Arc<App>, peer: SocketAddr, req: Request) -> Response<Body> {
    let (parts, body) = req.into_parts();
    let h = &parts.headers;
    let method = parts.method.clone();
    let path = parts.uri.path().to_string();
    let reading = method == Method::GET || method == Method::HEAD;
    let conn = client(&app, peer, h);
    let host = header_str(h, "host").to_string();
    // the health check answers under any name: Docker asks as 127.0.0.1, Pangolin/Newt under the target name
    if !hosts::allowed(&app, &host, conn.trusted) && !(path == "/api/health" && reading) {
        let name: String = hosts::host_name(&host).chars().take(100).collect();
        app.note_refused_host(&name);
        let text = t(
            "Kein Zugriff unter „{host}“ (DNS-Rebinding-Schutz). Adresse unter Einstellungen → Allgemein eintragen (Hörwerk dafür einmal per IP öffnen) oder ALLOWED_HOSTS={host} setzen.",
            &[("host", &name)],
        );
        return respond(
            StatusCode::MISDIRECTED_REQUEST,
            "text/plain; charset=utf-8",
            Bytes::from(text),
            &[],
            true,
        );
    }
    let token = cookie_token(h);
    let session = |app: &App| app.accounts.session_user(&app.db, &token, &conn.ip);
    if reading {
        match path.as_str() {
            "/favicon.ico" => return static_file(&app, "icon.png"),
            // the installed app (iPhone home screen): the browser asks for these before anyone is signed in
            "/manifest.webmanifest" => return static_file(&app, "manifest.webmanifest"),
            "/apple-touch-icon.png" => return static_file(&app, "apple-touch-icon.png"),
            "/i18n.js" => {
                let data = json!({"lang": crate::text::language(), "text": crate::text::catalog_json()});
                return respond(
                    StatusCode::OK,
                    "text/javascript; charset=utf-8",
                    Bytes::from(format!("const I18N = {data};\n")),
                    &[],
                    false,
                );
            }
            "/me.js" => {
                // "help": whether the "So funktioniert's" boxes are shown (Einstellungen → Allgemein)
                let help = app.settings().get("show_help").and_then(Value::as_bool).unwrap_or(true);
                let me = session(&app)
                    .map(|u| {
                        let mut me = crate::accounts::me(&u);
                        me["help"] = json!(help);
                        // the colour scheme of the account – known before the page is painted
                        let uid = u.get("id").and_then(Value::as_i64).unwrap_or(0);
                        me["theme"] = json!(api::theme_of(&api::prefs(&app, uid)));
                        me
                    })
                    .unwrap_or(Value::Null);
                return respond(
                    StatusCode::OK,
                    "text/javascript; charset=utf-8",
                    Bytes::from(format!("const ME = {me};\n")),
                    &[],
                    false,
                );
            }
            _ => {}
        }
        if let Some(rel) = path.strip_prefix("/static/") {
            if !PUBLIC_STATIC.contains(&rel) && session(&app).is_none() {
                return json_response(
                    StatusCode::UNAUTHORIZED,
                    &json!({"error": t("Bitte anmelden", &[]), "login": true}),
                    &[],
                    false,
                );
            }
            return static_file(&app, rel);
        }
        if let Some(tok) = path.strip_prefix("/invite/") {
            if !crate::invites::valid_token_format(tok) {
                return simple(404, &t("Nicht gefunden", &[]));
            }
            return static_file(&app, "login.html");
        }
        if path == "/" || path == "/login" {
            let signed_in = session(&app).is_some();
            if path == "/" && !signed_in {
                return redirect("/login");
            }
            if path == "/login" && signed_in {
                return redirect("/");
            }
            return static_file(&app, if path == "/" { "index.html" } else { "login.html" });
        }
        if let Some(access) = page_access(&path) {
            let Some(user) = session(&app) else {
                let next: String = percent_encoding::utf8_percent_encode(&path, percent_encoding::NON_ALPHANUMERIC)
                    .to_string()
                    .replace("%2F", "/");
                return redirect(&format!("/login?next={next}"));
            };
            if access == "admin" && user["role"] != "admin" {
                return redirect("/");
            }
            return static_file(&app, "index.html");
        }
    }
    if !reading && !same_origin(h) {
        return simple(403, &t("Anfrage von fremder Seite abgelehnt", &[]));
    }
    let length: usize = match header_str(h, "content-length").trim() {
        "" => 0,
        v => match v.parse() {
            Ok(n) => n,
            Err(_) => return simple(400, &t("Ungültige Länge", &[])),
        },
    };
    let Some((route, params)) = api::find(&method, &path) else {
        return simple(404, &t("Nicht gefunden", &[]));
    };
    // who is asking is checked before a single byte of the body is read (uploads can be large)
    let user = if route.access != "public" || !token.is_empty() {
        session(&app)
    } else {
        None
    };
    if route.access != "public" && user.is_none() {
        return json_response(
            StatusCode::UNAUTHORIZED,
            &json!({"error": t("Bitte anmelden", &[]), "login": true}),
            &[],
            false,
        );
    }
    if route.access == "admin" {
        let u = user.as_ref().expect("checked above");
        if u["role"] != "admin" {
            crate::authlog::event(
                "forbidden",
                &[
                    ("user", u["name"].as_str().unwrap_or("")),
                    ("ip", &conn.ip),
                    ("path", &path),
                ],
            );
            return simple(403, &t("Dafür fehlen die Rechte", &[]));
        }
        if u["totp"] != true && app.setting_bool("admin_totp_required") {
            return json_response(
                StatusCode::FORBIDDEN,
                &json!({"error": t("Admins brauchen einen Zweitfaktor – bitte unter Konto einrichten", &[]), "totp_setup": true}),
                &[],
                false,
            );
        }
    }
    let limit = if route.raw_body { MAX_UPLOAD } else { MAX_BODY };
    if length > limit {
        return json_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            &json!({"error": t("Anfrage zu groß", &[])}),
            &[],
            true,
        );
    }
    if route.raw_body && header_str(h, "content-length").is_empty() {
        return simple(411, &t("Ungültige Länge", &[]));
    }
    let mut raw = None;
    let mut json_body = Value::Null;
    if route.raw_body {
        raw = Some(body);
    } else if length > 0 {
        let bytes = match Limited::new(body, MAX_BODY).collect().await {
            Ok(c) => c.to_bytes(),
            Err(_) => return simple(400, &t("Ungültiges JSON", &[])),
        };
        json_body = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(_) => return simple(400, &t("Ungültiges JSON", &[])),
        };
    }
    let query = parts
        .uri
        .query()
        .map(|q| {
            q.split('&')
                .map(|kv| {
                    let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                    let dec = |s: &str| {
                        percent_encoding::percent_decode_str(&s.replace('+', " "))
                            .decode_utf8_lossy()
                            .into_owned()
                    };
                    (dec(k), dec(v))
                })
                .collect()
        })
        .unwrap_or_default();
    let https = is_https(&app, &conn, h);
    let conn_info = json!({"peer": conn.peer.to_string(), "trusted": conn.trusted, "proxied": conn.forwarded,
                           "host": host.chars().take(200).collect::<String>(),
                           "forwarded_for": header_str(h, "x-forwarded-for").chars().take(300).collect::<String>(),
                           "forwarded_proto": header_str(h, "x-forwarded-proto").chars().take(20).collect::<String>()});
    let request = Req {
        method: method.clone(),
        path: path.clone(),
        query,
        body: json_body,
        raw,
        length,
        user,
        ip: conn.ip.clone(),
        https,
        token,
        agent: header_str(h, "user-agent").to_string(),
        proxied: conn.forwarded,
        conn: conn_info,
    };
    let audit = route.access == "admin" && !reading;
    let user_name = request.user_name();
    let ip = request.ip.clone();
    let reply = (route.handler)(app.clone(), request, params).await;
    if audit {
        app.audit(&user_name, &ip, &format!("{method} {path}"), "");
    }
    match reply {
        Reply::Json(status, v, extra) => json_response(status, &v, &extra, route.raw_body),
        Reply::Raw(status, ctype, body, extra) => respond(status, ctype, body, &extra, false),
        Reply::Stream(stream) => {
            let mut r = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
                .header(header::CACHE_CONTROL, "no-store")
                .header("x-accel-buffering", "no");
            for (k, v) in SECURITY_HEADERS {
                r = r.header(k, v);
            }
            r.body(Body::from_stream(stream)).expect("valid response")
        }
    }
}

#[cfg(test)]
mod internal_error_tests {
    use super::*;

    #[test]
    fn only_a_number_leaves_the_server() {
        crate::text::set_language("de");
        let secret: Box<dyn std::any::Any + Send> = Box::new(String::from("SELECT pw FROM users /config/x"));
        let r = internal_error(&Method::GET, "/api/x", secret.as_ref());
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = futures_util::FutureExt::now_or_never(r.into_body().collect())
            .unwrap()
            .unwrap()
            .to_bytes();
        let v: Value = serde_json::from_slice(&body).unwrap();
        let rid = v["request_id"].as_str().unwrap();
        assert_eq!(rid.len(), 8);
        assert_eq!(
            v["error"],
            json!(format!(
                "Interner Fehler (Nr. {rid}) – Einzelheiten stehen im Protokoll"
            ))
        );
        assert!(!String::from_utf8_lossy(&body).contains("SELECT"));
    }
}
