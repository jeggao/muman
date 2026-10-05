//! HTTP GETs behind a trait, so LRCLIB lookups run against a fake server
//! in tests. Production uses `ureq` with rustls and its bundled roots, so
//! no system TLS library is needed on any platform.

use std::time::Duration;

#[derive(Debug)]
pub enum TransportError {
    Timeout,
    /// DNS, connection refused, TLS and the like.
    Transport(String),
    /// The server answered with a status outside 2xx.
    Status {
        code: u16,
        body: String,
    },
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => f.write_str("timed out"),
            Self::Transport(e) => write!(f, "transport: {e}"),
            Self::Status { code, body } => write!(f, "status {code}: {body}"),
        }
    }
}

impl std::error::Error for TransportError {}

pub trait HttpTransport {
    /// GET `url` and return the response body as text.
    fn get_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<String, TransportError>;
}

/// The agent is built per call so each request's timeout is exact.
#[derive(Debug, Default, Clone, Copy)]
pub struct UreqTransport;

impl HttpTransport for UreqTransport {
    fn get_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<String, TransportError> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .build()
            .into();
        let mut request = agent.get(url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let mut response = match request.call() {
            Ok(r) => r,
            Err(ureq::Error::Timeout(_)) => return Err(TransportError::Timeout),
            Err(e) => return Err(TransportError::Transport(e.to_string())),
        };
        let code = response.status().as_u16();
        let body = response.body_mut().read_to_string();
        if !(200..300).contains(&code) {
            return Err(TransportError::Status {
                code,
                body: body.unwrap_or_default(),
            });
        }
        body.map_err(|e| TransportError::Transport(e.to_string()))
    }
}

/// `url` as an `http(s)://` base without trailing slashes; a bare
/// `host:port` gets `http://`. Any other scheme is refused, so a song
/// list cannot turn a lookup into a local file read.
pub fn normalize_base(url: &str) -> Result<String, String> {
    let url = url.trim();
    if let Some((scheme, authority)) = url.split_once("://") {
        let lower = scheme.to_ascii_lowercase();
        if lower != "http" && lower != "https" {
            return Err(format!("`{scheme}` is not http or https"));
        }
        let authority = authority.trim_end_matches('/');
        if authority.is_empty() {
            return Err(format!("`{url}` names no server"));
        }
        Ok(format!("{scheme}://{authority}"))
    } else if url.is_empty() {
        Err("the address is empty".into())
    } else {
        Ok(format!("http://{}", url.trim_end_matches('/')))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_base_keeps_http_and_https_and_drops_trailing_slashes() {
        assert_eq!(
            normalize_base("https://lrclib.example/").unwrap(),
            "https://lrclib.example"
        );
        assert_eq!(
            normalize_base("127.0.0.1:8080/").unwrap(),
            "http://127.0.0.1:8080"
        );
    }

    #[test]
    fn a_base_refuses_other_schemes_and_empty_servers() {
        assert!(normalize_base("file:///etc/passwd").is_err());
        assert!(normalize_base("FTP://host").is_err());
        assert!(normalize_base("https://").is_err());
        assert!(normalize_base("  ").is_err());
    }
}
