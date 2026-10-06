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
//!
//! A [`Service`] asks through its throttle: a 429 or a 503 is a refusal
//! for going too fast, which holds every later request back, for as long
//! as the server's `Retry-After` says when it says, and is asked again up
//! to [`RETRIES`] times. A service still refusing after that is
//! [`Refusing`], so a run puts its other lookups off to the next run
//! rather than record each as failed.

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
        /// How long its `Retry-After` asks a client to wait.
        retry_after: Option<Duration>,
    },
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => f.write_str("timed out"),
            Self::Transport(e) => write!(f, "transport: {e}"),
            Self::Status { code, body, .. } => write!(f, "status {code}: {body}"),
        }
    }
}

impl std::error::Error for TransportError {}

pub trait HttpTransport {
    /// GET `url` and return the response body.
    fn get(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<Vec<u8>, TransportError>;

    /// GET `url` and return the response body as text.
    fn get_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<String, TransportError> {
        String::from_utf8(self.get(url, headers, timeout)?)
            .map_err(|e| TransportError::Transport(e.to_string()))
    }
}

/// The agent is built per call so each request's timeout is exact.
#[derive(Debug, Default, Clone, Copy)]
pub struct UreqTransport;

impl HttpTransport for UreqTransport {
    fn get(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<Vec<u8>, TransportError> {
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
        if !(200..300).contains(&code) {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            return Err(TransportError::Status {
                code,
                body: response.body_mut().read_to_string().unwrap_or_default(),
                retry_after,
            });
        }
        response
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_vec()
            .map_err(|e| TransportError::Transport(e.to_string()))
    }
}

/// The largest body read: a cover at full size is a few MiB.
const MAX_BODY: u64 = 64 << 20;

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

    /// Hold every later request back after the `times`th refusal in a row,
    /// at least as long as the server asked.
    pub fn refused(&self, times: u32, asked: Option<Duration>) {
        self.hold(Instant::now(), times, asked);
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

    fn hold(&self, now: Instant, times: u32, asked: Option<Duration>) {
        let pause = self
            .backoff
            .saturating_mul(1 << times.saturating_sub(1).min(16))
            .max(asked.unwrap_or_default().min(MAX_ASKED));
        let mut next = self
            .next
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *next = Some(next.map_or(now, |n| n.max(now)).max(now + pause));
    }
}

/// The longest a server's `Retry-After` holds a run back; one asking for
/// longer is refusing for this run.
const MAX_ASKED: Duration = Duration::from_secs(120);

/// Times a request refused for going too fast is asked again.
pub const RETRIES: u32 = 3;

/// A service still refusing requests for going too fast after
/// [`RETRIES`] tries.
#[derive(Debug)]
pub struct Refusing {
    pub service: &'static str,
}

impl std::fmt::Display for Refusing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} refuses requests for going too fast", self.service)
    }
}

impl std::error::Error for Refusing {}

/// One service, asked through `transport` at the pace of `throttle`.
#[derive(Clone, Copy)]
pub struct Service<'a> {
    pub name: &'static str,
    pub transport: &'a (dyn HttpTransport + Sync),
    pub throttle: &'a Throttle,
}

impl std::fmt::Debug for Service<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Service<'_> {
    /// GET `url` on its turn, asking again after a refusal; `None` for a
    /// 404, and [`Refusing`] when the refusals outlast the retries.
    pub fn get(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        timeout: Duration,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        let mut refused = 0;
        loop {
            self.throttle.wait();
            match self.transport.get(url, headers, timeout) {
                Ok(body) => return Ok(Some(body)),
                Err(TransportError::Status { code: 404, .. }) => return Ok(None),
                Err(TransportError::Status {
                    code: 429 | 503,
                    retry_after,
                    ..
                }) => {
                    if refused == RETRIES || retry_after.is_some_and(|a| a > MAX_ASKED) {
                        return Err(Refusing { service: self.name }.into());
                    }
                    refused += 1;
                    self.throttle.refused(refused, retry_after);
                }
                Err(e) => return Err(anyhow::anyhow!("{url}: {e}")),
            }
        }
    }

    /// [`Self::get`] as text.
    pub fn get_text(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        timeout: Duration,
    ) -> anyhow::Result<Option<String>> {
        self.get(url, headers, timeout)?
            .map(|b| String::from_utf8(b).map_err(|e| anyhow::anyhow!("{url}: {e}")))
            .transpose()
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
        t.hold(later, 2, None);
        assert_eq!(t.take(later), Duration::from_secs(4));
        assert_eq!(t.take(later), Duration::from_secs(5));
        let after = later + Duration::from_secs(60);
        t.hold(after, 1, Some(Duration::from_secs(30)));
        assert_eq!(
            t.take(after),
            Duration::from_secs(30),
            "the server's wait is kept"
        );
    }

    /// Answers each request with the next of `script`, then 200.
    struct Scripted {
        script: Mutex<Vec<u16>>,
        retry_after: Option<Duration>,
    }

    impl HttpTransport for Scripted {
        fn get(&self, _: &str, _: &[(&str, &str)], _: Duration) -> Result<Vec<u8>, TransportError> {
            let mut script = self.script.lock().unwrap();
            if script.is_empty() {
                return Ok(b"ok".to_vec());
            }
            Err(TransportError::Status {
                code: script.remove(0),
                body: "error code: 1015".into(),
                retry_after: self.retry_after,
            })
        }
    }

    fn ask(script: &[u16], retry_after: Option<Duration>) -> anyhow::Result<Option<String>> {
        let transport = Scripted {
            script: Mutex::new(script.to_vec()),
            retry_after,
        };
        let throttle = Throttle::none();
        Service {
            name: "Lumo Lyrics",
            transport: &transport,
            throttle: &throttle,
        }
        .get_text("http://lyrics.test/a", &[], Duration::from_secs(1))
    }

    #[test]
    fn a_refusal_is_asked_again_until_the_retries_run_out() {
        assert_eq!(ask(&[429, 503, 429], None).unwrap().as_deref(), Some("ok"));
        let e = ask(&[429; 4], None).unwrap_err();
        assert!(e.downcast_ref::<Refusing>().is_some(), "{e:#}");
        assert_eq!(
            e.to_string(),
            "Lumo Lyrics refuses requests for going too fast"
        );
        assert_eq!(ask(&[404], None).unwrap(), None);
        assert!(
            ask(&[500], None)
                .unwrap_err()
                .downcast_ref::<Refusing>()
                .is_none()
        );
    }

    #[test]
    fn a_server_asking_for_too_long_a_wait_is_refusing_at_once() {
        let e = ask(&[429], Some(MAX_ASKED + Duration::from_secs(1))).unwrap_err();
        assert!(e.downcast_ref::<Refusing>().is_some(), "{e:#}");
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
