//! HTTP GETs behind a trait, so LRCLIB and MusicBrainz lookups run
//! against a fake server in tests. Production uses `ureq` with rustls and
//! its bundled roots, so no system TLS library is needed on any platform.
//!
//! Every request names muman by [`user_agent`], in the form MusicBrainz
//! asks of every client, `name/version ( contact )`, with the repository
//! as the contact; a service that throttles anonymous clients can tell
//! muman apart and reach whoever runs it. A [`Throttle`] spaces the
//! requests to one service across every thread of a run, for a service
//! that allows so many a second from one address.

use std::sync::Mutex;
use std::time::{Duration, Instant};

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

/// What every request names muman as: `muman/<version> ( <repository> )`.
#[must_use]
pub fn user_agent() -> String {
    format!(
        "muman/{} ( {} )",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_REPOSITORY")
    )
}

/// Requests to one service spaced at least `gap` apart, whichever thread
/// makes them, each taking the next free turn; a refusal for going too
/// fast holds every later turn back.
#[derive(Debug)]
pub struct Throttle {
    gap: Duration,
    backoff: Duration,
    /// When the next request may go.
    next: Mutex<Option<Instant>>,
}

impl Throttle {
    /// Turns `gap` apart, held back by `backoff`, doubling, after each
    /// refusal in a row.
    #[must_use]
    pub fn new(gap: Duration, backoff: Duration) -> Self {
        Self {
            gap,
            backoff,
            next: Mutex::new(None),
        }
    }

    /// No spacing and no holding back, for tests.
    #[must_use]
    pub fn none() -> Self {
        Self::new(Duration::ZERO, Duration::ZERO)
    }

    /// Wait for this request's turn.
    pub fn wait(&self) {
        let wait = self.take(Instant::now());
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
    }

    /// Hold every later request back after the `times`th refusal in a row.
    pub fn refused(&self, times: u32) {
        self.hold(Instant::now(), times);
    }

    /// Take the next free turn at `now`; how long until it comes.
    fn take(&self, now: Instant) -> Duration {
        let mut next = self
            .next
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let turn = next.map_or(now, |n| n.max(now));
        *next = Some(turn + self.gap);
        turn - now
    }

    fn hold(&self, now: Instant, times: u32) {
        let pause = self
            .backoff
            .saturating_mul(1 << times.saturating_sub(1).min(16));
        let mut next = self
            .next
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *next = Some(next.map_or(now, |n| n.max(now)).max(now + pause));
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
    fn a_throttle_spaces_turns_across_callers_and_holds_back_after_a_refusal() {
        let t = Throttle::new(Duration::from_secs(1), Duration::from_secs(2));
        let start = Instant::now();
        assert_eq!(t.take(start), Duration::ZERO);
        assert_eq!(t.take(start), Duration::from_secs(1));
        assert_eq!(t.take(start), Duration::from_secs(2));
        let later = start + Duration::from_secs(10);
        assert_eq!(t.take(later), Duration::ZERO, "an idle gap is not saved up");
        t.hold(later, 2);
        assert_eq!(t.take(later), Duration::from_secs(4));
        assert_eq!(t.take(later), Duration::from_secs(5));
    }

    #[test]
    fn the_user_agent_names_muman_and_a_contact() {
        let agent = user_agent();
        assert!(agent.starts_with("muman/"), "{agent}");
        assert!(
            agent.ends_with(" ( https://github.com/jeggao/muman )"),
            "{agent}"
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
