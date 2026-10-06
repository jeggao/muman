//! How far a long step has got: a status line pinned under the messages
//! on a terminal, plain lines now and then in a log, or JSON events for
//! a program reading them, as `--progress` says.
//!
//! A command names the steps it may take ([`Progress::plan`]), so a step shows as
//! `[3/5] Writing`; a step it skips leaves its number unused, as a
//! package manager's numbered steps do. Each step counts the items it
//! has done of how many, and names those under way: a worker holds a
//! [`Working`] while it is on one, so parallel work shows every song in
//! hand. The time left is the step's rate so far applied to what is
//! left.
//!
//! | Mode | Where | What |
//! |---|---|---|
//! | `auto` on a terminal | stderr | One status line under the messages, redrawn in place: the step, a bar, the count, the time left, what is under way |
//! | `auto` elsewhere, `plain` | stderr | A line every [`PLAIN_EVERY`] or tenth of the way, and when the step ends |
//! | `json` | stderr | One JSON object a line: `start`, `progress` (at most every [`JSON_EVERY`]) and `finish` events |
//! | `none` | | Nothing |
//!
//! Every message a command writes goes through [`Console`], which lifts
//! the status line out of the way and puts it back, so the two never
//! tear each other. Terminals that show a task's progress in their tab
//! or taskbar, from the `OSC 9;4` sequence cargo and others write, get
//! it too; others would print it, so it is written only to those known
//! to read it.
//!
//! Progress is ambient, as the terminal is: [`install`] makes one the
//! run's for as long as its guard lives, and a step anywhere reports to
//! it. Nothing is installed in tests, whose steps report to nothing.

use std::fmt::Write as _;
use std::io::{self, Write};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

/// How often a plain line says how far a step has got, at most.
pub const PLAIN_EVERY: Duration = Duration::from_secs(10);
/// How often a JSON `progress` event is written, at most.
pub const JSON_EVERY: Duration = Duration::from_millis(500);
/// A step quicker than this shows no status line, which would only
/// flicker.
const SHOW_AFTER: Duration = Duration::from_millis(300);

/// What `--progress` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Mode {
    /// A status line on a terminal, plain lines elsewhere.
    #[default]
    Auto,
    /// Plain lines, terminal or not.
    Plain,
    /// JSON events, one a line.
    Json,
    /// Nothing.
    None,
}

/// How a run shows progress, once its terminal is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A status line, and the tab or taskbar's progress when the
    /// terminal shows it.
    Bar {
        taskbar: bool,
    },
    Plain,
    Json,
    Hidden,
}

impl Kind {
    /// What `mode` comes to on a stderr that is a terminal or not.
    #[must_use]
    pub fn of(mode: Mode, terminal: bool, taskbar: bool) -> Self {
        match mode {
            Mode::Auto if terminal => Self::Bar { taskbar },
            Mode::Auto | Mode::Plain => Self::Plain,
            Mode::Json => Self::Json,
            Mode::None => Self::Hidden,
        }
    }
}

/// Where progress goes, shared by every thread of a run.
#[derive(Clone)]
pub struct Progress {
    inner: Arc<Inner>,
}

struct Inner {
    kind: Kind,
    /// Where plain lines, JSON events and the taskbar's sequence go.
    sink: Mutex<Box<dyn Write + Send>>,
    plan: Mutex<Vec<String>>,
    /// The bar showing now, for [`Console`] to lift out of the way.
    bar: Mutex<Option<ProgressBar>>,
}

impl std::fmt::Debug for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Progress")
            .field("kind", &self.inner.kind)
            .finish_non_exhaustive()
    }
}

impl Progress {
    /// Progress of `kind`, written to `sink`.
    #[must_use]
    pub fn new(kind: Kind, sink: Box<dyn Write + Send>) -> Self {
        Self {
            inner: Arc::new(Inner {
                kind,
                sink: Mutex::new(sink),
                plan: Mutex::new(Vec::new()),
                bar: Mutex::new(None),
            }),
        }
    }

    /// Progress shown nowhere.
    #[must_use]
    pub fn hidden() -> Self {
        Self::new(Kind::Hidden, Box::new(io::sink()))
    }

    /// The steps the command may take, in order, numbering each.
    pub fn plan(&self, steps: &[&str]) {
        *lock(&self.inner.plan) = steps.iter().map(ToString::to_string).collect();
    }

    /// Begin a step of `total` items, or of a number not known yet; it
    /// ends when dropped.
    #[must_use]
    pub fn step(&self, name: &str, total: Option<u64>) -> Step {
        let plan = lock(&self.inner.plan);
        let index = plan
            .iter()
            .position(|s| s == name)
            .map(|n| (n + 1, plan.len()));
        drop(plan);
        let state = Arc::new(State {
            name: name.to_string(),
            index,
            total: Mutex::new(total),
            done: Mutex::new(0),
            working: Mutex::new(Vec::new()),
            note: Mutex::new(None),
            started: Instant::now(),
            reported: Mutex::new(Reported::default()),
            bar: Mutex::new(None),
        });
        let step = Step {
            progress: self.clone(),
            state,
        };
        step.started();
        step
    }

    /// Run `f` with the status line lifted out of the way, as a prompt
    /// or a message must.
    pub fn suspend<R>(&self, f: impl FnOnce() -> R) -> R {
        let bar = lock(&self.inner.bar).clone();
        match bar {
            Some(bar) => bar.suspend(f),
            None => f(),
        }
    }

    fn write(&self, text: &str) {
        let mut sink = lock(&self.inner.sink);
        let _ = sink.write_all(text.as_bytes());
        let _ = sink.flush();
    }

    fn taskbar(&self) -> bool {
        matches!(self.inner.kind, Kind::Bar { taskbar: true })
    }
}

/// One step under way.
#[derive(Debug)]
pub struct Step {
    progress: Progress,
    state: Arc<State>,
}

#[derive(Debug)]
struct State {
    name: String,
    /// Its place in the command's plan, and the plan's length.
    index: Option<(usize, usize)>,
    total: Mutex<Option<u64>>,
    done: Mutex<u64>,
    /// What workers are on now, in the order they took it.
    working: Mutex<Vec<String>>,
    /// What the step says of itself when no item is under way.
    note: Mutex<Option<String>>,
    started: Instant,
    reported: Mutex<Reported>,
    bar: Mutex<Option<ProgressBar>>,
}

/// What was last said of a step, so plain lines and events come at a
/// pace.
#[derive(Debug, Default)]
struct Reported {
    at: Option<Instant>,
    tenth: u64,
    percent: Option<u64>,
}

/// An item a worker is on; done when dropped.
#[derive(Debug)]
pub struct Working<'a> {
    step: &'a Step,
    name: String,
}

impl Drop for Working<'_> {
    fn drop(&mut self) {
        {
            let mut working = lock(&self.step.state.working);
            if let Some(at) = working.iter().position(|w| *w == self.name) {
                working.remove(at);
            }
        }
        *lock(&self.step.state.done) += 1;
        self.step.update();
    }
}

impl Step {
    /// The step's title: `[3/5] Writing`, or the name alone off the plan.
    #[must_use]
    pub fn title(&self) -> String {
        match self.state.index {
            Some((n, of)) => format!("[{n}/{of}] {}", self.state.name),
            None => self.state.name.clone(),
        }
    }

    /// Begin on the item `name`, done when the guard is dropped.
    #[must_use]
    pub fn working(&self, name: &str) -> Working<'_> {
        lock(&self.state.working).push(name.to_string());
        self.update();
        Working {
            step: self,
            name: name.to_string(),
        }
    }

    /// Count `n` more items done, with none named under way.
    pub fn advance(&self, n: u64) {
        *lock(&self.state.done) += n;
        self.update();
    }

    /// Say what is under way when it is no item counted, as a download's
    /// progress is.
    pub fn note(&self, text: &str) {
        *lock(&self.state.note) = Some(text.to_string());
        self.update();
    }

    /// Set how many items the step has, once known.
    pub fn set_total(&self, total: u64) {
        *lock(&self.state.total) = Some(total);
        self.update();
    }

    /// What it has done, of how many.
    #[must_use]
    pub fn count(&self) -> (u64, Option<u64>) {
        (*lock(&self.state.done), *lock(&self.state.total))
    }

    fn started(&self) {
        if self.progress.inner.kind == Kind::Json {
            self.event("start");
        }
    }

    /// The time left at the rate so far, once anything is done.
    fn left(&self) -> Option<Duration> {
        let (done, total) = self.count();
        let total = total?;
        if done == 0 || done >= total {
            return None;
        }
        let count = |n: u64| f64::from(u32::try_from(n).unwrap_or(u32::MAX));
        let per = self.state.started.elapsed().as_secs_f64() / count(done);
        Some(Duration::from_secs_f64(per * count(total - done)))
    }

    /// What is under way, the names of the items in hand first.
    fn doing(&self) -> String {
        let working = lock(&self.state.working);
        if working.is_empty() {
            lock(&self.state.note).clone().unwrap_or_default()
        } else {
            working.join(" · ")
        }
    }

    fn update(&self) {
        match self.progress.inner.kind {
            Kind::Bar { .. } => self.draw(),
            Kind::Plain => self.plain(false),
            Kind::Json => {
                let due = {
                    let mut r = lock(&self.state.reported);
                    let due = r.at.is_none_or(|at| at.elapsed() >= JSON_EVERY);
                    if due {
                        r.at = Some(Instant::now());
                    }
                    due
                };
                if due {
                    self.event("progress");
                }
            }
            Kind::Hidden => {}
        }
    }

    fn draw(&self) {
        let (done, total) = self.count();
        let mut bar = lock(&self.state.bar);
        if bar.is_none() {
            if self.state.started.elapsed() < SHOW_AFTER && total.is_some_and(|t| done < t) {
                return;
            }
            let new = match total {
                Some(t) => ProgressBar::with_draw_target(Some(t), ProgressDrawTarget::stderr()),
                None => ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr()),
            };
            new.set_style(style(total.is_some()));
            new.set_prefix(self.title());
            new.enable_steady_tick(Duration::from_millis(200));
            *lock(&self.progress.inner.bar) = Some(new.clone());
            *bar = Some(new);
        }
        if let Some(bar) = bar.as_ref() {
            if let Some(t) = total {
                bar.set_length(t);
            }
            bar.set_position(done);
            bar.set_message(self.doing());
        }
        drop(bar);
        if self.progress.taskbar()
            && let Some(t) = total.filter(|t| *t > 0)
        {
            let percent = done.min(t) * 100 / t;
            let mut r = lock(&self.state.reported);
            if r.percent != Some(percent) {
                r.percent = Some(percent);
                drop(r);
                self.progress.write(&format!("\x1b]9;4;1;{percent}\x07"));
            }
        }
    }

    /// A plain line when a tenth more is done or [`PLAIN_EVERY`] has
    /// passed, and when the step ends.
    fn plain(&self, end: bool) {
        let (done, total) = self.count();
        let tenth = total.filter(|t| *t > 0).map_or(0, |t| done * 10 / t);
        let due = {
            let mut r = lock(&self.state.reported);
            let since = r.at.unwrap_or(self.state.started);
            // The last tenth is the end's to say.
            let finished = total.is_some_and(|t| done >= t);
            let due = end || (!finished && (tenth > r.tenth || since.elapsed() >= PLAIN_EVERY));
            if due {
                r.at = Some(Instant::now());
                r.tenth = tenth;
            }
            due
        };
        if !due {
            return;
        }
        let mut line = format!("{}: {done}", self.title());
        if let Some(t) = total {
            let _ = write!(line, "/{t}");
        }
        if end {
            let _ = write!(line, ", done in {}", human(self.state.started.elapsed()));
        } else if let Some(left) = self.left().filter(|l| l.as_secs() > 0) {
            let _ = write!(line, ", about {} left", human(left));
        }
        self.progress.write(&format!("{line}\n"));
    }

    fn event(&self, event: &str) {
        let (done, total) = self.count();
        let working = lock(&self.state.working).clone();
        let mut value = serde_json::json!({
            "event": event,
            "step": self.state.name,
            "done": done,
            "total": total,
        });
        if let Some((n, of)) = self.state.index {
            value["index"] = n.into();
            value["steps"] = of.into();
        }
        match event {
            "progress" => {
                value["working"] = working.into();
                if let Some(note) = lock(&self.state.note).clone() {
                    value["note"] = note.into();
                }
                if let Some(left) = self.left() {
                    value["eta_secs"] = left.as_secs().into();
                }
            }
            "finish" => value["secs"] = self.state.started.elapsed().as_secs_f64().into(),
            _ => {}
        }
        self.progress.write(&format!("{value}\n"));
    }
}

impl Drop for Step {
    fn drop(&mut self) {
        match self.progress.inner.kind {
            Kind::Bar { .. } => {
                if let Some(bar) = lock(&self.state.bar).take() {
                    bar.finish_and_clear();
                }
                *lock(&self.progress.inner.bar) = None;
                if self.progress.taskbar() {
                    self.progress.write("\x1b]9;4;0\x07");
                }
            }
            Kind::Plain => self.plain(true),
            Kind::Json => self.event("finish"),
            Kind::Hidden => {}
        }
    }
}

/// The status line: the step and its bar, count and time left, then what
/// is under way, cut to the terminal's width.
fn style(counted: bool) -> ProgressStyle {
    let template = if counted {
        "{prefix:.cyan} [{bar:24}] {pos}/{len} {percent:>3}% {left:>12}  {wide_msg:.dim}"
    } else {
        "{prefix:.cyan} {spinner} {pos}  {wide_msg:.dim}"
    };
    ProgressStyle::with_template(template)
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars("=> ")
        // Nothing until something is done, when there is a rate to go by.
        .with_key(
            "left",
            |state: &indicatif::ProgressState, w: &mut dyn std::fmt::Write| {
                let left = state.eta();
                if state.len().is_some_and(|l| state.pos() < l) && left.as_secs() > 0 {
                    let _ = write!(w, "{} left", human(left));
                }
            },
        )
}

/// `d` as `41 s`, `2 min 5 s` or `1 h 3 min`.
fn human(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min {} s", s / 60, s % 60),
        _ => format!("{} h {} min", s / 3600, s % 3600 / 60),
    }
}

/// How an item is named in what is under way: a file's name without its
/// folders or extension.
#[must_use]
pub fn label(path: &std::path::Path) -> String {
    path.file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned())
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The run's progress, made so by [`install`].
static CURRENT: RwLock<Option<Progress>> = RwLock::new(None);

/// While the guard lives, `progress` is the run's.
#[derive(Debug)]
pub struct Installed;

impl Drop for Installed {
    fn drop(&mut self) {
        *CURRENT.write().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// Make `progress` the run's until the guard is dropped.
#[must_use]
pub fn install(progress: Progress) -> Installed {
    *CURRENT.write().unwrap_or_else(PoisonError::into_inner) = Some(progress);
    Installed
}

/// The run's progress, or one shown nowhere when none is installed.
#[must_use]
pub fn current() -> Progress {
    CURRENT
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .unwrap_or_else(Progress::hidden)
}

/// Begin a step of the run's progress.
#[must_use]
pub fn step(name: &str, total: Option<u64>) -> Step {
    current().step(name, total)
}

/// Run `f` with the run's status line lifted out of the way.
pub fn suspend<R>(f: impl FnOnce() -> R) -> R {
    current().suspend(f)
}

/// Whether the run draws a status line, which a yt-dlp download's
/// progress then goes into rather than a line of its own.
#[must_use]
pub fn drawing() -> bool {
    matches!(current().inner.kind, Kind::Bar { .. })
}

/// A writer whose lines go above the run's status line.
#[derive(Debug)]
pub struct Console<W: Write> {
    inner: W,
    line: Vec<u8>,
}

impl<W: Write> Console<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            line: Vec::new(),
        }
    }
}

impl<W: Write> Write for Console<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.line.extend_from_slice(buf);
        if let Some(end) = self.line.iter().rposition(|b| *b == b'\n') {
            let whole: Vec<u8> = self.line.drain(..=end).collect();
            suspend(|| {
                self.inner.write_all(&whole)?;
                self.inner.flush()
            })?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.line.is_empty() {
            let part: Vec<u8> = std::mem::take(&mut self.line);
            suspend(|| self.inner.write_all(&part))?;
        }
        self.inner.flush()
    }
}

impl<W: Write> Drop for Console<W> {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sink whose bytes a test reads back.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            lock(&self.0).extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Shared {
        fn text(&self) -> String {
            String::from_utf8(lock(&self.0).clone()).unwrap()
        }
    }

    #[test]
    fn a_step_is_numbered_by_the_plan_and_counts_what_workers_finish() {
        let progress = Progress::hidden();
        progress.plan(&["Measuring", "Writing"]);
        let step = progress.step("Writing", Some(3));
        assert_eq!(step.title(), "[2/2] Writing");
        {
            let _a = step.working("Lantern Weather");
            let _b = step.working("Rooms of Salt");
            assert_eq!(step.doing(), "Lantern Weather · Rooms of Salt");
        }
        step.advance(1);
        assert_eq!(step.count(), (3, Some(3)));
        assert_eq!(progress.step("Fitting", None).title(), "Fitting");
    }

    #[test]
    fn plain_lines_say_each_tenth_and_the_end() {
        let sink = Shared::default();
        let progress = Progress::new(Kind::Plain, Box::new(sink.clone()));
        {
            let step = progress.step("Writing", Some(20));
            for _ in 0..20 {
                step.advance(1);
            }
        }
        let text = sink.text();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("Writing: 2/20"), "{text}");
        assert_eq!(lines.len(), 10, "nine tenths, then the end: {text}");
        assert!(lines[9].starts_with("Writing: 20/20, done in"), "{text}");
    }

    #[test]
    fn json_events_start_report_and_finish() {
        let sink = Shared::default();
        let progress = Progress::new(Kind::Json, Box::new(sink.clone()));
        progress.plan(&["Looking up"]);
        {
            let step = progress.step("Looking up", Some(2));
            let _w = step.working("Paper Comets");
        }
        let events: Vec<serde_json::Value> = sink
            .text()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(events[0]["event"], "start");
        assert_eq!(events[0]["index"], 1);
        assert_eq!(events[1]["event"], "progress");
        assert_eq!(events[1]["working"][0], "Paper Comets");
        let last = events.last().unwrap();
        assert_eq!(
            (last["event"].as_str(), last["done"].as_u64()),
            (Some("finish"), Some(1))
        );
    }

    #[test]
    fn auto_draws_only_on_a_terminal() {
        assert_eq!(
            Kind::of(Mode::Auto, true, true),
            Kind::Bar { taskbar: true }
        );
        assert_eq!(Kind::of(Mode::Auto, false, true), Kind::Plain);
        assert_eq!(Kind::of(Mode::None, true, true), Kind::Hidden);
    }

    #[test]
    fn a_console_writes_whole_lines_through() {
        let mut out = Vec::new();
        {
            let mut console = Console::new(&mut out);
            write!(console, "Written").unwrap();
            writeln!(console, " again").unwrap();
            write!(console, "tail").unwrap();
        }
        assert_eq!(String::from_utf8(out).unwrap(), "Written again\ntail");
    }
}
