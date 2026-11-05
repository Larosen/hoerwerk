//! Indexer search -> SABnzbd with the library cross-check, and the book search in the shops (routes/search.py).

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use super::{Reply, Req, error, json_reply};
use crate::app::App;
use crate::text::t;
use crate::words::match_words;
use crate::{audible, links, packs, releases, requests};

type Params = HashMap<String, String>;

const BOOK_OUT: [&str; 19] = [
    "asin",
    "title",
    "subtitle",
    "authors",
    "narrators",
    "series",
    "series_asin",
    "series_position",
    "runtime_min",
    "release_date",
    "cover_url",
    "format_type",
    "url",
    "source",
    "language",
    "isbn",
    "id",
    "region",
    "genres",
];

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let lang = crate::text::language();
    tokio::task::spawn_blocking(move || {
        crate::text::set_language(&lang);
        f()
    })
    .await
    .expect("worker thread")
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

fn book_key(b: &Value) -> String {
    let first = b["authors"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .unwrap_or("");
    let last = first
        .split_whitespace()
        .last()
        .map(|w| match_words(w).into_iter().next().unwrap_or_default())
        .unwrap_or_default();
    format!("{}|{last}", match_words(s(b, "title")).join(" "))
}

fn owned(entries: &[Value]) -> Value {
    let Some(e) = entries.first() else { return Value::Null };
    json!({"path": e["path"], "title": e["title"], "authors": e["authors"],
           "why": if e["why"].is_null() { json!("similar") } else { e["why"].clone() }, "count": entries.len()})
}

fn region(app: &App, req: &Req) -> Result<String, Reply> {
    let r = req.str("region");
    let r = if r.is_empty() {
        app.settings()
            .get("ab_region")
            .and_then(Value::as_str)
            .unwrap_or("de")
            .to_string()
    } else {
        r
    };
    if !audible::REGIONS.iter().any(|x| x.0 == r) {
        return Err(error(400, t("Unbekannte Region", &[])));
    }
    Ok(r)
}

/// Releases at the indexers (admins): every result marked when the library has it already.
pub async fn indexers(app: Arc<App>, req: Req, _: Params) -> Reply {
    let query = req.str("query").trim().to_string();
    if query.chars().count() < 2 {
        return error(400, t("Suchbegriff fehlt", &[]));
    }
    let extra: Vec<String> = req
        .get("extra")
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(str::to_string))
        .take(3)
        .collect();
    let language = req.str("language");
    let book = if req.get("book").is_object() {
        requests::clean(req.get("book")).ok()
    } else {
        None
    }; // no author: the plain search
    let a = app.clone();
    let q = query.clone();
    let result = blocking(move || match &book {
        Some(b) => {
            // a picked book: round by round, broader only while no release fits it
            let lang = language.clone();
            let fits = |found: &[Value]| {
                found
                    .iter()
                    .any(|r| releases::evaluate(r, b, &lang, true, &[], true)["ok"] == true)
            };
            a.usenet
                .search_stages(&releases::search_stages(b), &fits, &language, releases::SEARCH_BUDGET)
                .map(|mut res| {
                    let mut list = res["results"].as_array().cloned().unwrap_or_default();
                    let ranked = a.usenet.rank_results(&mut list, &q, &language);
                    res["results"] = json!(ranked);
                    res
                })
        }
        None => a.usenet.search(&q, &language, &extra, true, 6),
    })
    .await;
    let mut res = match result {
        Ok(r) => r,
        Err(e) => return error(400, e.to_string()),
    };
    let lib = &app.library;
    if let Some(list) = res["results"].as_array_mut() {
        for r in list.iter_mut() {
            let title = s(r, "title").to_string();
            r["owned"] = owned(&lib.match_release(&title, false));
            if let Some(pack) = packs::pack_info(&title) {
                // several volumes in one download: which of them are here already?
                let have = lib.series_owned(s(&pack, "series"), s(&pack, "author"));
                let mut p = pack.clone();
                let owned_map: serde_json::Map<String, Value> = pack["volumes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_i64)
                    .filter_map(|v| have.get(&v).map(|w| (v.to_string(), json!(w))))
                    .collect();
                p["owned"] = Value::Object(owned_map);
                r["pack"] = p;
                r["owned"] = Value::Null;
            }
        }
    }
    // what the user typed may already be in the library, even if no release name says so
    res["library"] = json!(
        lib.match_release(&query, true)
            .iter()
            .take(3)
            .map(|e| json!({"path": e["path"], "title": e["title"], "authors": e["authors"], "format": e["format"]}))
            .collect::<Vec<_>>()
    );
    json_reply(res)
}

fn for_browser(app: &App, req: &Req, books: &[Value]) -> Vec<Value> {
    let requested = app.requests.status_by_key(app);
    let admin = req.user.as_ref().is_some_and(|u| u["role"] == "admin");
    books
        .iter()
        .map(|b| {
            let mut item: serde_json::Map<String, Value> = BOOK_OUT
                .iter()
                .map(|k| (k.to_string(), b.get(*k).cloned().unwrap_or(Value::Null)))
                .collect();
            let o = owned(&app.library.find(b));
            // family accounts learn *that* a book is there, not where it lies on the server
            item.insert(
                "owned".into(),
                if admin || o.is_null() {
                    o
                } else {
                    json!({"title": o["title"]})
                },
            );
            item.insert(
                "requested".into(),
                requested.get(&requests::book_key(b)).cloned().unwrap_or(Value::Null),
            );
            Value::Object(item)
        })
        .collect()
}

/// Look the book up at Audible first (German title and author as the shop lists them), BookBeat second.
pub async fn books(app: Arc<App>, req: Req, _: Params) -> Reply {
    let query = req.str("query").trim().to_string();
    if query.chars().count() < 2 {
        return error(400, t("Suchbegriff fehlt", &[]));
    }
    let region = match region(&app, &req) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let language = req.str("language");
    if !matches!(language.as_str(), "" | "de" | "en") {
        return error(400, t("Ungültige Daten", &[]));
    }
    let cq = audible::clean_query(&query);
    let hints = json!({"query": if cq.is_empty() { query.clone() } else { cq }, "asin": audible::find_asin(&[&query])});
    let fallback = app
        .settings()
        .get("ab_fallback")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let (a, r, lang) = (app.clone(), region.clone(), language.clone());
    let found = blocking(move || {
        let (mut books, err) = match a.providers.search(&hints, &r, fallback) {
            Ok(b) => (b.into_iter().take(8).collect::<Vec<_>>(), None),
            Err(e) => (Vec::new(), Some(e.0)),
        };
        // BookBeat as a second shop: only what Audible doesn't have (same title and author)
        let seen: Vec<String> = books.iter().map(book_key).collect();
        let extra: Vec<Value> = links::bookbeat_search(s(&hints, "query"), &r, 8)
            .into_iter()
            .filter(|b| {
                !seen.contains(&book_key(b))
                    && matches!(
                        if s(b, "language").is_empty() {
                            "german"
                        } else {
                            s(b, "language")
                        },
                        "german" | "deutsch"
                    )
            })
            .collect();
        books.extend(audible::rank(&extra, &hints).into_iter().take(6));
        if !lang.is_empty() {
            let mut wanted: Vec<Value> = books
                .iter()
                .filter(|b| matches!(releases::book_language(b).as_str(), l if l == lang || l.is_empty()))
                .cloned()
                .collect();
            if wanted.is_empty() && lang == "en" && !matches!(r.as_str(), "us" | "uk") {
                // an English edition the German store doesn't list: ask the US store
                wanted = a
                    .providers
                    .search(&hints, "us", fallback)
                    .map(|b| {
                        b.into_iter()
                            .take(8)
                            .filter(|x| releases::book_language(x) == "en")
                            .collect()
                    })
                    .unwrap_or_default();
            }
            books = wanted;
        }
        (books, err)
    })
    .await;
    let (books, err) = found;
    if books.is_empty()
        && let Some(e) = err
    {
        return error(502, e);
    }
    json_reply(json!({"books": for_browser(&app, &req, &books), "region": region}))
}

/// All released volumes of a series, in order – for a series request.
pub async fn series(app: Arc<App>, req: Req, _: Params) -> Reply {
    let asin = req.str("asin").trim().to_uppercase();
    let r = req.str("region");
    let region = if r.is_empty() {
        app.settings()
            .get("ab_region")
            .and_then(Value::as_str)
            .unwrap_or("de")
            .to_string()
    } else {
        r
    };
    let language = match req.str("language").as_str() {
        "de" => "german",
        "en" => "english",
        _ => "",
    };
    let valid = asin.len() == 10 && asin.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    if !valid || !audible::REGIONS.iter().any(|x| x.0 == region) {
        return error(400, t("Ungültige Daten", &[]));
    }
    let (a, rg) = (app.clone(), region.clone());
    match blocking(move || a.providers.client.series(&asin, &rg, language, audible::SERIES_MAX)).await {
        Ok(series) => {
            let books = series["books"].as_array().cloned().unwrap_or_default();
            json_reply(
                json!({"series": {"asin": series["asin"], "title": series["title"], "region": region}, "books": for_browser(&app, &req, &books)}),
            )
        }
        Err(e) => error(502, e.0),
    }
}

/// A release from the search to SABnzbd (admins).
pub async fn send(app: Arc<App>, req: Req, _: Params) -> Reply {
    let mut hints = serde_json::Map::new();
    for k in ["title", "author", "asin"] {
        let v = req.get(k);
        if !(v.is_null() || v == &json!("") || v == &json!(false) || v == &json!(0)) {
            hints.insert(k.into(), v.clone());
        }
    }
    let vols = req.get("volumes");
    if !vols.is_null() {
        let Some(list) = vols.as_array().filter(|l| {
            !l.is_empty()
                && l.iter()
                    .all(|v| v.as_i64().is_some_and(|n| (0..1000).contains(&n)) && v.is_i64())
        }) else {
            return error(400, t("Bitte mindestens einen Band auswählen", &[]));
        };
        let mut v: Vec<i64> = list.iter().filter_map(Value::as_i64).collect();
        v.sort();
        v.dedup();
        hints.insert("volumes".into(), json!(v));
    }
    let id = req.str("id");
    let a = app.clone();
    match blocking(move || a.usenet.send(&id, &Value::Object(hints))).await {
        Ok(res) => {
            app.notifier.send(
                "download_sent",
                &t("Download an SABnzbd geschickt", &[]),
                res["title"].as_str().unwrap_or(""),
                None,
            );
            json_reply(res)
        }
        Err(e) => error(400, e.to_string()),
    }
}
