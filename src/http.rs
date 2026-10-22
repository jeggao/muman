//! HTTP GETs behind a trait, so LRCLIB and MusicBrainz lookups run
//! against a fake server in tests. Production uses `ureq` with rustls,
//! so no system TLS library is needed on any platform, and verifies a
//! server as the platform's own programs do, by the system's certificate
//! store, so an authority the machine trusts, a proxy's among them, is
//! trusted here too.
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
//! to [`RETRIES`] times; a request given its turn before the refusal,
//! still waiting for it, waits again once it comes. A service still
//! refusing after that is [`Refusing`], so a run puts its other lookups
//! off to the next run rather than record each as failed. `Retry-After`
//! is read in seconds or as the HTTP date RFC 9110 has servers send.
//!
//! A request that reaches no server, its name not resolving, its
//! connection refused or not open within [`CONNECT`], or a proxy failing
//! to connect it, is [`Unreachable`]: the network is down, or the service
//! is, and every other request would wait out its own timeout to fail
//! alike, so it too puts the run's other lookups off. A server that
//! answers, with an error, a body that does not read, or nothing before
//! the timeout once connected, fails that request alone, so one request
//! a service cannot answer never holds the others back run after run.
//! Either way the throttle stops: a later request to the service in the
//! run, one already waiting its turn too, fails alike without being sent.
//!
//! An error's body is kept only as far as `ERROR_EXCERPT`, whitespace
//! run together: a failure is recorded and printed with it, and a
//! server's error page runs to megabytes.
//!
//! An error names its request by its address without the query
//! ([`shown`]), and the query is cut from what the transport or the
//! server said too: AcoustID's holds the application key, a key of the
//! user's own when `[providers.acoustid] key` sets one, and the song's
//! whole fingerprint, and an error is printed and recorded in the state.

use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub enum TransportError {
    /// No answer in time from a server connected to.
    Timeout,
    /// No server reached: a name that does not resolve, a connection
    /// refused or not open in time, a proxy that would not connect.
    Unreachable(String),
    /// TLS, a connection cut, a body that does not read and the like.
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
            Self::Unreachable(e) => write!(f, "unreachable: {e}"),
            Self::Transport(e) => write!(f, "transport: {e}"),
            Self::Status { code, body, .. } => write!(f, "status {code}: {body}"),
        }
    }
}

impl std::error::Error for TransportError {}

impl TransportError {
    /// The error of a request of `url`, the query left out of what it
    /// says.
    #[must_use]
    pub fn hiding(self, url: &str) -> Self {
        match self {
            Self::Unreachable(e) => Self::Unreachable(without_query(url, &e)),
            Self::Transport(e) => Self::Transport(without_query(url, &e)),
            Self::Status {
                code,
                body,
                retry_after,
            } => Self::Status {
                code,
                body: without_query(url, &body),
                retry_after,
            },
            Self::Timeout => Self::Timeout,
        }
    }
}

/// `url` without its query and fragment: what an error names a request
/// by.
#[must_use]
pub fn shown(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

/// `text` with the query of `url` left out wherever it appears.
fn without_query(url: &str, text: &str) -> String {
    let query = &url[shown(url).len()..];
    if query.is_empty() {
        text.to_string()
    } else {
        text.replace(query, "")
    }
}

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
        text(self.get(url, headers, timeout)?).map_err(TransportError::Transport)
    }
}

/// A body as text, or why it is none: one compressed though no
/// compression was asked for is told apart.
fn text(body: Vec<u8>) -> Result<String, String> {
    if body.starts_with(&[0x1f, 0x8b]) {
        return Err("the body is gzip-compressed, which muman does not ask for".into());
    }
    String::from_utf8(body).map_err(|e| format!("the body is not UTF-8 text: {e}"))
}

/// How many characters of an error's body are kept.
const ERROR_EXCERPT: usize = 200;

/// The start of an error's body, whitespace run together.
fn excerpt(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let mut words = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some((at, _)) = words.char_indices().nth(ERROR_EXCERPT) {
        words.truncate(at);
        words.push('…');
    }
    words
}

/// How long a `Retry-After` asks to wait from `now`: its seconds, or
/// until its date.
fn retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    if let Ok(secs) = value.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    Some(http_date(value)?.duration_since(now).unwrap_or_default())
}

/// An HTTP date in the one form RFC 9110 has servers send,
/// `Sun, 06 Nov 1994 08:49:37 GMT`.
fn http_date(s: &str) -> Option<SystemTime> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let (_, rest) = s.split_once(", ")?;
    let [day, month, year, time, "GMT"] = rest.split(' ').collect::<Vec<_>>()[..] else {
        return None;
    };
    let month = i64::try_from(MONTHS.iter().position(|m| *m == month)? + 1).ok()?;
    let (day, year): (i64, i64) = (day.parse().ok()?, year.parse().ok()?);
    let hms: Vec<u64> = time
        .split(':')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    let [h, m, sec] = hms[..] else {
        return None;
    };
    if !(1..=31).contains(&day) || h > 23 || m > 59 || sec > 60 {
        return None;
    }
    // Days since 1970-01-01, by the proleptic Gregorian calendar's
    // 400-year eras of 146,097 days, the year taken to begin in March.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = u64::try_from(era * 146_097 + day_of_era - 719_468).ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(days * 86_400 + h * 3600 + m * 60 + sec))
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
        let connect = CONNECT.min(timeout / 2);
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .timeout_global(Some(timeout))
            .timeout_resolve(Some(connect))
            .timeout_connect(Some(connect))
            .http_status_as_error(false)
            .build()
            .into();
        let mut request = agent.get(url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let mut response = request.call().map_err(failure)?;
        let code = response.status().as_u16();
        if !(200..300).contains(&code) {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| retry_after(v, SystemTime::now()));
            let body = response
                .body_mut()
                .with_config()
                .limit(ERROR_READ)
                .read_to_vec()
                .unwrap_or_default();
            return Err(TransportError::Status {
                code,
                body: excerpt(&body),
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

/// The longest a connection may take to open, a proxy's CONNECT and the
/// TLS handshake included, before the server counts as unreachable: on a
/// working network it opens in well under a second.
pub const CONNECT: Duration = Duration::from_secs(10);

/// A call that failed, told by whether it reached a server: the phases
/// before the request is sent each time out on their own, at most
/// [`CONNECT`], so a timeout names its phase.
fn failure(e: ureq::Error) -> TransportError {
    use std::io::ErrorKind;
    match e {
        ureq::Error::Timeout(ureq::Timeout::Resolve | ureq::Timeout::Connect) => {
            TransportError::Unreachable("the connection did not open in time".into())
        }
        ureq::Error::Timeout(_) => TransportError::Timeout,
        ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        | ureq::Error::ConnectProxyFailed(_)
        | ureq::Error::InvalidProxyUrl => TransportError::Unreachable(e.to_string()),
        // A name that does not resolve fails as an I/O error of no kind
        // of its own; a connection made and then cut has one of these.
        ureq::Error::Io(ref io)
            if !matches!(
                io.kind(),
                ErrorKind::ConnectionReset
                    | ErrorKind::ConnectionAborted
                    | ErrorKind::BrokenPipe
                    | ErrorKind::UnexpectedEof
            ) =>
        {
            TransportError::Unreachable(e.to_string())
        }
        e => TransportError::Transport(e.to_string()),
    }
}

/// The largest body read: a cover at full size is a few MiB.
const MAX_BODY: u64 = 64 << 20;

/// The most of an error's body read, for its [`excerpt`].
const ERROR_READ: u64 = 64 << 10;

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
/// fast holds every later turn back, one taken already too, and a
/// service refusing or unreachable for the run stops every request.
#[derive(Debug)]
pub struct Throttle {
    gap: Duration,
    backoff: Duration,
    turns: Mutex<Turns>,
}

#[derive(Debug, Default)]
struct Turns {
    /// When the next request may go.
    next: Option<Instant>,
    /// Until when the last refusal holds requests back.
    held: Option<Instant>,
    /// Why the service is asked nothing more this run.
    stopped: Option<Stop>,
}

/// Why a run asks a service nothing more.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Stop {
    Refusing,
    Unreachable(String),
}

impl Throttle {
    /// Turns `gap` apart, held back by `backoff`, doubling, after each
    /// refusal in a row.
    #[must_use]
    pub fn new(gap: Duration, backoff: Duration) -> Self {
        Self {
            gap,
            backoff,
            turns: Mutex::new(Turns::default()),
        }
    }

    /// No spacing and no holding back, for tests.
    #[must_use]
    pub fn none() -> Self {
        Self::new(Duration::ZERO, Duration::ZERO)
    }

    fn turns(&self) -> std::sync::MutexGuard<'_, Turns> {
        self.turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Wait for this request's turn, and again for a later one when a
    /// refusal came while it waited; why the service is asked nothing
    /// when it stopped.
    fn wait(&self) -> Result<(), Stop> {
        loop {
            let wait = self.take(Instant::now())?;
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
            if self.ready(Instant::now())? {
                return Ok(());
            }
        }
    }

    /// Hold every later request back after the `times`th refusal in a row,
    /// at least as long as the server asked.
    fn refused(&self, times: u32, asked: Option<Duration>) {
        self.hold(Instant::now(), times, asked);
    }

    /// Ask the service nothing more this run; the first reason stays.
    fn stop(&self, why: Stop) {
        self.turns().stopped.get_or_insert(why);
    }

    /// Take the next free turn at `now`; how long until it comes.
    fn take(&self, now: Instant) -> Result<Duration, Stop> {
        let mut turns = self.turns();
        if let Some(why) = &turns.stopped {
            return Err(why.clone());
        }
        let turn = turns.next.map_or(now, |n| n.max(now));
        turns.next = Some(turn + self.gap);
        Ok(turn - now)
    }

    /// Whether a request whose turn has come at `now` may go: no refusal
    /// since holds it back, and the service is not stopped.
    fn ready(&self, now: Instant) -> Result<bool, Stop> {
        let turns = self.turns();
        if let Some(why) = &turns.stopped {
            return Err(why.clone());
        }
        Ok(turns.held.is_none_or(|h| h <= now))
    }

    fn hold(&self, now: Instant, times: u32, asked: Option<Duration>) {
        let pause = self
            .backoff
            .saturating_mul(1 << times.saturating_sub(1).min(16))
            .max(asked.unwrap_or_default().min(MAX_ASKED));
        let until = now + pause;
        let mut turns = self.turns();
        turns.held = Some(turns.held.map_or(until, |h| h.max(until)));
        turns.next = Some(turns.next.map_or(now, |n| n.max(now)).max(until));
    }
}

/// The longest a server's `Retry-After` holds a run back; one asking for
/// longer is refusing for this run.
const MAX_ASKED: Duration = Duration::from_secs(120);

/// Times a request refused for going too fast is asked again.
pub const RETRIES: u32 = 3;

/// A service still refusing requests for going too fast after
/// [`RETRIES`] tries.
#[derive(Debug, Clone)]
pub struct Refusing {
    pub service: &'static str,
}

impl std::fmt::Display for Refusing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} refuses requests for going too fast", self.service)
    }
}

impl std::error::Error for Refusing {}

/// A service no request reaches, and why.
#[derive(Debug, Clone)]
pub struct Unreachable {
    pub service: &'static str,
    pub why: String,
}

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} cannot be reached ({})", self.service, self.why)
    }
}

impl std::error::Error for Unreachable {}

/// A copy of `e` when the service failed rather than the request, as
/// [`Refusing`] or [`Unreachable`]: the run asks that service nothing
/// more, and records none of what it put off as failed.
#[must_use]
pub fn put_off(e: &anyhow::Error) -> Option<anyhow::Error> {
    if let Some(r) = e.downcast_ref::<Refusing>() {
        return Some(r.clone().into());
    }
    e.downcast_ref::<Unreachable>().map(|u| u.clone().into())
}

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
    /// 404, [`Refusing`] when the refusals outlast the retries, and
    /// [`Unreachable`] when no server is reached. Either of those stops
    /// the throttle, so a request after it fails alike unsent. Any other
    /// failure is the [`TransportError`], in the context of `url`.
    pub fn get(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        timeout: Duration,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        let mut refused = 0;
        loop {
            self.throttle.wait().map_err(|why| self.error(why))?;
            match self.transport.get(url, headers, timeout) {
                Ok(body) => return Ok(Some(body)),
                Err(TransportError::Status { code: 404, .. }) => return Ok(None),
                Err(TransportError::Status {
                    code: 429 | 503,
                    retry_after,
                    ..
                }) => {
                    if refused == RETRIES || retry_after.is_some_and(|a| a > MAX_ASKED) {
                        return Err(self.stop(Stop::Refusing));
                    }
                    refused += 1;
                    self.throttle.refused(refused, retry_after);
                }
                Err(TransportError::Unreachable(why)) => {
                    return Err(self.stop(Stop::Unreachable(without_query(url, &why))));
                }
                Err(e) => {
                    return Err(anyhow::Error::new(e.hiding(url)).context(shown(url).to_string()));
                }
            }
        }
    }

    /// Stop the throttle for `why`, and say so.
    fn stop(&self, why: Stop) -> anyhow::Error {
        self.throttle.stop(why.clone());
        self.error(why)
    }

    fn error(&self, why: Stop) -> anyhow::Error {
        match why {
            Stop::Refusing => Refusing { service: self.name }.into(),
            Stop::Unreachable(why) => Unreachable {
                service: self.name,
                why,
            }
            .into(),
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
            .map(|b| text(b).map_err(|e| anyhow::anyhow!("{}: {e}", shown(url))))
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
        let take = |at| t.take(at).unwrap();
        let start = Instant::now();
        assert_eq!(take(start), Duration::ZERO);
        assert_eq!(take(start), Duration::from_secs(1));
        assert_eq!(take(start), Duration::from_secs(2));
        let later = start + Duration::from_secs(10);
        assert_eq!(take(later), Duration::ZERO, "an idle gap is not saved up");
        t.hold(later, 2, None);
        assert_eq!(take(later), Duration::from_secs(4));
        assert_eq!(take(later), Duration::from_secs(5));
        let after = later + Duration::from_secs(60);
        t.hold(after, 1, Some(Duration::from_secs(30)));
        assert_eq!(
            take(after),
            Duration::from_secs(30),
            "the server's wait is kept"
        );
    }

    #[test]
    fn a_turn_given_before_a_refusal_waits_out_its_hold() {
        let t = Throttle::new(Duration::from_secs(1), Duration::from_secs(5));
        let start = Instant::now();
        assert_eq!(t.take(start), Ok(Duration::ZERO));
        assert_eq!(t.take(start), Ok(Duration::from_secs(1)));
        t.hold(start, 1, None);
        let turn = start + Duration::from_secs(1);
        assert_eq!(t.ready(turn), Ok(false), "the refusal came after the turn");
        assert_eq!(t.take(turn), Ok(Duration::from_secs(4)));
        assert_eq!(t.ready(start + Duration::from_secs(5)), Ok(true));
    }

    /// Answers `/a` with a 503 once, slowly, and everything else at once;
    /// says when `/a` is first asked, and keeps when each was.
    struct Slow {
        asked: Mutex<Vec<(String, Instant)>>,
        entered: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    }

    impl HttpTransport for Slow {
        fn get(
            &self,
            url: &str,
            _: &[(&str, &str)],
            _: Duration,
        ) -> Result<Vec<u8>, TransportError> {
            self.asked
                .lock()
                .unwrap()
                .push((url.to_string(), Instant::now()));
            if url.ends_with("/a")
                && let Some(entered) = self.entered.lock().unwrap().take()
            {
                entered.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(100));
                return Err(TransportError::Status {
                    code: 503,
                    body: String::new(),
                    retry_after: None,
                });
            }
            Ok(b"ok".to_vec())
        }
    }

    #[test]
    fn a_request_waiting_its_turn_is_held_back_by_a_refusal_meanwhile() {
        let (entered, first) = std::sync::mpsc::channel();
        let slow = Slow {
            asked: Mutex::new(Vec::new()),
            entered: Mutex::new(Some(entered)),
        };
        let throttle = Throttle::new(Duration::from_millis(300), Duration::from_secs(1));
        let service = Service {
            name: "Lumo Lyrics",
            transport: &slow,
            throttle: &throttle,
        };
        let start = Instant::now();
        std::thread::scope(|s| {
            s.spawn(|| service.get("http://lyrics.test/a", &[], Duration::from_secs(1)));
            first.recv().unwrap();
            s.spawn(|| service.get("http://lyrics.test/b", &[], Duration::from_secs(1)));
        });
        let asked = slow.asked.lock().unwrap();
        let (_, b) = asked.iter().find(|(u, _)| u.ends_with("/b")).unwrap();
        assert!(
            b.duration_since(start) >= Duration::from_millis(900),
            "asked {:?} after the start, inside the hold",
            b.duration_since(start)
        );
    }

    /// Fails every request as reaching no server, counting them.
    #[derive(Default)]
    struct Down {
        asked: Mutex<usize>,
    }

    impl HttpTransport for Down {
        fn get(&self, _: &str, _: &[(&str, &str)], _: Duration) -> Result<Vec<u8>, TransportError> {
            *self.asked.lock().unwrap() += 1;
            Err(TransportError::Unreachable("connection refused".into()))
        }
    }

    #[test]
    fn a_call_that_reached_no_server_is_told_from_one_that_did() {
        use std::io::{Error, ErrorKind};
        let unreachable = |e| matches!(failure(e), TransportError::Unreachable(_));
        assert!(unreachable(ureq::Error::HostNotFound));
        assert!(unreachable(ureq::Error::ConnectProxyFailed(
            "proxy server responded 503/Service Unavailable".into()
        )));
        assert!(unreachable(ureq::Error::Io(
            ErrorKind::ConnectionRefused.into()
        )));
        assert!(unreachable(ureq::Error::Io(Error::other(
            "failed to lookup address information"
        ))));
        assert!(unreachable(ureq::Error::Timeout(ureq::Timeout::Connect)));
        assert!(matches!(
            failure(ureq::Error::Timeout(ureq::Timeout::RecvResponse)),
            TransportError::Timeout
        ));
        assert!(matches!(
            failure(ureq::Error::Io(ErrorKind::ConnectionReset.into())),
            TransportError::Transport(_)
        ));
    }

    #[test]
    fn a_service_unreachable_or_refusing_is_asked_nothing_more_in_the_run() {
        let down = Down::default();
        let throttle = Throttle::none();
        let service = Service {
            name: "Lumo Lyrics",
            transport: &down,
            throttle: &throttle,
        };
        for _ in 0..2 {
            let e = service
                .get("http://lyrics.test/a", &[], Duration::from_secs(1))
                .unwrap_err();
            assert_eq!(
                e.to_string(),
                "Lumo Lyrics cannot be reached (connection refused)"
            );
            assert!(put_off(&e.context("asking")).is_some());
        }
        assert_eq!(*down.asked.lock().unwrap(), 1, "the second is not sent");

        let transport = Scripted {
            script: Mutex::new(vec![429; 4]),
            retry_after: None,
        };
        let throttle = Throttle::none();
        let service = Service {
            name: "Lumo Lyrics",
            transport: &transport,
            throttle: &throttle,
        };
        let ask = || service.get("http://lyrics.test/a", &[], Duration::from_secs(1));
        assert!(ask().unwrap_err().downcast_ref::<Refusing>().is_some());
        assert!(
            ask().unwrap_err().downcast_ref::<Refusing>().is_some(),
            "the server would answer now, but is not asked"
        );
        assert!(put_off(&self::ask(&[500], None).unwrap_err()).is_none());
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
    fn retry_after_is_read_in_seconds_or_as_a_date() {
        let now = UNIX_EPOCH + Duration::from_secs(784_111_717);
        assert_eq!(retry_after(" 30 ", now), Some(Duration::from_secs(30)));
        assert_eq!(
            retry_after("Sun, 06 Nov 1994 08:49:37 GMT", now),
            Some(Duration::from_secs(60))
        );
        assert_eq!(
            retry_after("Sun, 06 Nov 1994 08:47:37 GMT", now),
            Some(Duration::ZERO),
            "a date gone by asks for no wait"
        );
        assert_eq!(
            http_date("Tue, 29 Feb 2028 00:00:00 GMT"),
            Some(UNIX_EPOCH + Duration::from_secs(1_835_395_200))
        );
        for bad in [
            "soon",
            "Sun, 06 Nov 1994 08:49:37 CET",
            "Sun, 32 Nov 1994 08:49:37 GMT",
        ] {
            assert_eq!(retry_after(bad, now), None, "{bad}");
        }
    }

    #[test]
    fn an_error_body_is_kept_short() {
        let page = format!("<html>\n  <p>busy</p>\n{}</html>", "é".repeat(10_000));
        let kept = excerpt(page.as_bytes());
        assert!(kept.starts_with("<html> <p>busy</p> éé"), "{kept}");
        assert_eq!(kept.chars().count(), ERROR_EXCERPT + 1);
        assert!(kept.ends_with('…'));
        assert_eq!(excerpt(b"TrackNotFound"), "TrackNotFound");
    }

    #[test]
    fn a_compressed_body_is_told_apart() {
        let err = text(vec![0x1f, 0x8b, 8, 0]).unwrap_err();
        assert!(err.contains("gzip"), "{err}");
        assert!(text(vec![0xff]).unwrap_err().contains("not UTF-8"));
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
