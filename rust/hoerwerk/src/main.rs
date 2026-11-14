//! Hörwerk – the audiobook workshop, Rust backend. Same API (contract/), same database, same frontend as app/.
// still being built: parts of the shared state are used by the pipeline that is not ported yet
#![allow(dead_code)]

mod accounts;
mod app;
mod audible;
mod authlog;
mod chapters;
mod config;
mod db;
mod difflib;
mod discover;
mod edition;
mod encoder;
mod engine;
mod filing;
mod follows;
mod free;
mod htmlent;
mod http;
mod invites;
mod library;
mod links;
mod listening;
mod media;
mod mediaservers;
mod notify;
mod packs;
mod pdftext;
mod pipeline;
mod quality;
mod releases;
mod requests;
mod rx;
mod settings;
mod stats;
mod tagger;
mod text;
mod totp;
mod uploads;
mod usenet;
mod web;
mod words;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rand::RngCore;

use crate::app::App;
use crate::config::Config;
use crate::db::Db;
use crate::text::t;

/// AUTH_USER/AUTH_PASS of older versions become the admin; without any account a setup code is shown.
fn first_account(app: &mut App) {
    let auth = !app.cfg.auth_user.is_empty() && !app.cfg.auth_pass.is_empty();
    if app.accounts.count(&app.db) > 0 {
        if auth {
            text::info(
                "Start",
                &t(
                    "AUTH_USER/AUTH_PASS werden nicht mehr gebraucht – Konten verwaltest du in der App.",
                    &[],
                ),
            );
        }
        return;
    }
    if auth {
        let name: String = app.cfg.auth_user.trim().chars().take(40).collect();
        let ts = text::now();
        let _ = app.db.lock().execute(
            "INSERT INTO users(name, pw, role, perms, created, pw_changed) VALUES(?,?,?,?,?,?)",
            rusqlite::params![name, accounts::hash_password(&app.cfg.auth_pass), "admin", "{}", ts, ts],
        );
        authlog::event("setup", &[("user", &app.cfg.auth_user), ("ip", "env")]);
        text::info(
            "Start",
            &t(
                "Admin-Konto „{name}“ aus AUTH_USER/AUTH_PASS angelegt.",
                &[("name", &app.cfg.auth_user)],
            ),
        );
        return;
    }
    let mut b = [0u8; 6];
    rand::rng().fill_bytes(&mut b);
    app.setup_code = b.chunks(2).map(hex::encode_upper).collect::<Vec<_>>().join("-");
    text::warn(
        "Anmeldung",
        &t(
            "Noch kein Konto: Admin im Browser anlegen. Von außerhalb des Heimnetzes braucht es den Einrichtungs-Code {code}.",
            &[("code", &app.setup_code)],
        ),
    );
}

/// The first library scan, then processing may start; if the library cannot be read, nothing is processed (no
/// duplicate check against a half-known library) and it is tried every minute. Afterwards every few hours.
async fn library_rescanner(app: Arc<App>) {
    use std::sync::atomic::Ordering;
    loop {
        let a = app.clone();
        match tokio::task::spawn_blocking(move || a.library.scan())
            .await
            .unwrap_or_else(|e| Err(e.to_string()))
        {
            Ok(_) => break,
            Err(e) => {
                *app.library_error.lock().unwrap_or_else(|p| p.into_inner()) = e.clone();
                text::error(
                    "Bibliothek",
                    &t(
                        "Bibliothek konnte nicht eingelesen werden – Verarbeitung wartet: {error}",
                        &[("error", &e)],
                    ),
                );
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        }
    }
    app.library_error.lock().unwrap_or_else(|p| p.into_inner()).clear();
    app.library_ready.store(true, Ordering::SeqCst);
    // only now: books are checked against the library for duplicates
    app.pipeline.note(String::new());
    app.engine.start();
    let mut waiting: Vec<_> = app
        .pipeline
        .all()
        .into_iter()
        .filter(|j| j.lock().status == "queued")
        .collect();
    waiting.sort_by(|a, b| {
        a.lock()
            .created
            .partial_cmp(&b.lock().created)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for j in &waiting {
        app.pipeline.schedule(&app, j);
    }
    std::thread::Builder::new()
        .name("library-trickle".into())
        .spawn({
            let app = app.clone();
            move || {
                while !app.stopping() {
                    std::thread::sleep(std::time::Duration::from_secs(
                        std::env::var("TRICKLE_SECONDS")
                            .ok()
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(60),
                    ));
                    app.pipeline.trickle_tick(&app);
                }
            }
        })
        .expect("trickle thread");
    std::thread::Builder::new()
        .name("follows".into())
        .spawn({
            let app = app.clone();
            move || {
                // a little after the start (the library is read first), then every half hour – each follow
                // itself is looked at once a day
                let mut wait = 600;
                while !app.stopping() {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    wait -= 1;
                    if wait <= 0 {
                        crate::follows::tick(&app);
                        wait = 1800;
                    }
                }
            }
        })
        .expect("follows thread");
    std::thread::Builder::new()
        .name("inbox-scanner".into())
        .spawn({
            let app = app.clone();
            move || app.pipeline.scanner(&app)
        })
        .expect("scanner thread");
    loop {
        let hours = app
            .settings()
            .get("library_rescan_hours")
            .and_then(|v| v.as_i64())
            .unwrap_or(6)
            .max(1) as u64;
        tokio::time::sleep(Duration::from_secs(hours * 3600)).await;
        let a = app.clone();
        if let Ok(Err(e)) = tokio::task::spawn_blocking(move || a.library.scan()).await {
            text::error(
                "Bibliothek",
                &t("Bibliothek konnte nicht eingelesen werden: {error}", &[("error", &e)]),
            );
        }
    }
}

/// Requests: search again when due, follow downloads and imports (every two minutes); the daily database snapshot.
fn request_worker(app: &Arc<App>) {
    std::thread::sleep(Duration::from_secs(60)); // let the library index load first
    let n = app.requests.import_wanted(app);
    if n > 0 {
        let msg = if n == 1 {
            t("1 Hörbuch aus „Gesucht“ als Anfrage übernommen", &[])
        } else {
            t(
                "{n} Hörbücher aus „Gesucht“ als Anfragen übernommen",
                &[("n", &n.to_string())],
            )
        };
        text::info("Anfragen", &msg);
    }
    while !app.stopping() {
        app.requests.tick(app);
        discover::tick(&(app.clone() as Arc<dyn discover::Env>)); // review list daily, late audiobooks monthly
        app.maintain_db(text::now());
        std::thread::sleep(Duration::from_secs(120));
    }
}

async fn stop_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}

/// `hoerwerk health`: the container's health check (the image has no curl or Python) – exit 0 when /api/health answers.
fn health_check() -> ! {
    use std::io::{Read, Write};
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8080);
    let ok = (|| -> std::io::Result<bool> {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut s = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(4))?;
        s.set_read_timeout(Some(Duration::from_secs(4)))?;
        s.write_all(b"GET /api/health HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")?;
        let mut head = [0u8; 12];
        s.read_exact(&mut head)?;
        Ok(head.starts_with(b"HTTP/1.") && &head[9..12] == b"200")
    })()
    .unwrap_or(false);
    std::process::exit(if ok { 0 } else { 1 });
}

#[tokio::main]
async fn main() {
    if std::env::args().nth(1).as_deref() == Some("health") {
        health_check();
    }
    if let Some(mask) = std::env::var("UMASK")
        .ok()
        .and_then(|m| u32::from_str_radix(m.trim(), 8).ok())
    {
        // SAFETY: umask only changes the creation mask of this process
        unsafe { libc_umask(mask) };
    }
    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            text::error("Start", &format!("Start abgebrochen: {e}"));
            std::process::exit(2);
        }
    };
    text::load_catalog(&cfg.static_dir);
    authlog::setup(&cfg.config_dir);
    let _ = std::fs::create_dir_all(&cfg.config_dir);
    let db = match Db::open(&cfg.config_dir.join("hoerwerk.db")) {
        Ok(d) => d,
        Err(e) => {
            text::error("Start", &format!("Start abgebrochen: {e}"));
            std::process::exit(2);
        }
    };
    let mut app = App::new(cfg, db);
    text::set_language(app.settings().get("language").and_then(|v| v.as_str()).unwrap_or("de"));
    first_account(&mut app);
    let app = Arc::new(app);
    App::wire(&app);
    discover::cleanup(&app.db);
    app.pipeline.load(&app);
    free::cleanup(&app.cfg.inbox);
    let recovered = app.uploads.recover(); // uploads cannot continue after a restart; half-finished ones are completed
    if !recovered.is_empty() {
        app.pipeline.arrived(&recovered);
    }
    app.pipeline.note(t("Wartet, bis die Bibliothek eingelesen ist …", &[]));
    tokio::spawn(library_rescanner(app.clone()));
    std::thread::spawn({
        let app = app.clone();
        move || request_worker(&app)
    });
    std::thread::spawn({
        let app = app.clone();
        move || {
            while !app.stopping() {
                notify::watch_downloads(&app); // finished / failed SABnzbd downloads: tell, and look at the inbox
                std::thread::sleep(Duration::from_secs(30));
            }
        }
    });
    let port = app.cfg.port;
    text::info(
        "Start",
        &format!(
            "Hörwerk {} läuft auf Port {port} – Eingang: {}, Bibliothek: {}",
            settings::version(),
            app.cfg.inbox.display(),
            app.cfg.library.display()
        ),
    );
    let router = axum::Router::new().fallback(web::dispatch).with_state(app.clone());
    let listener = match tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port))).await {
        Ok(l) => l,
        Err(e) => {
            text::error("Start", &format!("Start abgebrochen: Port {port}: {e}"));
            std::process::exit(2);
        }
    };
    let stopper = app.clone();
    let server = axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async move {
            stop_signal().await;
            let _ = stopper.shutdown.send(true); // live streams end, no new requests
            stopper.changed.notify_waiters();
        });
    let mut stopped = app.shutdown.subscribe();
    let mut serving = tokio::spawn(async move { server.await });
    tokio::select! {
        r = &mut serving => {
            if let Ok(Err(e)) = r {
                text::error("Start", &format!("Server: {e}"));
            }
        }
        _ = stopped.wait_for(|s| *s) => {
            // requests still running get a few seconds, then we stop anyway
            let _ = tokio::time::timeout(Duration::from_secs(5), serving).await;
        }
    }
    // orderly shutdown within Docker's grace time: ffmpeg ends, running steps save their state (their jobs continue
    // after the next start), the database is closed
    text::info("Start", &t("Hörwerk wird beendet …", &[]));
    let a = app.clone();
    let _ = tokio::task::spawn_blocking(move || {
        a.pipeline.shutdown();
        if !a.engine.stop(Duration::from_secs(6)) {
            text::warn(
                "Start",
                &t(
                    "Nicht alle laufenden Schritte sind rechtzeitig fertig geworden – sie laufen nach dem Start neu",
                    &[],
                ),
            );
        }
        a.pipeline.save(&a);
    })
    .await;
    app.db.close();
    text::info("Start", &t("Hörwerk beendet.", &[]));
}

unsafe extern "C" {
    #[link_name = "umask"]
    fn libc_umask(mask: u32) -> u32;
}
