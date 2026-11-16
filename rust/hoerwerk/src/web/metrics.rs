//! Prometheus metrics (`GET /api/metrics`, text exposition format): queues, jobs, requests, library, uptime.
//! Off unless the environment variable METRICS_TOKEN is set; then only with `Authorization: Bearer <token>`
//! (Prometheus: `authorization: { credentials: <token> }`). Nothing personal – counts only.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::body::Bytes;
use axum::http::StatusCode;

use super::{Reply, Req, error};
use crate::app::App;
use crate::text::{now, t};

type Params = HashMap<String, String>;

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn escape(v: &str) -> String {
    v.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

/// One metric with HELP/TYPE and its samples (label set, value).
fn family(out: &mut String, name: &str, kind: &str, help: &str, samples: &[(String, f64)]) {
    out.push_str(&format!("# HELP {name} {help}\n# TYPE {name} {kind}\n"));
    for (labels, v) in samples {
        if labels.is_empty() {
            out.push_str(&format!("{name} {v}\n"));
        } else {
            out.push_str(&format!("{name}{{{labels}}} {v}\n"));
        }
    }
}

fn one(v: f64) -> Vec<(String, f64)> {
    vec![(String::new(), v)]
}

fn by(label: &str, counts: &BTreeMap<String, f64>) -> Vec<(String, f64)> {
    counts
        .iter()
        .map(|(k, v)| (format!("{label}=\"{}\"", escape(k)), *v))
        .collect()
}

/// The page of metrics as text.
pub fn render(app: &App) -> String {
    let mut out = String::new();
    family(
        &mut out,
        "hoerwerk_info",
        "gauge",
        "Version of this Hörwerk.",
        &[(format!("version=\"{}\"", escape(&crate::settings::version())), 1.0)],
    );
    family(
        &mut out,
        "hoerwerk_uptime_seconds",
        "gauge",
        "Seconds since the start.",
        &one((now() - app.started).max(0.0).round()),
    );
    // audiobook jobs by status
    let status = app.pipeline.status(app);
    let mut jobs: BTreeMap<String, f64> = [
        "queued",
        "matching",
        "converting",
        "review",
        "tagging",
        "done",
        "failed",
        "cancelled",
    ]
    .iter()
    .map(|s| (s.to_string(), 0.0))
    .collect();
    for j in status["jobs"].as_array().into_iter().flatten() {
        *jobs
            .entry(j["status"].as_str().unwrap_or("").to_string())
            .or_insert(0.0) += 1.0;
    }
    family(
        &mut out,
        "hoerwerk_jobs",
        "gauge",
        "Audiobook jobs by status (the last 200 finished ones are kept).",
        &by("status", &jobs),
    );
    family(
        &mut out,
        "hoerwerk_inbox_waiting",
        "gauge",
        "Downloads in the inbox that are not yet quiet long enough to be processed.",
        &one(status["waiting"].as_array().map_or(0, Vec::len) as f64),
    );
    // queues
    let (mut waiting, mut running, mut workers) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    if let Some(q) = app.engine.stats().as_object() {
        for (name, v) in q {
            waiting.insert(name.clone(), v["waiting"].as_f64().unwrap_or(0.0));
            running.insert(name.clone(), v["running"].as_f64().unwrap_or(0.0));
            workers.insert(name.clone(), v["workers"].as_f64().unwrap_or(0.0));
        }
    }
    family(
        &mut out,
        "hoerwerk_queue_waiting",
        "gauge",
        "Steps waiting in a queue.",
        &by("queue", &waiting),
    );
    family(
        &mut out,
        "hoerwerk_queue_running",
        "gauge",
        "Steps running in a queue.",
        &by("queue", &running),
    );
    family(
        &mut out,
        "hoerwerk_queue_workers",
        "gauge",
        "Workers of a queue.",
        &by("queue", &workers),
    );
    family(
        &mut out,
        "hoerwerk_queue_steps_done_total",
        "counter",
        "Steps finished since the start.",
        &one(app.engine.done() as f64),
    );
    // requests by status
    let mut reqs: BTreeMap<String, f64> = BTreeMap::new();
    {
        let c = app.db.lock();
        if let Ok(mut st) = c.prepare("SELECT status, COUNT(*) FROM requests GROUP BY status")
            && let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        {
            for (s, n) in rows.flatten() {
                reqs.insert(s, n as f64);
            }
        }
    }
    family(
        &mut out,
        "hoerwerk_requests",
        "gauge",
        "Book requests by status.",
        &by("status", &reqs),
    );
    // library
    let lib = app.library.stats();
    family(
        &mut out,
        "hoerwerk_library_books",
        "gauge",
        "Books in the library index.",
        &one(lib["books"].as_f64().unwrap_or(0.0)),
    );
    family(
        &mut out,
        "hoerwerk_library_m4b_books",
        "gauge",
        "Of them already one M4B.",
        &one(lib["m4b"].as_f64().unwrap_or(0.0)),
    );
    family(
        &mut out,
        "hoerwerk_library_scanning",
        "gauge",
        "1 while the library is being read.",
        &one(if lib["scanning"] == true { 1.0 } else { 0.0 }),
    );
    family(
        &mut out,
        "hoerwerk_library_last_scan_timestamp_seconds",
        "gauge",
        "When the library was read the last time (Unix time, 0 = never).",
        &one(lib["last_scan"].as_f64().unwrap_or(0.0)),
    );
    family(
        &mut out,
        "hoerwerk_live_connections",
        "gauge",
        "Open live-update connections of browsers.",
        &one(app.sse_clients.load(std::sync::atomic::Ordering::SeqCst) as f64),
    );
    family(
        &mut out,
        "hoerwerk_locks_held",
        "gauge",
        "Internal locks held at this moment (a value that stays above 0 for minutes means something hangs).",
        &one(app.busy_locks().len() as f64),
    );
    out
}

pub async fn metrics(app: Arc<App>, req: Req, _: Params) -> Reply {
    let token = app.cfg.env.get("METRICS_TOKEN").cloned().unwrap_or_default();
    if token.is_empty() {
        return error(404, t("Nicht gefunden", &[]));
    }
    if !same(&req.bearer, &token) {
        return error(401, t("Zugangsschlüssel fehlt oder ist falsch", &[]));
    }
    let a = app.clone();
    let body = tokio::task::spawn_blocking(move || render(&a))
        .await
        .unwrap_or_default();
    Reply::Raw(
        StatusCode::OK,
        "text/plain; version=0.0.4; charset=utf-8",
        Bytes::from(body),
        vec![],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_escaped_and_compared_in_constant_time() {
        assert_eq!(escape("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
        assert!(same("geheim", "geheim"));
        assert!(!same("geheim", "geheiM"));
        assert!(!same("geheim", "geheim!"));
    }

    #[test]
    fn a_family_is_written_in_the_text_format() {
        let mut out = String::new();
        family(&mut out, "x_total", "counter", "Help.", &[(String::new(), 3.0)]);
        family(&mut out, "y", "gauge", "Help.", &[("a=\"b\"".into(), 1.0)]);
        assert_eq!(
            out,
            "# HELP x_total Help.\n# TYPE x_total counter\nx_total 3\n# HELP y Help.\n# TYPE y gauge\ny{a=\"b\"} 1\n"
        );
    }
}
