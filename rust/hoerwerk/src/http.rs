//! Outgoing HTTP (ntfy, Audible, indexers, SABnzbd, Audiobookshelf, Plex): blocking – call it off the async threads.
//! System certificates like Python's urllib; credentials never follow a redirect (`follow` = false).

use std::time::Duration;

use serde_json::Value;

pub const USER_AGENT: &str = "Hoerwerk/7";

pub struct Resp {
    pub status: u16,
    pub body: Vec<u8>,
    pub content_type: String,
    pub retry_after: Option<String>,
}

impl Resp {
    pub fn json(&self) -> Option<Value> {
        serde_json::from_slice(&self.body).ok()
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

pub struct Call<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub headers: Vec<(String, String)>,
    pub body: Option<(String, Vec<u8>)>,
    pub timeout: Duration,
    pub follow: bool,
    pub limit: u64,
}

impl<'a> Call<'a> {
    pub fn new(method: &'a str, url: &'a str) -> Self {
        Call {
            method,
            url,
            headers: Vec::new(),
            body: None,
            timeout: Duration::from_secs(20),
            follow: true,
            limit: 8 << 20,
        }
    }

    pub fn header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.to_string(), v.to_string()));
        self
    }

    pub fn json(mut self, v: &Value) -> Self {
        self.body = Some(("application/json".into(), serde_json::to_vec(v).unwrap_or_default()));
        self
    }

    pub fn body(mut self, ctype: &str, data: Vec<u8>) -> Self {
        self.body = Some((ctype.to_string(), data));
        self
    }

    pub fn timeout(mut self, secs: u64) -> Self {
        self.timeout = Duration::from_secs(secs);
        self
    }

    pub fn no_redirect(mut self) -> Self {
        self.follow = false;
        self
    }

    pub fn limit(mut self, bytes: u64) -> Self {
        self.limit = bytes;
        self
    }

    /// Err = no answer at all (name, connection, TLS, timeout); every HTTP status is an Ok answer.
    pub fn send(self) -> Result<Resp, String> {
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .max_redirects(if self.follow { 5 } else { 0 })
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .tls_config(tls)
            .build()
            .into();
        let mut builder = ureq::http::Request::builder().method(self.method).uri(self.url);
        for (k, v) in &self.headers {
            builder = builder.header(k, v);
        }
        let (ctype, data) = self.body.unwrap_or_default();
        if !ctype.is_empty() {
            builder = builder.header("Content-Type", ctype);
        }
        let request = builder.body(data).map_err(|e| e.to_string())?;
        let mut resp = agent.run(request).map_err(|e| reason(&e))?;
        let status = resp.status().as_u16();
        let header = |k: &str| resp.headers().get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
        let content_type = header("content-type").unwrap_or_default();
        let retry_after = header("retry-after");
        let body = resp
            .body_mut()
            .with_config()
            .limit(self.limit)
            .read_to_vec()
            .map_err(|e| reason(&e))?;
        Ok(Resp {
            status,
            body,
            content_type,
            retry_after,
        })
    }
}

fn reason(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Timeout(_) => "timed out".into(),
        ureq::Error::HostNotFound => "Name or service not known".into(),
        ureq::Error::Io(io) => io.to_string(),
        other => other.to_string(),
    }
}

pub fn get(url: &str) -> Call<'_> {
    Call::new("GET", url)
}

pub fn post(url: &str) -> Call<'_> {
    Call::new("POST", url)
}

/// scheme://host:port of an address – credentials only stay with the same server.
pub fn origin(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", ""));
    let hostport = rest.split(['/', '?', '#']).next().unwrap_or("");
    let hostport = hostport.rsplit_once('@').map_or(hostport, |x| x.1);
    let (host, port) = match hostport.strip_prefix('[') {
        Some(v6) => {
            let (h, p) = v6.split_once(']').unwrap_or((v6, ""));
            (h.to_string(), p.trim_start_matches(':').to_string())
        }
        None => match hostport.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.to_string()),
            None => (hostport.to_string(), String::new()),
        },
    };
    format!("{}://{}:{}", scheme.to_lowercase(), host.to_lowercase(), port)
}

/// http(s) with a host name and – if given – a numeric port; no user:password@, no whitespace (notify._valid_url).
pub fn valid_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    if !matches!(scheme, "http" | "https") || url.chars().any(char::is_whitespace) {
        return false;
    }
    let netloc = rest.split(['/', '?', '#']).next().unwrap_or("");
    if netloc.contains('@') {
        return false;
    }
    let (host, port) = match netloc.strip_prefix('[') {
        Some(v6) => match v6.split_once(']') {
            Some((h, p)) => (h, p.strip_prefix(':')),
            None => return false,
        },
        None => match netloc.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (netloc, None),
        },
    };
    !host.is_empty()
        && port.is_none_or(|p| {
            p.is_empty()
                || (p.len() <= 5 && p.chars().all(|c| c.is_ascii_digit()) && p.parse::<u32>().is_ok_and(|n| n <= 65535))
        })
}

/// url-encode a query value
pub fn quote(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(origin("https://NTFY.example:8443/x"), "https://ntfy.example:8443");
        assert_eq!(origin("https://ntfy.sh"), "https://ntfy.sh:");
        assert!(valid_url("https://ntfy.sh") && valid_url("http://10.0.0.2:8080/x"));
        for bad in [
            "ftp://x",
            "https://u:p@x",
            "https://x:alert(1)",
            "https://",
            "https://a b",
            "x",
        ] {
            assert!(!valid_url(bad), "{bad}");
        }
    }
}
