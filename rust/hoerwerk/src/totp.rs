//! Second factor with an authenticator app (accounts.py): RFC 6238 codes (SHA-1, 6 digits, 30 s), one step of
//! clock drift either way, and no code accepted twice.

use hmac::{Hmac, Mac};
use rand::RngCore;
use sha1::Sha1;

use crate::app::App;
use crate::text::now;

const STEP: i64 = 30;
const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

pub fn base32(data: &[u8]) -> String {
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for b in data {
        buf = (buf << 8) | *b as u32;
        bits += 8;
        while bits >= 5 {
            out.push(ALPHABET[((buf >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn unbase32(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for c in text.trim_end_matches('=').chars() {
        let v = ALPHABET.iter().position(|a| *a as char == c.to_ascii_uppercase())? as u32;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            out.push(((buf >> (bits - 8)) & 0xff) as u8);
            bits -= 8;
        }
    }
    Some(out)
}

pub fn secret() -> String {
    let mut b = [0u8; 20];
    rand::rng().fill_bytes(&mut b);
    base32(&b)
}

pub fn code(secret: &str, step: i64) -> String {
    let key = unbase32(secret).unwrap_or_default();
    let mut mac = <Hmac<Sha1> as Mac>::new_from_slice(&key).expect("any key length");
    mac.update(&step.to_be_bytes());
    let d = mac.finalize().into_bytes();
    let off = (d[19] & 0x0f) as usize;
    let n = u32::from_be_bytes([d[off], d[off + 1], d[off + 2], d[off + 3]]) & 0x7fff_ffff;
    format!("{:06}", n % 1_000_000)
}

pub fn uri(secret: &str, name: &str) -> String {
    let q = |s: &str| percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string();
    format!(
        "otpauth://totp/{}?secret={secret}&issuer={}&algorithm=SHA1&digits=6&period=30",
        q(&format!("Hörwerk:{name}")).replace("%3A", ":"),
        q("Hörwerk")
    )
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A code for this secret – and never the same time step twice for one account (replay).
pub fn ok(app: &App, uid: i64, secret: &str, input: &str) -> bool {
    let input: String = input.chars().filter(char::is_ascii_digit).collect();
    if input.len() != 6 {
        return false;
    }
    let current = (now() as i64).div_euclid(STEP);
    for step in [current - 1, current, current + 1] {
        if same(&code(secret, step), &input) {
            let mut st = app.totp.lock().unwrap_or_else(|p| p.into_inner());
            if st.used.get(&uid).is_some_and(|u| *u >= step) {
                return false;
            }
            st.used.insert(uid, step);
            return true;
        }
    }
    false
}

pub fn stored_secret(app: &App, uid: i64) -> Option<String> {
    app.db
        .lock()
        .query_row("SELECT totp FROM users WHERE id=?", [uid], |r| {
            r.get::<_, Option<String>>(0)
        })
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
}

pub fn check(app: &App, uid: i64, input: &str) -> bool {
    stored_secret(app, uid).is_some_and(|s| ok(app, uid, &s, input))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_6238() {
        // RFC 6238 test secret "12345678901234567890", T = 59 s -> 94287082 (8 digits) -> 287082
        let s = base32(b"12345678901234567890");
        assert_eq!(s, "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
        assert_eq!(code(&s, 59 / 30), "287082");
        assert_eq!(code(&s, 1111111109 / 30), "081804");
        assert_eq!(unbase32("gezdgnbvgy3tqojq").unwrap(), b"1234567890");
        assert_eq!(secret().len(), 32);
    }
}
