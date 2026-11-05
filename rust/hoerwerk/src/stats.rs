//! Statistics per family account (stats.py): who asked for how many books, when, how many hours, how much space.
//! Admins see every account and the totals; everyone else their own numbers and the family's totals.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Value, json};

use crate::app::App;
use crate::text::{self, now, t};

const MONTHS: usize = 12;

pub fn period_days(period: &str) -> Option<Option<i64>> {
    match period {
        "30" => Some(Some(30)),
        "90" => Some(Some(90)),
        "365" => Some(Some(365)),
        "all" => Some(None),
        _ => None,
    }
}

#[derive(Default)]
pub struct Stats {
    sizes: Mutex<HashMap<PathBuf, (f64, u64)>>,
}

#[derive(Clone)]
struct Acc {
    id: Value,
    name: String,
    role: String,
    n: BTreeMap<&'static str, i64>,
    wait: Vec<f64>,
    last: Option<f64>,
    genres: Vec<(String, i64)>,
}

const COUNTERS: [&str; 12] = [
    "requests",
    "books",
    "available",
    "open",
    "declined",
    "failed",
    "series",
    "minutes",
    "minutes_available",
    "bytes",
    "english",
    "_",
];

impl Acc {
    fn blank(id: Value, name: &str, role: &str) -> Self {
        Acc {
            id,
            name: name.into(),
            role: role.into(),
            n: COUNTERS.iter().map(|k| (*k, 0)).collect(),
            wait: Vec::new(),
            last: None,
            genres: Vec::new(),
        }
    }

    fn add(&mut self, k: &'static str, v: i64) {
        *self.n.entry(k).or_default() += v;
    }

    fn genre(&mut self, g: &str) {
        match self.genres.iter_mut().find(|(x, _)| x == g) {
            Some(e) => e.1 += 1,
            None => self.genres.push((g.to_string(), 1)),
        }
    }

    /// Counter.most_common(3): by count, ties in first-seen order.
    fn finish(mut self) -> Value {
        self.wait
            .sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mut g = self.genres.clone();
        g.sort_by_key(|x| std::cmp::Reverse(x.1));
        let mut out = serde_json::Map::new();
        out.insert("id".into(), self.id.clone());
        out.insert("name".into(), json!(self.name));
        out.insert("role".into(), json!(self.role));
        for k in COUNTERS.iter().filter(|k| **k != "_") {
            out.insert(k.to_string(), json!(self.n[k]));
        }
        out.insert("last".into(), json!(self.last));
        out.insert(
            "wait_median".into(),
            if self.wait.is_empty() {
                Value::Null
            } else {
                json!(self.wait[self.wait.len() / 2])
            },
        );
        out.insert(
            "top_genres".into(),
            json!(g.iter().take(3).map(|x| x.0.clone()).collect::<Vec<_>>()),
        );
        Value::Object(out)
    }
}

struct Row {
    id: i64,
    user_id: Option<i64>,
    book: Value,
    status: String,
    created: f64,
    updated: f64,
    data: Value,
}

fn rows(app: &App, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Vec<Row> {
    let c = app.db.lock();
    let Ok(mut st) = c.prepare(sql) else { return Vec::new() };
    st.query_map(params, |r| {
        Ok(Row {
            id: r.get("id")?,
            user_id: r.get("user_id")?,
            book: serde_json::from_str(&r.get::<_, String>("book")?).unwrap_or(json!({})),
            status: r.get("status")?,
            created: r.get("created")?,
            updated: r.get("updated")?,
            data: serde_json::from_str(&r.get::<_, Option<String>>("data")?.unwrap_or_default()).unwrap_or(json!({})),
        })
    })
    .map(|it| it.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

fn month_key(ts: f64) -> String {
    let (y, m) = text::local_month(ts);
    format!("{y:04}-{m:02}")
}

fn months(ts: f64) -> Vec<String> {
    let (mut y, mut m) = text::local_month(ts);
    let mut keys = Vec::new();
    for _ in 0..MONTHS {
        keys.push(format!("{y:04}-{m:02}"));
        m -= 1;
        if m == 0 {
            y -= 1;
            m = 12;
        }
    }
    keys.reverse();
    keys
}

impl Stats {
    fn folder(&self, app: &App, req_path: &str, book: &Value) -> Option<PathBuf> {
        let root = &app.library.root;
        if root.as_os_str().is_empty() {
            return None;
        }
        let mut candidates: Vec<PathBuf> = Vec::new();
        if !req_path.is_empty() {
            candidates.push(PathBuf::from(req_path));
        }
        if let Some(e) = app.library.find(book).first()
            && let Some(p) = e["path"].as_str().filter(|p| !p.is_empty())
        {
            candidates.push(root.join(p));
        }
        candidates.into_iter().find_map(|p| {
            let real = std::fs::canonicalize(&p).ok()?;
            (real.starts_with(root) && real != *root && real.is_dir()).then_some(real)
        })
    }

    /// Bytes of a book folder (cached per folder and change).
    pub fn size(&self, app: &App, req_path: &str, book: &Value) -> u64 {
        let Some(folder) = self.folder(app, req_path, book) else {
            return 0;
        };
        let Some(mtime) = crate::media::mtime(&folder) else {
            return 0;
        };
        if let Some((m, s)) = self.sizes.lock().unwrap_or_else(|p| p.into_inner()).get(&folder)
            && *m == mtime
        {
            return *s;
        }
        let total = dir_size(&folder);
        let mut c = self.sizes.lock().unwrap_or_else(|p| p.into_inner());
        if c.len() > 5000 {
            c.clear();
        }
        c.insert(folder, (mtime, total));
        total
    }

    pub fn summary(&self, app: &App, period: &str) -> Option<Value> {
        let days = period_days(period)?;
        let ts = now();
        let since = days.map_or(0.0, |d| ts - d as f64 * 86400.0);
        let mut order: Vec<Option<i64>> = Vec::new();
        let mut users: HashMap<Option<i64>, Acc> = HashMap::new();
        for u in app.accounts.list(&app.db) {
            let id = u["id"].as_i64();
            order.push(id);
            users.insert(
                id,
                Acc::blank(
                    u["id"].clone(),
                    u["name"].as_str().unwrap_or(""),
                    u["role"].as_str().unwrap_or("user"),
                ),
            );
        }
        let mut total = Acc::blank(Value::Null, "", "user");
        let mut groups: HashMap<String, Option<i64>> = HashMap::new();
        let month_keys = months(ts);
        let mut month_rows: BTreeMap<String, (i64, i64)> = month_keys.iter().map(|k| (k.clone(), (0, 0))).collect();
        let mut per_user: HashMap<Option<i64>, HashMap<String, (i64, i64)>> = HashMap::new();
        for row in rows(
            app,
            "SELECT id, user_id, book, status, created, updated, data FROM requests ORDER BY created",
            &[],
        ) {
            let minutes = row.book["runtime_min"].as_f64().map(|f| f as i64).unwrap_or(0);
            let key = month_key(row.created);
            if month_rows.contains_key(&key) && row.status != "declined" {
                let m = month_rows.get_mut(&key).expect("present");
                m.0 += 1;
                m.1 += minutes;
                let mine = per_user.entry(row.user_id).or_default().entry(key).or_default();
                mine.0 += 1;
                mine.1 += minutes;
            }
            if row.created < since {
                continue;
            }
            let uid = row.user_id;
            users.entry(uid).or_insert_with(|| {
                order.push(uid);
                Acc::blank(json!(uid), &t("gelöschtes Konto", &[]), "user")
            });
            count(users.get_mut(&uid).expect("present"), &row, minutes);
            count(&mut total, &row, minutes);
            match row.data["group"]["id"].as_str().filter(|g| !g.is_empty()) {
                Some(gid) => {
                    if !groups.contains_key(gid) {
                        groups.insert(gid.to_string(), uid);
                        let u = users.get_mut(&uid).expect("present");
                        u.add("series", 1);
                        u.add("requests", 1);
                        total.add("series", 1);
                        total.add("requests", 1);
                    }
                }
                None => {
                    users.get_mut(&uid).expect("present").add("requests", 1);
                    total.add("requests", 1);
                }
            }
            if row.status == "available" {
                let size = self.size(app, row.data["path"].as_str().unwrap_or(""), &row.book) as i64;
                let wait = (row.updated - row.created).max(0.0);
                for s in [users.get_mut(&uid).expect("present"), &mut total] {
                    s.add("bytes", size);
                    s.wait.push(wait);
                }
            }
        }
        let mut out: Vec<(Option<i64>, Value)> = order
            .iter()
            .filter_map(|id| users.get(id).map(|a| (*id, a.clone().finish())))
            .collect();
        for (_, s) in out.iter_mut() {
            s["listen"] = Value::Null;
        }
        out.sort_by(|a, b| {
            let k = |s: &Value| {
                (
                    -s["books"].as_i64().unwrap_or(0),
                    -s["minutes"].as_i64().unwrap_or(0),
                    s["name"].as_str().unwrap_or("").to_lowercase(),
                )
            };
            k(&a.1).cmp(&k(&b.1))
        });
        let users_out: Vec<Value> = out
            .into_iter()
            .map(|(id, mut s)| {
                let got = per_user.get(&id).cloned().unwrap_or_default();
                s["months"] = json!(
                    month_keys
                        .iter()
                        .map(|k| {
                            let (b, m) = got.get(k).copied().unwrap_or((0, 0));
                            json!({"month": k, "books": b, "minutes": m})
                        })
                        .collect::<Vec<_>>()
                );
                s
            })
            .collect();
        Some(json!({
            "period": period, "since": if since == 0.0 { Value::Null } else { json!(since) }, "users": users_out,
            "total": total.finish(), "listen": null, "listen_errors": [],
            "months": month_keys.iter().map(|k| { let (b, m) = month_rows[k]; json!({"month": k, "books": b, "minutes": m}) }).collect::<Vec<_>>(),
            "library_books": app.library.stats()["books"],
        }))
    }

    /// For admins: every account (without the month rows – those are in `account`).
    pub fn overview(&self, app: &App, period: &str) -> Option<Value> {
        let mut s = self.summary(app, period)?;
        for u in s["users"].as_array_mut().into_iter().flatten() {
            if let Some(o) = u.as_object_mut() {
                o.remove("months");
            }
        }
        s["scope"] = json!("all");
        Some(s)
    }

    /// For a family member: their own numbers next to everyone's together – nobody else by name.
    pub fn personal(&self, app: &App, uid: i64, period: &str) -> Option<Value> {
        let s = self.summary(app, period)?;
        let mut me = s["users"]
            .as_array()
            .and_then(|u| u.iter().find(|x| x["id"].as_i64() == Some(uid)).cloned())
            .unwrap_or_else(|| {
                let mut b = Acc::blank(json!(uid), "", "user").finish();
                b["listen"] = Value::Null;
                b
            });
        let mine: HashMap<String, i64> = me["months"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| {
                (
                    m["month"].as_str().unwrap_or("").to_string(),
                    m["books"].as_i64().unwrap_or(0),
                )
            })
            .collect();
        if let Some(o) = me.as_object_mut() {
            o.remove("months");
        }
        let months: Vec<Value> = s["months"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| {
                let mut m = m.clone();
                m["mine"] = json!(mine.get(m["month"].as_str().unwrap_or("")).copied().unwrap_or(0));
                m
            })
            .collect();
        Some(
            json!({"scope": "own", "period": s["period"], "since": s["since"], "me": me, "total": s["total"],
                    "accounts": s["users"].as_array().map_or(0, Vec::len), "listen": s["listen"], "months": months,
                    "library_books": s["library_books"]}),
        )
    }

    /// One account in detail: its numbers and its requests – None when there is no such account.
    pub fn account(&self, app: &App, uid: i64, period: &str) -> Option<Value> {
        let s = self.summary(app, period)?;
        let mut user = s["users"]
            .as_array()?
            .iter()
            .find(|u| u["id"].as_i64() == Some(uid))?
            .clone();
        let since = s["since"].as_f64().unwrap_or(0.0);
        let reqs: Vec<Value> = rows(
            app,
            "SELECT id, user_id, book, status, created, updated, data FROM requests WHERE user_id=? AND created>=? ORDER BY created DESC LIMIT 500",
            &[&uid, &since],
        )
        .into_iter()
        .map(|r| {
            let b = &r.book;
            json!({"id": r.id, "title": b["title"].as_str().unwrap_or(""), "authors": b.get("authors").filter(|a| !a.is_null()).cloned().unwrap_or(json!([])),
                   "series": b["series"], "series_position": b["series_position"], "cover_url": b["cover_url"],
                   "language": b["language"], "runtime_min": b["runtime_min"].as_f64().map(|f| f as i64).unwrap_or(0),
                   "status": r.status, "created": r.created, "updated": r.updated, "group": r.data["group"]["title"],
                   "bytes": if r.status == "available" { self.size(app, r.data["path"].as_str().unwrap_or(""), b) } else { 0 }})
        })
        .collect();
        let months = user
            .as_object_mut()
            .and_then(|o| o.remove("months"))
            .unwrap_or(json!([]));
        Some(
            json!({"period": s["period"], "since": s["since"], "user": user, "months": months, "requests": reqs, "heard": []}),
        )
    }
}

/// One request in the numbers of an account (and of the total).
fn count(s: &mut Acc, row: &Row, minutes: i64) {
    s.add("books", 1);
    s.last = Some(s.last.unwrap_or(0.0).max(row.created));
    match row.status.as_str() {
        "available" => {
            s.add("available", 1);
            s.add("minutes_available", minutes);
        }
        "declined" => s.add("declined", 1),
        "failed" => s.add("failed", 1),
        _ => s.add("open", 1),
    }
    if row.status != "declined" {
        s.add("minutes", minutes);
    }
    if row.book["language"].as_str().unwrap_or("").to_lowercase() == "english" {
        s.add("english", 1);
    }
    if let Some(g) = row.book["genres"]
        .as_array()
        .and_then(|g| g.first())
        .and_then(Value::as_str)
    {
        s.genre(g);
    }
}

fn dir_size(folder: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![folder.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(e.path());
            } else if ft.is_symlink() && e.path().is_dir() {
                // os.walk lists a linked folder as a folder and does not follow it
            } else if let Ok(m) = std::fs::symlink_metadata(e.path()) {
                total += m.len();
            }
        }
    }
    total
}
