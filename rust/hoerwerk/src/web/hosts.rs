//! Which Host names Hörwerk answers under – protection against DNS rebinding (same rules as app/web/hosts.py):
//! IPs and localhost always; ALLOWED_HOSTS exactly when set (``*.domain``, ``*`` = any); otherwise home-network
//! names plus the public address from the settings; through a trusted proxy any name the proxy forwards.

use std::net::IpAddr;

use crate::app::App;

const LOOPBACK: [&str; 3] = ["localhost", "127.0.0.1", "::1"];
const HOME_SUFFIXES: [&str; 8] = [
    ".local",
    ".lan",
    ".home.arpa",
    ".internal",
    ".localdomain",
    ".localhost",
    ".fritz.box",
    ".home",
];

/// "Tower:8080" -> "tower", "[::1]:8080" -> "::1".
pub fn host_name(header: &str) -> String {
    let host = header.trim().to_lowercase();
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("").to_string();
    }
    if host.matches(':').count() == 1 {
        host.rsplit_once(':').map(|x| x.0.to_string()).unwrap_or(host)
    } else {
        host
    }
}

fn is_ip(name: &str) -> bool {
    name.split('%').next().is_some_and(|n| n.parse::<IpAddr>().is_ok())
}

fn home_name(name: &str) -> bool {
    let n = name.trim_end_matches('.');
    !n.is_empty() && (!n.contains('.') || HOME_SUFFIXES.iter().any(|s| n.ends_with(s)))
}

pub fn allowed_with(
    header: &str,
    configured: &std::collections::HashSet<String>,
    public_host: &str,
    via_proxy: bool,
) -> bool {
    let name = host_name(header);
    let name = name.trim_end_matches('.');
    if name.is_empty() {
        return false;
    }
    if LOOPBACK.contains(&name) || is_ip(name) || configured.contains("*") || (via_proxy && configured.is_empty()) {
        return true;
    }
    if !configured.is_empty() {
        return configured.contains(name)
            || configured.iter().any(|p| {
                p.strip_prefix('*')
                    .is_some_and(|suffix| suffix.starts_with('.') && name.ends_with(suffix))
            });
    }
    home_name(name) || (!public_host.is_empty() && name == public_host.to_lowercase())
}

pub fn allowed(app: &App, header: &str, via_proxy: bool) -> bool {
    let public = app.public_host();
    allowed_with(header, &app.cfg.allowed_hosts, &public, via_proxy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn default_rules() {
        let none = HashSet::new();
        for ok in [
            "127.0.0.1:8080",
            "localhost:8080",
            "[::1]:8080",
            "192.168.1.10:8080",
            "tower:8080",
            "TOWER",
            "nas.local",
            "unraid.lan",
            "nas.fritz.box",
            "hoerwerk.home.arpa",
        ] {
            assert!(allowed_with(ok, &none, "", false), "{ok}");
        }
        for bad in [
            "evil.example",
            "192.168.1.10.nip.io",
            "hoerbuecher.example.de",
            "localhost.evil.example",
            "",
        ] {
            assert!(!allowed_with(bad, &none, "", false), "{bad}");
        }
        assert!(allowed_with(
            "hoerbuecher.example.de",
            &none,
            "hoerbuecher.example.de",
            false
        ));
        assert!(allowed_with("hoerbuecher.example.de", &none, "", true));
        let list: HashSet<String> = ["*.example.de".to_string()].into();
        assert!(allowed_with("hb.example.de", &list, "", false));
        assert!(!allowed_with("example.de.evil.example", &list, "", false));
        assert!(!allowed_with("hoerbuecher.other.de", &list, "", true)); // an explicit list is exact
    }
}
