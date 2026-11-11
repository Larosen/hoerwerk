//! What runs here and the health check page: connection & proxy, security, storage, services,
//! system – each check: group, ok/warn/error/info, what to do.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::{Value, json};

use super::{Reply, Req, json_reply};
use crate::app::App;
use crate::text::{self, now, t};
use crate::{media, settings};

type Params = HashMap<String, String>;

fn check(group: &str, status: &str, title: String, detail: String, hint: String, id: &str) -> Value {
    json!({"id": if id.is_empty() { title.clone() } else { id.to_string() }, "group": group, "status": status, "title": title, "detail": detail, "hint": hint})
}

fn when(ts: f64) -> String {
    let tm = text::local(ts);
    format!(
        "{:02}.{:02}. {:02}:{:02}",
        tm.tm_mday,
        tm.tm_mon + 1,
        tm.tm_hour,
        tm.tm_min
    )
}

fn free_gb(path: &Path) -> f64 {
    let Ok(c) = std::ffi::CString::new(path.display().to_string()) else {
        return -1.0;
    };
    // SAFETY: statvfs only writes into the struct we own
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut st) != 0 {
            return -1.0;
        }
        (st.f_bavail as f64 * st.f_frsize as f64) / 1e9
    }
}

fn writable(path: &Path) -> bool {
    let Ok(c) = std::ffi::CString::new(path.display().to_string()) else {
        return false;
    };
    // SAFETY: access only reads the path
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

/// Folder there, writable, enough space – one line per folder.
fn dir_check(name: &str, path: &Path) -> Value {
    let id = format!("dir-{name}");
    let p = path.display().to_string();
    if !path.is_dir() {
        return check(
            "storage",
            "error",
            t("{name}: {path} fehlt", &[("name", name), ("path", &p)]),
            t("Ordner im Docker-Template einbinden.", &[]),
            String::new(),
            &id,
        );
    }
    let gb = free_gb(path);
    let free = if gb >= 0.0 {
        let n = format!("{gb:.1}");
        t(
            "{gb} GB frei",
            &[(
                "gb",
                &if text::language() == "de" {
                    n.replace('.', ",")
                } else {
                    n
                },
            )],
        )
    } else {
        String::new()
    };
    if !writable(path) {
        // SAFETY: getuid has no preconditions
        let uid = unsafe { libc::getuid() }.to_string();
        return check(
            "storage",
            "error",
            t("{name}: {path} nicht beschreibbar", &[("name", name), ("path", &p)]),
            t("Rechte prüfen (PUID/PGID, läuft als UID {uid}).", &[("uid", &uid)]),
            String::new(),
            &id,
        );
    }
    if (0.0..5.0).contains(&gb) {
        return check(
            "storage",
            "warn",
            t("{name}: nur noch {free}", &[("name", name), ("free", &free)]),
            p,
            t(
                "Beim Umwandeln braucht ein Hörbuch kurzzeitig etwa seine doppelte Größe.",
                &[],
            ),
            &id,
        );
    }
    check("storage", "ok", format!("{name}: {free}"), p, String::new(), &id)
}

pub async fn system(app: Arc<App>, _: Req, _: Params) -> Reply {
    let (ff, sb) = tokio::task::spawn_blocking(|| (media::ffmpeg_version(), media::sandbox_status()))
        .await
        .unwrap_or_default();
    json_reply(json!({"version": settings::version(), "ffmpeg": ff, "sandbox": sb, "queues": app.engine.stats()}))
}

/// For Docker, Pangolin & co.: alive. The version only for signed-in users (no help for attackers).
pub async fn health(_: Arc<App>, req: Req, _: Params) -> Reply {
    if req.user.is_some() {
        json_reply(json!({"ok": true, "version": settings::version()}))
    } else {
        json_reply(json!({"ok": true}))
    }
}

pub async fn diagnostics(app: Arc<App>, req: Req, _: Params) -> Reply {
    let mut checks: Vec<Value> = Vec::new();
    let mut add = |group: &str, status: &str, title: String, detail: String, hint: String, id: &str| {
        checks.push(check(group, status, title, detail, hint, id))
    };
    let conn = &req.conn;
    let peer = conn["peer"].as_str().unwrap_or("").to_string();
    let none = String::new;

    // -- connection & proxy (this very request) --
    add(
        "connection",
        "info",
        t("Hörwerk sieht dich als {ip}", &[("ip", &req.ip)]),
        if peer != req.ip {
            t("Über {peer}", &[("peer", &peer)])
        } else {
            none()
        },
        none(),
        "ip",
    );
    if conn["proxied"] == true {
        if conn["trusted"] == true {
            let xff = conn["forwarded_for"].as_str().filter(|x| !x.is_empty()).unwrap_or("–");
            add(
                "connection",
                "ok",
                t("Reverse-Proxy {peer} ist vertrauenswürdig", &[("peer", &peer)]),
                t("X-Forwarded-For: {xff}", &[("xff", xff)]),
                none(),
                "proxy",
            );
        } else {
            add(
                "connection",
                "error",
                t(
                    "Reverse-Proxy {peer} steht nicht in TRUSTED_PROXIES",
                    &[("peer", &peer)],
                ),
                t(
                    "Alle Besucher erscheinen mit der Adresse des Proxys (Anmelde-Sperre, CrowdSec).",
                    &[],
                ),
                format!("TRUSTED_PROXIES={peer}"),
                "proxy",
            );
        }
    } else if !app.cfg.trusted_proxies.is_empty() {
        let nets: Vec<String> = app
            .cfg
            .trusted_proxies
            .iter()
            .map(|n| format!("{}/{}", n.addr, n.prefix))
            .collect();
        add(
            "connection",
            "ok",
            t("Direkt verbunden (ohne Proxy)", &[]),
            t("Vertrauenswürdige Proxys: {nets}", &[("nets", &nets.join(", "))]),
            none(),
            "proxy",
        );
    } else {
        add(
            "connection",
            "info",
            t("Direkt verbunden, kein Reverse-Proxy eingerichtet", &[]),
            t(
                "Für den Zugriff aus dem Internet: Reverse-Proxy mit HTTPS davor und TRUSTED_PROXIES setzen.",
                &[],
            ),
            none(),
            "proxy",
        );
    }
    let mut untrusted: Vec<(String, (f64, f64))> = app
        .untrusted_proxies
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    untrusted.sort_by(|a, b| a.0.cmp(&b.0));
    for (p, (seen, _)) in untrusted.into_iter().filter(|(p, _)| *p != peer) {
        add(
            "connection",
            "warn",
            t(
                "{peer} schickte X-Forwarded-For, ist aber nicht vertrauenswürdig",
                &[("peer", &p)],
            ),
            t(
                "Zuletzt {ago}. Wenn das dein Proxy ist (z. B. Newt nach einem Neustart mit neuer IP): eintragen.",
                &[("ago", &when(seen))],
            ),
            format!("TRUSTED_PROXIES={p}"),
            &format!("untrusted-{p}"),
        );
    }
    if conn["proxied"] == true {
        if req.https {
            add(
                "connection",
                "ok",
                t("HTTPS erkannt – das Anmelde-Cookie ist nur über HTTPS gültig", &[]),
                none(),
                none(),
                "https",
            );
        } else {
            add(
                "connection",
                "warn",
                t("Der Proxy meldet kein HTTPS", &[]),
                t(
                    "Ohne „X-Forwarded-Proto: https“ vom vertrauenswürdigen Proxy wird das Anmelde-Cookie ohne „Secure“ gesetzt.",
                    &[],
                ),
                "COOKIE_SECURE=true".into(),
                "https",
            );
        }
    }
    let pub_url = app
        .settings()
        .get("public_url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if !pub_url.is_empty() {
        let host = app.public_host();
        let local = match host.parse::<IpAddr>() {
            Ok(IpAddr::V4(v4)) => v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_unspecified(),
            Ok(IpAddr::V6(v6)) => {
                v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80
            }
            Err(_) => host == "localhost" || host == "tower" || !host.contains('.'),
        };
        let plain = pub_url.starts_with("http:");
        let detail = if local {
            t(
                "Das ist keine Adresse aus dem Internet – Einladungslinks funktionieren so nur zu Hause.",
                &[],
            )
        } else if plain {
            t("Ohne HTTPS gehen Passwörter unverschlüsselt durchs Netz.", &[])
        } else {
            t("Einladungslinks zeigen hierher.", &[])
        };
        add(
            "connection",
            if local || plain { "warn" } else { "ok" },
            t("Öffentliche Adresse: {url}", &[("url", &pub_url)]),
            detail,
            none(),
            "public_url",
        );
    } else {
        add(
            "connection",
            "warn",
            t(
                "Keine öffentliche Adresse eingetragen – Einladungslinks gehen erst danach",
                &[],
            ),
            t("Einstellungen → Allgemein.", &[]),
            "https://hoerwerk.example.de".into(),
            "public_url",
        );
    }
    let host = super::hosts::host_name(conn["host"].as_str().unwrap_or(""));
    if app.cfg.allowed_hosts.contains("*") {
        add(
            "connection",
            "warn",
            t("ALLOWED_HOSTS=* – Hörwerk antwortet unter jedem Namen", &[]),
            t(
                "Kein Schutz gegen DNS-Rebinding. Besser die eigenen Namen eintragen.",
                &[],
            ),
            format!("ALLOWED_HOSTS={host}"),
            "hosts",
        );
    } else if !app.cfg.allowed_hosts.is_empty() {
        let mut names: Vec<&String> = app.cfg.allowed_hosts.iter().collect();
        names.sort();
        let list = names.iter().map(|x| x.as_str()).collect::<Vec<_>>().join(", ");
        add(
            "connection",
            "ok",
            t("Erlaubte Namen: {hosts}", &[("hosts", &list)]),
            t("Aufgerufen als {host}", &[("host", &host)]),
            none(),
            "hosts",
        );
    } else {
        add(
            "connection",
            "ok",
            t(
                "Hörwerk antwortet unter IP-Adressen, Namen aus dem Heimnetz und der öffentlichen Adresse",
                &[],
            ),
            t("Schutz gegen DNS-Rebinding. Weitere Namen: ALLOWED_HOSTS.", &[]),
            none(),
            "hosts",
        );
    }
    let mut refused: Vec<(String, f64)> = app
        .refused_hosts
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .map(|(k, v)| (k.clone(), v.0))
        .collect();
    refused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (name, seen) in refused.into_iter().take(5) {
        add(
            "connection",
            "warn",
            t("Aufruf als „{host}“ abgelehnt", &[("host", &name)]),
            t(
                "Zuletzt {ago}. Wenn das deine Adresse ist: als öffentliche Adresse eintragen oder ALLOWED_HOSTS setzen.",
                &[("ago", &when(seen))],
            ),
            format!("ALLOWED_HOSTS={name}"),
            &format!("refused-{name}"),
        );
    }
    if !app.library_ready.load(Ordering::SeqCst) {
        let err = app.library_error.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let (status, title, detail) = if err.is_empty() {
            (
                "warn",
                t("Bibliothek wird eingelesen – Bücher werden danach verarbeitet", &[]),
                t(
                    "Erst wenn Hörwerk die Bibliothek kennt, prüft es neue Bücher auf Doppelte.",
                    &[],
                ),
            )
        } else {
            (
                "error",
                t("Bibliothek nicht lesbar – es wird nichts verarbeitet", &[]),
                err,
            )
        };
        add("connection", status, title, detail, none(), "library-ready");
    }
    let backup_error = app.db_backup.lock().unwrap_or_else(|p| p.into_inner()).1.clone();
    if !backup_error.is_empty() {
        add(
            "storage",
            "error",
            t("Tägliche Datenbank-Sicherung fehlgeschlagen", &[]),
            t("{error} – neuer Versuch jede Stunde.", &[("error", &backup_error)]),
            none(),
            "db-backup",
        );
    }

    // -- security --
    let users = app.accounts.list(&app.db);
    let no_totp: Vec<&str> = users
        .iter()
        .filter(|u| u["role"] == "admin" && u["totp"] != true && u["disabled"] != true)
        .filter_map(|u| u["name"].as_str())
        .collect();
    if !no_totp.is_empty() {
        add(
            "security",
            "warn",
            t("Admins ohne Zweitfaktor: {names}", &[("names", &no_totp.join(", "))]),
            t(
                "Unter Konto → Zweitfaktor einrichten; danach in Entdecken & Anfragen „Admins brauchen einen Zweitfaktor“ einschalten.",
                &[],
            ),
            none(),
            "totp",
        );
    } else {
        add(
            "security",
            "ok",
            t("Alle Admins haben einen Zweitfaktor", &[]),
            none(),
            none(),
            "totp",
        );
    }
    if !app.setting_bool("admin_totp_required") {
        add(
            "security",
            "info",
            t("Zweitfaktor für Admins ist nicht Pflicht", &[]),
            none(),
            none(),
            "totp_required",
        );
    }
    let logs = app.cfg.config_dir.join("logs");
    let log = logs.join("auth.log").display().to_string();
    if logs.is_dir() && writable(&logs) {
        add(
            "security",
            "ok",
            t("Anmelde-Protokoll für CrowdSec: {path}", &[("path", &log)]),
            none(),
            none(),
            "authlog",
        );
    } else {
        add(
            "security",
            "error",
            t("Anmelde-Protokoll nicht schreibbar: {path}", &[("path", &log)]),
            none(),
            none(),
            "authlog",
        );
    }
    let blocked = app.accounts.blocked_count();
    let title = match blocked {
        0 => t("Keine Adresse wegen Fehlversuchen gesperrt", &[]),
        1 => t("1 Adresse gerade wegen Fehlversuchen gesperrt", &[]),
        n => t(
            "{n} Adressen gerade wegen Fehlversuchen gesperrt",
            &[("n", &n.to_string())],
        ),
    };
    add(
        "security",
        if blocked > 0 { "warn" } else { "ok" },
        title,
        none(),
        none(),
        "blocked",
    );

    // -- storage --
    for (name, path) in [
        (t("Eingang", &[]), app.cfg.inbox.clone()),
        (t("Bibliothek", &[]), app.cfg.library.clone()),
        (t("Konfiguration", &[]), app.cfg.config_dir.clone()),
    ] {
        checks.push(dir_check(&name, &path));
    }
    let mut add = |group: &str, status: &str, title: String, detail: String, hint: String, id: &str| {
        checks.push(check(group, status, title, detail, hint, id))
    };

    // -- services (the page tests the connected ones itself, one by one) --
    let src = app.usenet.public_config();
    let (abs_cfg, plex_cfg, ntfy) = (
        app.abs.public_config(),
        app.plex.public_config(),
        app.notifier.public_config(),
    );
    if src["indexers"].as_array().is_none_or(|a| a.is_empty()) {
        add(
            "services",
            "info",
            t(
                "Kein Indexer eingerichtet – Anfragen werden nicht automatisch besorgt",
                &[],
            ),
            none(),
            none(),
            "indexers",
        );
    }
    if src["sab"]["apikey_set"] != true {
        add(
            "services",
            "info",
            t("SABnzbd nicht eingerichtet", &[]),
            none(),
            none(),
            "sab",
        );
    }
    if ntfy["configured"] == true {
        match ntfy["last_error"].as_str() {
            Some(e) => add(
                "services",
                "error",
                t("ntfy: {error}", &[("error", e)]),
                none(),
                none(),
                "ntfy",
            ),
            None => add(
                "services",
                "ok",
                t(
                    "ntfy eingerichtet ({topic})",
                    &[("topic", ntfy["topic"].as_str().unwrap_or(""))],
                ),
                none(),
                none(),
                "ntfy",
            ),
        }
    } else {
        add(
            "services",
            "info",
            t("ntfy nicht eingerichtet – keine Benachrichtigungen aufs Handy", &[]),
            none(),
            none(),
            "ntfy",
        );
    }
    let region = app
        .settings()
        .get("ab_region")
        .and_then(Value::as_str)
        .unwrap_or("de")
        .to_string();
    let (shop, ff, sb) = tokio::task::spawn_blocking(move || {
        let url = format!(
            "{}/1.0/catalog/categories?root=Genres",
            crate::audible::api_host(&region)
        );
        (
            crate::audible::get_json(&url).map(|_| ()),
            media::ffmpeg_version(),
            media::sandbox_status(),
        )
    })
    .await
    .unwrap_or_else(|_| {
        (
            Err(crate::audible::AudibleError("intern".into())),
            String::new(),
            json!({}),
        )
    });
    match shop {
        Ok(()) => add(
            "services",
            "ok",
            t("Audible-Shop erreichbar", &[]),
            none(),
            none(),
            "audible",
        ),
        Err(e) => add(
            "services",
            "error",
            t("Audible-Shop nicht erreichbar: {error}", &[("error", &e.0)]),
            t(
                "Ohne Audible keine Erkennung, Tags und Vorschläge – Internetzugang des Containers prüfen.",
                &[],
            ),
            none(),
            "audible",
        ),
    }

    // -- system --
    add(
        "system",
        "ok",
        t(
            "Hörwerk {version}, läuft seit {since}",
            &[("version", &settings::version()), ("since", &when(app.started))],
        ),
        none(),
        none(),
        "version",
    );
    add(
        "system",
        if ff == t("nicht gefunden", &[]) { "error" } else { "ok" },
        t("ffmpeg {version}", &[("version", &ff)]),
        none(),
        none(),
        "ffmpeg",
    );
    if sb["active"] == true {
        add(
            "system",
            "ok",
            t(
                "ffmpeg abgeschottet (Landlock ABI {abi})",
                &[("abi", &sb["landlock_abi"].to_string())],
            ),
            none(),
            none(),
            "sandbox",
        );
    } else {
        add(
            "system",
            "warn",
            t("ffmpeg ohne Abschottung", &[]),
            t("Landlock ist nicht verfügbar. Kein Fehler, aber weniger Schutz.", &[]),
            none(),
            "sandbox",
        );
    }
    let q = app.engine.stats();
    let n = |a: &str, b: &str| q[a][b].to_string();
    add(
        "system",
        "info",
        t(
            "Warteschlangen: Umwandeln {e} aktiv / {ew} wartend, Erkennen & Ablegen {i} / {iw}",
            &[
                ("e", &n("encode", "running")),
                ("ew", &n("encode", "waiting")),
                ("i", &n("io", "running")),
                ("iw", &n("io", "waiting")),
            ],
        ),
        none(),
        none(),
        "queues",
    );
    let indexers: Vec<Value> = src["indexers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| i["enabled"] != false && i["apikey_set"] == true)
        .map(|i| json!({"id": i["id"], "name": if i["name"].as_str().is_some_and(|n| !n.is_empty()) { i["name"].clone() } else { i["url"].clone() }}))
        .collect();
    let ntfy_cfg = app
        .db
        .get("ntfy")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    json_reply(json!({
        "checks": checks, "server_time": now(), "app_url": app.notifier.link_base(&ntfy_cfg), "version": settings::version(),
        "services": {"indexers": indexers, "sab": src["sab"]["apikey_set"] == true && src["sab"]["url"].as_str().is_some_and(|u| !u.is_empty()),
                     "abs": !abs_cfg["url"].as_str().unwrap_or("").is_empty() && abs_cfg["token_set"] == true,
                     "plex": !plex_cfg["url"].as_str().unwrap_or("").is_empty() && plex_cfg["token_set"] == true},
    }))
}
