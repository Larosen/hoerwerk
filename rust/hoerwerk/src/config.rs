//! Environment of the container – the same variables as the Python version (README, contract/settings.json).

use std::collections::{BTreeMap, HashSet};
use std::env;
use std::net::IpAddr;
use std::path::PathBuf;

/// A network (IP or CIDR) whose X-Forwarded-For is believed.
#[derive(Debug, Clone, PartialEq)]
pub struct Net {
    pub addr: IpAddr,
    pub prefix: u8,
}

impl Net {
    pub fn contains(&self, ip: &IpAddr) -> bool {
        match (self.addr, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = if self.prefix == 0 {
                    0
                } else {
                    u32::MAX << (32 - u32::from(self.prefix))
                };
                u32::from(net) & mask == u32::from(*ip) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = if self.prefix == 0 {
                    0
                } else {
                    u128::MAX << (128 - u32::from(self.prefix))
                };
                u128::from(net) & mask == u128::from(*ip) & mask
            }
            _ => false,
        }
    }
}

pub fn parse_networks(text: &str) -> Result<Vec<Net>, String> {
    let mut out = vec![];
    for raw in text.split([',', ';', ' ']).map(str::trim).filter(|s| !s.is_empty()) {
        let (addr, prefix) = match raw.split_once('/') {
            Some((a, p)) => (a, Some(p)),
            None => (raw, None),
        };
        let ip: IpAddr = addr
            .parse()
            .map_err(|_| format!("TRUSTED_PROXIES: „{raw}“ ist keine IP-Adresse oder kein Netz"))?;
        let max = if ip.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) => p
                .parse::<u8>()
                .ok()
                .filter(|p| *p <= max)
                .ok_or_else(|| format!("TRUSTED_PROXIES: „{raw}“"))?,
            None => max,
        };
        out.push(Net { addr: ip, prefix });
    }
    Ok(out)
}

/// ALLOWED_HOSTS: names without scheme, path or port; a port is tolerated and dropped; anything else stops the start.
pub fn parse_hosts(text: &str) -> Result<HashSet<String>, String> {
    let rx = regex_lite::Regex::new(
        r"^(?:\*|(?:\*\.)?[a-z0-9_](?:[a-z0-9_-]{0,62})(?:\.[a-z0-9_](?:[a-z0-9_-]{0,62}))*\.?|\[?[0-9a-f:.]+\]?)$",
    )
    .expect("valid pattern");
    let mut out = HashSet::new();
    for raw in text.replace(';', ",").split(',') {
        let mut name = raw.trim().to_lowercase();
        if name.is_empty() {
            continue;
        }
        if !name.starts_with('[')
            && name.matches(':').count() == 1
            && let Some((host, port)) = name.split_once(':')
            && port.chars().all(|c| c.is_ascii_digit())
        {
            name = host.to_string();
        }
        if name != "*" && !name.starts_with('*') {
            name = name.trim_end_matches('.').trim_matches(['[', ']']).to_string();
        }
        if name.contains('/') || name.contains('@') || name.contains(' ') || !rx.is_match(&name) {
            return Err(format!(
                "ALLOWED_HOSTS: „{}“ ist kein Hostname – nur Namen wie tower, hoerbuecher.example.de, *.example.de oder 192.168.1.10 (ohne https:// und ohne Pfad)",
                raw.trim()
            ));
        }
        out.insert(name);
    }
    Ok(out)
}

static COMPARE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// COMPARE_MODE=true: a second instance next to the live one (a copy of /config, see docs/VERGLEICH.md) – it
/// starts no downloads, searches no requests on its own, sends no notifications and asks no media server to scan.
pub fn compare_mode() -> bool {
    COMPARE.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_compare_mode(on: bool) {
    COMPARE.store(on, std::sync::atomic::Ordering::Relaxed);
}

#[derive(Debug, Clone)]
pub struct Config {
    pub config_dir: PathBuf,
    pub inbox: PathBuf,
    pub library: PathBuf,
    pub work_dir: PathBuf,
    pub static_dir: PathBuf,
    pub port: u16,
    pub auth_user: String,
    pub auth_pass: String,
    pub allowed_hosts: HashSet<String>,
    pub trusted_proxies: Vec<Net>,
    pub cookie_secure: String,
    /// ffmpeg threads per conversion, CPU priority, steps at the same time per queue
    pub cpu_jobs: usize,
    pub nice: String,
    pub encode_workers: usize,
    pub io_workers: usize,
    pub env: BTreeMap<String, String>,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Self::from_map(env::vars().collect())
    }

    pub fn from_map(env: BTreeMap<String, String>) -> Result<Self, String> {
        let get = |k: &str, d: &str| {
            env.get(k)
                .cloned()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| d.to_string())
        };
        let inbox = PathBuf::from(get("INBOX_DIR", "/inbox"));
        let static_dir = env.get("STATIC_DIR").map(PathBuf::from).unwrap_or_else(|| {
            ["/app/static", "app/static", "../app/static"]
                .iter()
                .map(PathBuf::from)
                .find(|p| p.join("index.html").exists())
                .unwrap_or_else(|| PathBuf::from("/app/static"))
        });
        Ok(Config {
            config_dir: PathBuf::from(get("CONFIG_DIR", "/config")),
            work_dir: env
                .get("WORK_DIR")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| inbox.join(".work")),
            library: PathBuf::from(get("LIBRARY_DIR", "/audiobooks")),
            inbox,
            static_dir,
            port: get("PORT", "8080")
                .parse()
                .map_err(|_| "PORT: keine Zahl".to_string())?,
            auth_user: env.get("AUTH_USER").cloned().unwrap_or_default(),
            auth_pass: env.get("AUTH_PASS").cloned().unwrap_or_default(),
            allowed_hosts: parse_hosts(env.get("ALLOWED_HOSTS").map(String::as_str).unwrap_or(""))?,
            trusted_proxies: parse_networks(env.get("TRUSTED_PROXIES").map(String::as_str).unwrap_or(""))?,
            cookie_secure: get("COOKIE_SECURE", "auto").trim().to_lowercase(),
            cpu_jobs: match env.get("CPU_CORES").filter(|v| !v.is_empty()) {
                Some(v) => v.trim().parse().map_err(|_| "CPU_CORES: keine Zahl".to_string())?,
                None => (std::thread::available_parallelism().map_or(2, |n| n.get()) / 2).max(1),
            },
            nice: env.get("NICE").cloned().unwrap_or_else(|| "10".into()),
            encode_workers: get("ENCODE_WORKERS", "1")
                .trim()
                .parse::<usize>()
                .map_err(|_| "ENCODE_WORKERS: keine Zahl".to_string())?
                .max(1),
            io_workers: get("IO_WORKERS", "2")
                .trim()
                .parse::<usize>()
                .map_err(|_| "IO_WORKERS: keine Zahl".to_string())?
                .max(1),
            env,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_like_python() {
        let h = parse_hosts("Tower, hoerbuecher.example.de:443 ,*.example.de,192.168.1.10,[::1],*").unwrap();
        for want in [
            "tower",
            "hoerbuecher.example.de",
            "*.example.de",
            "192.168.1.10",
            "::1",
            "*",
        ] {
            assert!(h.contains(want), "{want}");
        }
        for bad in ["https://x.de", "x.de/path", "a b", "user@x"] {
            assert!(parse_hosts(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn networks() {
        let n = parse_networks("172.18.0.0/16, 10.0.0.5").unwrap();
        assert!(n[0].contains(&"172.18.4.2".parse().unwrap()));
        assert!(!n[0].contains(&"172.19.0.1".parse().unwrap()));
        assert!(n[1].contains(&"10.0.0.5".parse().unwrap()));
        assert!(parse_networks("kein-netz").is_err());
    }
}
