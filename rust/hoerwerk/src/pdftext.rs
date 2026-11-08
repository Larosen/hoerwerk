//! Text with positions from simple PDFs – enough for Deutschlandfunk's monthly best-of lists.
//! Deliberately small and defensive: only FlateDecode streams are unpacked (size-limited each and in total),
//! images are skipped, nothing is executed or rendered. Text is decoded with the font's ToUnicode map
//! (two-byte Identity-H fonts) or as WinAnsi; positions come from Tm/Td/TD/T*.

use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use regex::bytes::Regex as BRegex;
use serde_json::{Value, json};

pub const MAX_PDF: usize = 8 << 20;
pub const MAX_STREAM: usize = 4 << 20;
const MAX_TOTAL: usize = 24 << 20;
const MAX_TOKENS: usize = 400_000;

macro_rules! brx {
    ($p:expr) => {{
        static R: LazyLock<BRegex> = LazyLock::new(|| BRegex::new($p).expect("valid regex"));
        &*R
    }};
}

/// x, y, text
pub type Chunk = (f64, f64, String);
type Objects = BTreeMap<u64, (Vec<u8>, Option<Vec<u8>>)>;

fn num(b: &[u8]) -> u64 {
    b.iter()
        .fold(0u64, |n, d| n.saturating_mul(10).saturating_add(u64::from(d - b'0')))
}

fn inflate(data: &[u8], left: &mut usize) -> Vec<u8> {
    if *left == 0 || data.is_empty() {
        return Vec::new();
    }
    let limit = MAX_STREAM.min(*left);
    let mut d = flate2::Decompress::new(true);
    let mut out: Vec<u8> = Vec::with_capacity(limit.min(1 << 16));
    loop {
        let before = (d.total_in(), d.total_out());
        let room = limit - out.len();
        if room == 0 {
            break;
        }
        out.reserve(room.min(1 << 20));
        let input = &data[d.total_in() as usize..];
        let spare = (out.capacity() - out.len()).min(room);
        let start = out.len();
        out.resize(start + spare, 0);
        let res = d.decompress(input, &mut out[start..], flate2::FlushDecompress::None);
        let produced = (d.total_out() - before.1) as usize;
        out.truncate(start + produced);
        match res {
            Err(_) => return Vec::new(),
            Ok(flate2::Status::StreamEnd) => break,
            Ok(_) if (d.total_in(), d.total_out()) == before => break, // input used up
            Ok(_) => {}
        }
    }
    *left -= out.len();
    out
}

/// {number: (dictionary, unpacked stream or None)} – object streams (/ObjStm) included.
pub fn objects(raw: &[u8]) -> Objects {
    let mut left = MAX_TOTAL;
    let mut objs = Objects::new();
    let mut order: Vec<u64> = Vec::new();
    for m in brx!(r"(?s-u)(\d{1,7})\s+0\s+obj\b(.*?)\bendobj").captures_iter(raw) {
        let n = num(&m[1]);
        let body = &m[2];
        if !objs.contains_key(&n) {
            order.push(n);
        }
        let Some(sm) = brx!(r"(?-u)stream\r?\n").find(body) else {
            objs.insert(n, (body.to_vec(), None));
            continue;
        };
        let head = &body[..sm.start()];
        let mut data = &body[sm.end()..];
        data = match data.windows(9).rposition(|w| w == b"endstream") {
            Some(p) => &data[..p],
            None => &data[..data.len().saturating_sub(1)],
        };
        let data = if brx!(r"(?-u)/Subtype\s*/Image").is_match(head) {
            Vec::new()
        } else if !head.windows(7).any(|w| w == b"/Filter") {
            data[..data.len().min(MAX_STREAM)].to_vec()
        } else if brx!(r"(?-u)/Filter\s*(?:/FlateDecode\b|\[\s*/FlateDecode\s*\])").is_match(head) {
            inflate(data, &mut left)
        } else {
            Vec::new()
        };
        objs.insert(n, (head.to_vec(), Some(data)));
    }
    let mut extra: Vec<(u64, Vec<u8>)> = Vec::new();
    for k in &order {
        let (head, data) = &objs[k];
        let Some(data) = data.as_ref().filter(|d| !d.is_empty()) else {
            continue;
        };
        if !brx!(r"(?-u)/Type\s*/ObjStm").is_match(head) {
            continue;
        }
        let (Some(n), Some(first)) = (
            brx!(r"(?-u)/N\s+(\d+)").captures(head),
            brx!(r"(?-u)/First\s+(\d+)").captures(head),
        ) else {
            continue;
        };
        let first_at = num(&first[1]).min(usize::MAX as u64) as usize;
        let count = 2 * num(&n[1]).min(10000) as usize;
        let nums: Vec<u64> = data[..first_at.min(data.len())]
            .split(|c| b" \t\n\r\x0b\x0c".contains(c))
            .filter(|x| !x.is_empty())
            .take(count)
            .filter(|x| x.iter().all(u8::is_ascii_digit))
            .map(num)
            .collect();
        let pairs: Vec<(u64, u64)> = nums.chunks_exact(2).map(|p| (p[0], p[1])).collect();
        for (i, (no, off)) in pairs.iter().enumerate() {
            let start = first_at.saturating_add(*off as usize).min(data.len());
            let end = match pairs.get(i + 1) {
                Some(next) => first_at.saturating_add(next.1 as usize).min(data.len()),
                None => data.len(),
            };
            extra.push((*no, data[start..end.max(start)].to_vec()));
        }
    }
    for (no, body) in extra {
        objs.entry(no).or_insert((body, None));
    }
    objs
}

fn reference(text: &[u8], key: &str) -> Option<u64> {
    let re = BRegex::new(&format!(r"(?-u)/{key}\s*(\d+)\s+0\s+R")).ok()?;
    re.captures(text).map(|c| num(&c[1]))
}

fn hex_bytes(h: &[u8]) -> Result<Vec<u8>, String> {
    if h.len() % 2 == 1 {
        return Err("non-hexadecimal number found in fromhex() arg".into());
    }
    hex::decode(h).map_err(|e| e.to_string())
}

fn hex_int(h: &[u8]) -> u32 {
    h.iter()
        .fold(0u32, |n, c| (n << 4) | (*c as char).to_digit(16).unwrap_or(0))
}

fn utf16be(b: &[u8]) -> String {
    let units: Vec<u16> = b.chunks_exact(2).map(|p| u16::from_be_bytes([p[0], p[1]])).collect();
    let mut s = String::from_utf16_lossy(&units);
    if b.len() % 2 == 1 {
        s.push('\u{fffd}');
    }
    s
}

/// ToUnicode map: bfchar and bfrange (the plain start-code form).
fn cmap(data: &[u8]) -> Result<HashMap<u32, String>, String> {
    let mut out = HashMap::new();
    for block in brx!(r"(?s-u)beginbfchar(.*?)endbfchar").captures_iter(data) {
        for c in brx!(r"(?-u)<([0-9A-Fa-f]{2,8})>\s*<([0-9A-Fa-f]{2,64})>").captures_iter(&block[1]) {
            out.insert(hex_int(&c[1]), utf16be(&hex_bytes(&c[2])?));
        }
    }
    for block in brx!(r"(?s-u)beginbfrange(.*?)endbfrange").captures_iter(data) {
        for c in
            brx!(r"(?-u)<([0-9A-Fa-f]{2,8})>\s*<([0-9A-Fa-f]{2,8})>\s*<([0-9A-Fa-f]{2,8})>").captures_iter(&block[1])
        {
            let (lo, hi, start) = (hex_int(&c[1]) as i64, hex_int(&c[2]) as i64, hex_int(&c[3]) as i64);
            for i in lo..=hi.min(lo + 4096) {
                let cp = start + i - lo;
                if (0..0x110000).contains(&cp) {
                    out.insert(i as u32, char::from_u32(cp as u32).unwrap_or('\u{fffd}').to_string());
                }
            }
        }
    }
    Ok(out)
}

fn string(tok: &[u8]) -> Vec<u8> {
    if tok.starts_with(b"<") {
        let mut hexed: Vec<u8> = tok[1..tok.len() - 1]
            .iter()
            .copied()
            .filter(|c| !c.is_ascii_whitespace() && *c != 0x0b)
            .collect();
        if hexed.len() % 2 == 1 {
            hexed.push(b'0');
        }
        return hex::decode(&hexed).unwrap_or_default();
    }
    let s = if tok.len() >= 2 {
        &tok[1..tok.len() - 1]
    } else {
        &[][..]
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c == b'\\' && i + 1 < s.len() {
            let n = s[i + 1];
            match n {
                b'n' | b'r' | b't' | b'b' | b'f' => {
                    out.push(match n {
                        b'n' => b'\n',
                        b'r' => b'\r',
                        b't' => b'\t',
                        b'b' => 8,
                        _ => 12,
                    });
                    i += 2;
                }
                b'0'..=b'7' => {
                    let mut j = i + 1;
                    while j < s.len() && j < i + 4 && (b'0'..=b'7').contains(&s[j]) {
                        j += 1;
                    }
                    out.push(s[i + 1..j].iter().fold(0u32, |v, d| v * 8 + u32::from(d - b'0')) as u8);
                    i = j;
                }
                _ => {
                    out.push(n);
                    i += 2;
                }
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

const CP1252: [char; 32] = [
    '€', '\u{fffd}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{fffd}', 'Ž', '\u{fffd}', '\u{fffd}',
    '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{fffd}', 'ž', 'Ÿ',
];

fn cp1252(b: u8) -> char {
    if (0x80..0xa0).contains(&b) {
        CP1252[(b - 0x80) as usize]
    } else {
        b as char
    }
}

type Font = (Option<HashMap<u32, String>>, bool);

fn fonts(objs: &Objects, page: &[u8]) -> Result<HashMap<Vec<u8>, Font>, String> {
    let mut res: &[u8] = page;
    if let Some(n) = reference(page, "Resources")
        && let Some(o) = objs.get(&n)
    {
        res = &o.0;
    }
    let mut out = HashMap::new();
    let Some(fm) = brx!(r"(?s-u)/Font\s*(<<.*?>>|\d+\s+0\s+R)").captures(res) else {
        return Ok(out);
    };
    let mut table: &[u8] = &fm[1];
    if table.ends_with(b"R") {
        let first: Vec<u8> = table.iter().copied().take_while(u8::is_ascii_digit).collect();
        table = objs.get(&num(&first)).map_or(&[][..], |o| &o.0);
    }
    for c in brx!(r"(?-u)/([^\s/<>\[\]]+)\s+(\d+)\s+0\s+R").captures_iter(table) {
        let head = objs.get(&num(&c[2])).map_or(&[][..], |o| &o.0[..]);
        let map = match reference(head, "ToUnicode").and_then(|tu| objs.get(&tu)) {
            Some(o) => Some(cmap(o.1.as_deref().unwrap_or(b""))?),
            None => None,
        };
        let two = head.windows(10).any(|w| w == b"Identity-H");
        out.insert(c[1].to_vec(), (map, two));
    }
    Ok(out)
}

enum Tok<'a> {
    B(&'a [u8]),
    Arr(Vec<Tok<'a>>),
}

fn float(t: &Tok) -> Option<f64> {
    match t {
        Tok::B(b) => std::str::from_utf8(b).ok()?.trim().parse().ok(),
        Tok::Arr(_) => None,
    }
}

fn decode(font: &Font, bs: &[u8]) -> String {
    let (table, two) = font;
    let table = table.as_ref().filter(|t| !t.is_empty());
    if *two {
        return bs
            .chunks_exact(2)
            .map(|p| {
                table
                    .and_then(|t| t.get(&u32::from(u16::from_be_bytes([p[0], p[1]]))))
                    .map_or("", |s| s.as_str())
            })
            .collect();
    }
    match table {
        Some(t) => bs
            .iter()
            .map(|b| t.get(&u32::from(*b)).cloned().unwrap_or_else(|| cp1252(*b).to_string()))
            .collect(),
        None => bs.iter().map(|b| cp1252(*b)).collect(),
    }
}

fn page_chunks(objs: &Objects, page: &[u8]) -> Result<Vec<Chunk>, String> {
    let fonts = fonts(objs, page)?;
    let mut data: Vec<u8> = Vec::new();
    if let Some(cm) = brx!(r"(?-u)/Contents\s*(\[[^\]]*\]|\d+\s+0\s+R)").captures(page) {
        for (i, r) in brx!(r"(?-u)(\d+)\s+0\s+R").captures_iter(&cm[1]).enumerate() {
            if i > 0 {
                data.push(b'\n');
            }
            if let Some((_, Some(s))) = objs.get(&num(&r[1])) {
                data.extend_from_slice(s);
            }
        }
    }
    let token =
        brx!(r#"(?s-u)\((?:\\.|[^\\)])*\)|<[0-9A-Fa-f\s]*>|\[|\]|/[^\s/\[\]()<>{}]+|[-+]?\d*\.?\d+|[A-Za-z'"*]+"#);
    let operator = brx!(r#"(?-u)^[A-Za-z'"*]+$"#);
    let none: Font = (None, false);
    let mut font: &Font = &none;
    let mut chunks = Vec::new();
    let mut stack: Vec<Tok> = Vec::new();
    let (mut x, mut y, mut lx, mut ly, mut lead) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (n, m) in token.find_iter(&data).enumerate() {
        if n > MAX_TOKENS {
            break;
        }
        let tok = m.as_bytes();
        if tok == b"[" {
            stack.push(Tok::B(tok));
            continue;
        }
        if tok == b"]" {
            let mut arr = Vec::new();
            while let Some(top) = stack.pop() {
                if matches!(top, Tok::B(b"[")) {
                    break;
                }
                arr.push(top);
            }
            arr.reverse();
            stack.push(Tok::Arr(arr));
            continue;
        }
        if !operator.is_match(tok) {
            stack.push(Tok::B(tok));
            continue;
        }
        let k = stack.len();
        {
            match tok {
                b"Tf" if k >= 2 => {
                    if let Tok::B(name) = &stack[k - 2] {
                        font = fonts.get(name.get(1..).unwrap_or(b"")).unwrap_or(&none);
                    }
                }
                b"Tm" if k >= 6 => {
                    if let (Some(a), Some(b)) = (float(&stack[k - 2]), float(&stack[k - 1])) {
                        (x, lx, y, ly) = (a, a, b, b);
                    }
                }
                b"Td" | b"TD" if k >= 2 => {
                    if let (Some(a), Some(b)) = (float(&stack[k - 2]), float(&stack[k - 1])) {
                        lx += a;
                        ly += b;
                        (x, y) = (lx, ly);
                        if tok == b"TD" {
                            lead = -b;
                        }
                    }
                }
                b"TL" if k >= 1 => {
                    if let Some(v) = float(&stack[k - 1]) {
                        lead = v;
                    }
                }
                b"T*" => {
                    ly -= lead;
                    (x, y) = (lx, ly);
                }
                b"BT" => (x, y, lx, ly) = (0.0, 0.0, 0.0, 0.0),
                b"Tj" | b"'" | b"\"" if k >= 1 => {
                    if let Tok::B(s) = &stack[k - 1] {
                        chunks.push((x, y, decode(font, &string(s))));
                    }
                }
                b"TJ" if k >= 1 => {
                    if let Tok::Arr(arr) = &stack[k - 1] {
                        let mut text = String::new();
                        let mut broken = false;
                        for e in arr {
                            match e {
                                Tok::B(b) if b.starts_with(b"(") || b.starts_with(b"<") => {
                                    text += &decode(font, &string(b))
                                }
                                Tok::B(_) => match float(e) {
                                    Some(v) if v < -200.0 => text.push(' '),
                                    Some(_) => {}
                                    None => broken = true,
                                },
                                Tok::Arr(_) => {}
                            }
                        }
                        if !broken {
                            chunks.push((x, y, text));
                        }
                    }
                }
                _ => {}
            }
        }
        stack.clear();
    }
    Ok(chunks)
}

/// Every page's text pieces with their positions (x to the right, y upwards).
pub fn pages(raw: &[u8]) -> Result<Vec<Vec<Chunk>>, String> {
    if !raw.starts_with(b"%PDF") || raw.len() > MAX_PDF {
        return Err("no PDF".into());
    }
    let objs = objects(raw);
    objs.values()
        .filter(|(head, _)| brx!(r"(?-u)/Type\s*/Page\b").is_match(head))
        .map(|(head, _)| page_chunks(&objs, head))
        .collect()
}

fn round(v: f64) -> i64 {
    v.round_ties_even() as i64
}

fn sorted_pieces(mut v: Vec<(f64, String)>) -> Vec<(f64, String)> {
    v.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.cmp(&b.1))
    });
    v
}

fn squeeze(parts: &[String]) -> String {
    parts.join(" ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Deutschlandfunk's best-of lists: [{rank, title, author}] – each entry a column starting with "3 (5)" (place,
/// last month's place), then the title, then the author(s) indented beneath it.
pub fn best_list(raw: &[u8]) -> Result<Vec<Value>, String> {
    let rank_rx = crate::rx!(r"(?i)^\s*(\d{1,2})\s*\(\s*(?:\d{1,2}|-|–|neu)\s*\)\s*$");
    let stop_rx = crate::rx!(
        r"(?i)^(?:Übersetzt|Aus dem|Herausgegeben|Hrsg|Mit |Illustriert|Gelesen|\d+\s+Seiten|\d+[,.]?\d*\s+Euro)"
    );
    let mut out: Vec<(i64, String, String)> = Vec::new();
    for chunks in pages(raw)? {
        let mut lines: Vec<(i64, Vec<(f64, String)>)> = Vec::new();
        for (x, y, s) in &chunks {
            let ry = round(*y);
            match lines.iter_mut().find(|l| l.0 == ry) {
                Some(l) => l.1.push((*x, s.clone())),
                None => lines.push((ry, vec![(*x, s.clone())])),
            }
        }
        let mut anchors: Vec<(f64, i64, i64)> = Vec::new();
        for (y, cs) in lines {
            let mut groups: Vec<(f64, f64, Vec<String>)> = Vec::new();
            for (x, s) in sorted_pieces(cs) {
                match groups.last_mut() {
                    Some(g) if x - g.1 < 25.0 => {
                        g.1 = x;
                        g.2.push(s);
                    }
                    _ => groups.push((x, x, vec![s])),
                }
            }
            for (gx, _, parts) in groups {
                if let Some(m) = crate::rx::search(rank_rx, &parts.concat())
                    && let Ok(r) = m[1].parse::<i64>()
                {
                    anchors.push((gx, y, r));
                }
            }
        }
        for &(ax, ay, rank) in &anchors {
            let right = anchors
                .iter()
                .filter(|a| a.1 == ay && a.0 > ax)
                .map(|a| a.0)
                .fold(ax + 200.0, f64::min);
            let mut column: BTreeMap<i64, Vec<(f64, String)>> = BTreeMap::new();
            for (x, y, s) in &chunks {
                if ax - 3.0 <= *x && *x < right - 3.0 && (ay as f64) - 260.0 < *y && *y < (ay as f64) - 1.0 {
                    column.entry(round(*y)).or_default().push((*x, s.clone()));
                }
            }
            let (mut title, mut author): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
            for (_, cs) in column.into_iter().rev() {
                let cs = sorted_pieces(cs);
                let text: String = cs.iter().map(|c| c.1.as_str()).collect::<String>().trim().to_string();
                if text.is_empty() {
                    continue;
                }
                let indented = cs[0].0 >= ax + 40.0;
                if !indented && author.is_empty() {
                    if crate::rx::is_match(rank_rx, &text) {
                        break;
                    }
                    title.push(text);
                } else if indented && !title.is_empty() {
                    if crate::rx::is_match(stop_rx, &text) {
                        break;
                    }
                    author.push(text);
                } else if !author.is_empty() {
                    break;
                }
            }
            if !title.is_empty() && !author.is_empty() {
                out.push((rank, squeeze(&title), squeeze(&author)));
            }
        }
    }
    out.sort_by_key(|e| e.0);
    Ok(out
        .into_iter()
        .map(|(rank, title, author)| json!({"rank": rank, "title": title, "author": author}))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harmless_input() {
        assert!(best_list(b"<html>").is_err());
        let bomb = {
            use std::io::Write;
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            e.write_all(&vec![b'0'; MAX_STREAM * 3]).unwrap();
            e.finish().unwrap()
        };
        let mut raw =
            b"%PDF-1.7\n1 0 obj\n<</Type/Page/Contents 2 0 R>>\nendobj\n2 0 obj\n<</Filter/FlateDecode>>stream\n"
                .to_vec();
        raw.extend(bomb);
        raw.extend(b"\nendstream\nendobj\n3 0 obj\n<</Filter/FlateDecode>>stream\nnot zlib\nendstream\nendobj\n");
        assert_eq!(best_list(&raw).unwrap(), Vec::<Value>::new());
        let objs = objects(&raw);
        assert_eq!(objs[&2].1.as_ref().unwrap().len(), MAX_STREAM);
        assert!(objs[&3].1.as_ref().unwrap().is_empty());
    }
}
