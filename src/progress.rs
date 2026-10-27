//! How far a long step has got: a status line pinned under the messages
//! on a terminal, plain lines now and then in a log, or JSON events for
//! a program reading them, as `--progress` says.
//!
//! A command names the steps it may take ([`Progress::plan`]), so a step shows as
//! `[3/5] Writing`; a step it skips, or begins with no items, leaves its
//! number unused, as a package manager's numbered steps do. Each step counts the items it
//! has done of how many, and names those under way: a worker holds a
//! [`Working`] while it is on one, so parallel work shows every song in
//! hand. The time left is the step's recent rate, over the last
//! [`WINDOW`], applied to what is left: items can differ in cost by
//! thousands of times, as a lookup record and an hour-long video do, and
//! the rate since the step began would say "1 s left" after a run of
//! cheap ones, then rise for the rest of the step. The window ends now,
//! not at the last item done, so a step stuck on one item says so.
//!
//! A run has one status line, whatever steps are under way. A step begun
//! inside another, as a fetch of what a round of lookups found, takes the
//! line once it has shown for `SHOW_AFTER`, and gives it back to the
//! step around it when it ends; two bars drawn to one line would take
//! turns on it with each tick. A step begun beneath the others
//! ([`step_beneath`]), as torrents fetched beside yt-dlp, takes the line
//! only once no other holds it. The line keeps to the terminal's width,
//! which is read again at each update: as it narrows, the line drops its
//! bar's percentage, then the bar, then the time left, so the names of
//! what is under way keep room and the line never wraps.
//!
//! | Mode | Where | What |
//! |---|---|---|
//! | `auto` on a terminal | stderr | One status line under the messages, redrawn in place: the step, a bar, the count, the time left, what is under way |
//! | `auto` elsewhere, `plain` | stderr | A line every [`PLAIN_EVERY`] or tenth of the way, and when the step ends |
//! | `json` | stderr | One JSON object a line: `start`, `progress` (at most every [`JSON_EVERY`]) and `finish` events |
//! | `none` | | Nothing |
//!
//! What a step says is under way, a song's name or a download's line,
//! is made [`crate::ui::printable`] as it is given, so the status line,
//! a plain line and an event show the same text, and none of them a
//! sequence a title held.
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

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::{self, Write};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

/// How often a plain line says how far a step has got, at most.
pub const PLAIN_EVERY: Duration = Duration::from_secs(10);
/// How often a JSON `progress` event is written, at most.
pub const JSON_EVERY: Duration = Duration::from_millis(500);
/// How far back the rate the time left is told by reaches.
pub const WINDOW: Duration = Duration::from_secs(60);
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
    line: Mutex<Line>,
}

/// The run's one status line, and the steps that may take it.
#[derive(Default)]
struct Line {
    /// The steps under way, the one begun last at the end: the one the
    /// line shows.
    steps: Vec<Arc<State>>,
    /// The bar showing now, for [`Console`] to lift out of the way.
    bar: Option<Shown>,
}

/// A bar on the status line, the step it shows and how it is laid out.
struct Shown {
    step: Arc<State>,
    bar: ProgressBar,
    layout: Layout,
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
                line: Mutex::new(Line::default()),
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
    /// ends when dropped. A step of no items shows nothing.
    #[must_use]
    pub fn step(&self, name: &str, total: Option<u64>) -> Step {
        self.begin(name, total, false)
    }

    /// Begin a step as [`Self::step`] does, but beneath every step under
    /// way: it takes the line only once no other holds it, as work run
    /// beside another step's does.
    #[must_use]
    pub fn step_beneath(&self, name: &str, total: Option<u64>) -> Step {
        self.begin(name, total, true)
    }

    fn begin(&self, name: &str, total: Option<u64>, beneath: bool) -> Step {
        if total == Some(0) && self.inner.kind != Kind::Hidden {
            return Self::hidden().step(name, total);
        }
        let plan = lock(&self.inner.plan);
        let index = plan
            .iter()
            .position(|s| s == name)
            .map(|n| (n + 1, plan.len()));
        drop(plan);
        let state = Arc::new(State {
            name: crate::ui::printable(name).into_owned(),
            index,
            total: Mutex::new(total),
            done: Mutex::new(0),
            working: Mutex::new(Vec::new()),
            note: Mutex::new(None),
            marks: Mutex::new(VecDeque::new()),
            started: Instant::now(),
            reported: Mutex::new(Reported::default()),
        });
        if matches!(self.inner.kind, Kind::Bar { .. }) {
            let mut line = lock(&self.inner.line);
            if beneath {
                line.steps.insert(0, Arc::clone(&state));
            } else {
                line.steps.push(Arc::clone(&state));
            }
        }
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
        let bar = lock(&self.inner.line).bar.as_ref().map(|s| s.bar.clone());
        match bar {
            Some(bar) => bar.suspend(f),
            None => f(),
        }
    }

    /// Show `state` on the status line, if it is the step begun last.
    fn draw(&self, state: &Arc<State>) {
        let (done, total) = state.count();
        let layout = Layout::of(columns(), total.is_some());
        let mut line = lock(&self.inner.line);
        if !line.steps.last().is_some_and(|s| Arc::ptr_eq(s, state)) {
            return;
        }
        if !line
            .bar
            .as_ref()
            .is_some_and(|s| Arc::ptr_eq(&s.step, state))
        {
            if state.started.elapsed() < SHOW_AFTER && total.is_some_and(|t| done < t) {
                return;
            }
            if let Some(old) = line.bar.take() {
                old.bar.finish_and_clear();
            }
            // Begun again, as when a step inside it ends, the bar goes on
            // from the step's start rather than from now.
            let bar = ProgressBar::with_draw_target(total, ProgressDrawTarget::stderr())
                .with_elapsed(state.started.elapsed())
                .with_position(done)
                .with_message(state.doing())
                .with_style(layout.style(Arc::clone(state)))
                .with_prefix(state.title());
            bar.enable_steady_tick(Duration::from_millis(200));
            lock(&state.reported).percent = None;
            line.bar = Some(Shown {
                step: Arc::clone(state),
                bar,
                layout,
            });
        }
        if let Some(shown) = line.bar.as_mut() {
            if shown.layout != layout {
                shown.bar.set_style(layout.style(Arc::clone(state)));
                shown.layout = layout;
            }
            if let Some(t) = total {
                shown.bar.set_length(t);
            }
            shown.bar.set_position(done);
            shown.bar.set_message(state.doing());
        }
        drop(line);
        if self.taskbar()
            && let Some(t) = total.filter(|t| *t > 0)
        {
            let percent = done.min(t) * 100 / t;
            let mut r = lock(&state.reported);
            if r.percent != Some(percent) {
                r.percent = Some(percent);
                drop(r);
                self.write(&format!("\x1b]9;4;1;{percent}\x07"));
            }
        }
    }

    /// Take `state` off the status line, and give the line back to the
    /// step it was begun in.
    fn end(&self, state: &Arc<State>) {
        let outer = {
            let mut line = lock(&self.inner.line);
            line.steps.retain(|s| !Arc::ptr_eq(s, state));
            if line
                .bar
                .as_ref()
                .is_some_and(|s| Arc::ptr_eq(&s.step, state))
                && let Some(shown) = line.bar.take()
            {
                shown.bar.finish_and_clear();
            }
            line.steps.last().cloned()
        };
        match outer {
            Some(outer) => self.draw(&outer),
            None if self.taskbar() => self.write("\x1b]9;4;0\x07"),
            None => {}
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
    /// When each count was reached since the step began, back to the
    /// last before the [`WINDOW`].
    marks: Mutex<VecDeque<(Duration, u64)>>,
    started: Instant,
    reported: Mutex<Reported>,
}

impl State {
    fn title(&self) -> String {
        match self.index {
            Some((n, of)) => format!("[{n}/{of}] {}", self.name),
            None => self.name.clone(),
        }
    }

    fn count(&self) -> (u64, Option<u64>) {
        (*lock(&self.done), *lock(&self.total))
    }

    /// The time left at the recent rate, once anything is done.
    fn left(&self) -> Option<Duration> {
        let (done, total) = self.count();
        left(self.started.elapsed(), &lock(&self.marks), done, total?)
    }

    /// Count `n` more items done, now.
    fn add(&self, n: u64) {
        let mut done = lock(&self.done);
        *done += n;
        mark(&mut lock(&self.marks), self.started.elapsed(), *done);
    }

    /// What is under way, the names of the items in hand first.
    fn doing(&self) -> String {
        let working = lock(&self.working);
        if working.is_empty() {
            lock(&self.note).clone().unwrap_or_default()
        } else {
            working.join(" · ")
        }
    }
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
        self.step.state.add(1);
        self.step.update();
    }
}

impl Step {
    /// The step's title: `[3/5] Writing`, or the name alone off the plan.
    #[must_use]
    pub fn title(&self) -> String {
        self.state.title()
    }

    /// Begin on the item `name`, done when the guard is dropped.
    #[must_use]
    pub fn working(&self, name: &str) -> Working<'_> {
        let name = crate::ui::printable(name).into_owned();
        lock(&self.state.working).push(name.clone());
        self.update();
        Working { step: self, name }
    }

    /// Count `n` more items done, with none named under way.
    pub fn advance(&self, n: u64) {
        self.state.add(n);
        self.update();
    }

    /// Say what is under way when it is no item counted, as a download's
    /// progress is.
    pub fn note(&self, text: &str) {
        *lock(&self.state.note) = Some(crate::ui::printable(text).into_owned());
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
        self.state.count()
    }

    fn started(&self) {
        if self.progress.inner.kind == Kind::Json {
            self.event("start");
        }
    }

    fn update(&self) {
        match self.progress.inner.kind {
            Kind::Bar { .. } => self.progress.draw(&self.state),
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
        let note = lock(&self.state.note)
            .clone()
            .filter(|_| total.is_none() && !end);
        let mut line = match note {
            // A step that counts nothing, as aria2c's, says only its note.
            Some(note) if done == 0 => format!("{}: {note}", self.title()),
            _ => format!("{}: {done}", self.title()),
        };
        if let Some(t) = total {
            let _ = write!(line, "/{t}");
        }
        if end {
            let _ = write!(line, ", done in {}", human(self.state.started.elapsed()));
        } else if let Some(left) = self.state.left().filter(|l| l.as_secs() > 0) {
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
                if let Some(left) = self.state.left() {
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
            Kind::Bar { .. } => self.progress.end(&self.state),
            Kind::Plain => self.plain(true),
            Kind::Json => self.event("finish"),
            Kind::Hidden => {}
        }
    }
}

/// What the status line holds, by the terminal's width: the step, then
/// its bar, count, percentage and time left as they fit, then what is
/// under way in whatever is left, cut to fit.
///
/// Each layout is used from the width where its fixed fields at their
/// longest, `[2/2] Writing the zip`, `12345/12345` and `59 min 59 s
/// left`, leave a dozen columns for the names of what is under way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// A step of a number of items not known yet: a spinner and the count.
    Uncounted,
    Full,
    /// A shorter bar, without the percentage it shows already.
    Short,
    /// The count and the time left.
    Count,
    /// The count alone.
    Bare,
}

impl Layout {
    const FULL_FROM: usize = 100;
    const SHORT_FROM: usize = 80;
    const COUNT_FROM: usize = 60;

    fn of(columns: usize, counted: bool) -> Self {
        match columns {
            _ if !counted => Self::Uncounted,
            Self::FULL_FROM.. => Self::Full,
            Self::SHORT_FROM.. => Self::Short,
            Self::COUNT_FROM.. => Self::Count,
            _ => Self::Bare,
        }
    }

    fn template(self) -> &'static str {
        match self {
            Self::Uncounted => "{prefix:.cyan} {spinner} {pos}  {wide_msg:.dim}",
            Self::Full => {
                "{prefix:.cyan} [{bar:24}] {pos}/{len} {percent:>3}% {left:>12}  {wide_msg:.dim}"
            }
            Self::Short => "{prefix:.cyan} [{bar:12}] {pos}/{len} {left}  {wide_msg:.dim}",
            Self::Count => "{prefix:.cyan} {pos}/{len} {left}  {wide_msg:.dim}",
            Self::Bare => "{prefix:.cyan} {pos}/{len}  {wide_msg:.dim}",
        }
    }

    fn style(self, step: Arc<State>) -> ProgressStyle {
        ProgressStyle::with_template(self.template())
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("=> ")
            .with_key(
                "left",
                move |_: &indicatif::ProgressState, w: &mut dyn std::fmt::Write| {
                    if let Some(left) = step.left().filter(|l| l.as_secs() > 0) {
                        let _ = write!(w, "{} left", human(left));
                    }
                },
            )
    }
}

/// The terminal's width in columns, or a common width when stderr is no
/// terminal.
pub(crate) fn columns() -> usize {
    terminal_size::terminal_size_of(io::stderr()).map_or(80, |(w, _)| usize::from(w.0))
}

/// `done` reached at `at`, with the marks before the last one at or
/// before the window's start let go.
fn mark(marks: &mut VecDeque<(Duration, u64)>, at: Duration, done: u64) {
    marks.push_back((at, done));
    let start = at.saturating_sub(WINDOW);
    while marks.get(1).is_some_and(|(t, _)| *t <= start) {
        marks.pop_front();
    }
}

/// The time `total - done` items take at the rate of the [`WINDOW`]
/// ending at `elapsed`: from the last of `marks` at or before its start,
/// else from the step's start. With nothing done since, the time since
/// counts as one item's, so a stuck step's estimate grows. Nothing until
/// something is done, when there is a rate to go by.
fn left(
    elapsed: Duration,
    marks: &VecDeque<(Duration, u64)>,
    done: u64,
    total: u64,
) -> Option<Duration> {
    if done == 0 || done >= total {
        return None;
    }
    let start = elapsed.saturating_sub(WINDOW);
    let (from, before) = marks
        .iter()
        .rev()
        .find(|(t, _)| *t <= start)
        .copied()
        .unwrap_or_default();
    let count = |n: u64| f64::from(u32::try_from(n).unwrap_or(u32::MAX));
    let per =
        elapsed.saturating_sub(from).as_secs_f64() / count(done.saturating_sub(before).max(1));
    Some(Duration::from_secs_f64(per * count(total - done)))
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

/// Begin a step of the run's progress beneath every step under way.
#[must_use]
pub fn step_beneath(name: &str, total: Option<u64>) -> Step {
    current().step_beneath(name, total)
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
    /// Whether the reader closed the pipe, as `| head` does once it has
    /// read enough: what the run says after is dropped, and the run goes
    /// on rather than stop halfway through writing the library.
    closed: bool,
    /// Beside `json` events, each message is one too, so that every line
    /// on stderr reads as JSON.
    events: bool,
}

impl<W: Write> Console<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            line: Vec::new(),
            closed: false,
            events: false,
        }
    }

    /// The console, each line a `message` event under `--progress json`.
    #[must_use]
    pub fn for_kind(inner: W, kind: Kind) -> Self {
        Self {
            inner,
            line: Vec::new(),
            closed: false,
            events: kind == Kind::Json,
        }
    }

    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.closed {
            return Ok(());
        }
        let events;
        let bytes = if self.events {
            events = message_events(bytes);
            events.as_bytes()
        } else {
            bytes
        };
        match suspend(|| {
            self.inner.write_all(bytes)?;
            self.inner.flush()
        }) {
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {
                self.closed = true;
                Ok(())
            }
            done => done,
        }
    }
}

/// Each line of `bytes`, its colours taken out, as a `message` event.
fn message_events(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut events = String::new();
    for line in anstream::adapter::strip_str(&text)
        .to_string()
        .lines()
        .filter(|l| !l.trim().is_empty())
    {
        events.push_str(&serde_json::json!({"event": "message", "text": line}).to_string());
        events.push('\n');
    }
    events
}

impl<W: Write> Write for Console<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.line.extend_from_slice(buf);
        if let Some(end) = self.line.iter().rposition(|b| *b == b'\n') {
            let whole: Vec<u8> = self.line.drain(..=end).collect();
            self.put(&whole)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let part: Vec<u8> = std::mem::take(&mut self.line);
        self.put(&part)
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

    struct Closed(usize);

    impl Write for Closed {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            self.0 += 1;
            Err(io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_closed_pipe_drops_what_follows_rather_than_stop_the_run() {
        let mut console = Console::new(Closed(0));
        writeln!(console, "Updated: one").unwrap();
        writeln!(console, "Updated: two").unwrap();
        console.flush().unwrap();
        assert_eq!(console.inner.0, 1, "written to once, then let be");
    }

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
            assert_eq!(step.state.doing(), "Lantern Weather · Rooms of Salt");
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
    fn a_plain_line_of_a_step_counting_nothing_says_its_note() {
        let sink = Shared::default();
        let progress = Progress::new(Kind::Plain, Box::new(sink.clone()));
        {
            let step = progress.step("Fetching torrents", None);
            step.state.reported.lock().unwrap().at = Instant::now().checked_sub(PLAIN_EVERY);
            step.note("aria2c: 3.0MiB/10MiB(28%)");
        }
        assert!(
            sink.text()
                .starts_with("Fetching torrents: aria2c: 3.0MiB/10MiB(28%)\n"),
            "{}",
            sink.text()
        );
    }

    #[test]
    fn what_a_step_says_is_under_way_is_printable() {
        let sink = Shared::default();
        let progress = Progress::new(Kind::Json, Box::new(sink.clone()));
        {
            let step = progress.step("Fetching", Some(2));
            step.note("Lantern \x1b]0;X\x07Weather");
            let _w = step.working("Rooms \x1b[2Jof Salt");
            assert_eq!(step.state.doing(), "Rooms �[2Jof Salt");
        }
        let text = sink.text();
        assert!(
            !text.contains('\x1b') && !text.contains("\\u001b"),
            "{text}"
        );
        let progress: serde_json::Value = text
            .lines()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
            .find(|e| e["event"] == "progress")
            .unwrap();
        assert_eq!(progress["note"], "Lantern �]0;X�Weather");
    }

    #[test]
    fn a_step_of_no_items_writes_nothing() {
        for kind in [Kind::Plain, Kind::Json] {
            let sink = Shared::default();
            let progress = Progress::new(kind, Box::new(sink.clone()));
            progress.plan(&["Measuring", "Writing"]);
            drop(progress.step("Writing", Some(0)));
            assert_eq!(sink.text(), "", "{kind:?}");
        }
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
    fn a_step_begun_inside_another_takes_the_line_and_gives_it_back() {
        let sink = Shared::default();
        let progress = Progress::new(Kind::Bar { taskbar: true }, Box::new(sink.clone()));
        let showing = || {
            lock(&progress.inner.line)
                .bar
                .as_ref()
                .map(|s| s.step.name.clone())
        };
        let outer = progress.step("Looking up", Some(2));
        outer.advance(2);
        assert_eq!(showing().as_deref(), Some("Looking up"));
        {
            let inner = progress.step("Fetching", Some(1));
            inner.advance(1);
            assert_eq!(showing().as_deref(), Some("Fetching"));
            outer.note("Paper Comets");
            assert_eq!(showing().as_deref(), Some("Fetching"));
        }
        assert_eq!(showing().as_deref(), Some("Looking up"));
        assert!(!sink.text().contains("\x1b]9;4;0"), "{:?}", sink.text());
        drop(outer);
        assert_eq!(showing(), None);
        assert!(sink.text().ends_with("\x1b]9;4;0\x07"), "{:?}", sink.text());
    }

    #[test]
    fn a_step_begun_beneath_takes_the_line_once_no_other_holds_it() {
        let progress = Progress::new(Kind::Bar { taskbar: false }, Box::new(io::sink()));
        let showing = || {
            lock(&progress.inner.line)
                .bar
                .as_ref()
                .map(|s| s.step.name.clone())
        };
        let fetching = progress.step("Fetching", None);
        fetching.note("Paper Comets");
        let torrents = progress.step_beneath("Fetching torrents", None);
        torrents.note("Lantern Weather 3%");
        assert_eq!(showing().as_deref(), Some("Fetching"));
        drop(fetching);
        assert_eq!(showing().as_deref(), Some("Fetching torrents"));
        let looking = progress.step("Looking up", None);
        looking.note("Rooms of Salt");
        assert_eq!(
            showing().as_deref(),
            Some("Looking up"),
            "a step begun later"
        );
    }

    #[test]
    fn the_line_drops_fields_as_the_terminal_narrows() {
        assert_eq!(Layout::of(120, true), Layout::Full);
        assert_eq!(Layout::of(99, true), Layout::Short);
        assert_eq!(Layout::of(79, true), Layout::Count);
        assert_eq!(Layout::of(40, true), Layout::Bare);
        assert_eq!(Layout::of(40, false), Layout::Uncounted);
    }

    #[test]
    fn the_time_left_follows_the_recent_rate() {
        let secs = Duration::from_secs;
        let mut marks = VecDeque::new();
        mark(&mut marks, secs(10), 1);
        assert_eq!(left(secs(10), &marks, 1, 4), Some(secs(30)));
        assert_eq!(left(secs(10), &VecDeque::new(), 0, 4), None);
        // A thousand cheap items in a second, then one every 2 s.
        let mut marks = VecDeque::new();
        mark(&mut marks, secs(1), 1000);
        let mut done = 1000;
        for t in (3..=301).step_by(2) {
            done += 1;
            mark(&mut marks, secs(t), done);
        }
        let rest = 2000 - done;
        let estimate = left(secs(301), &marks, done, 2000).unwrap();
        assert!(
            estimate.as_secs().abs_diff(2 * rest) <= 2,
            "{estimate:?} for {rest} items at 2 s each"
        );
        assert!(marks.len() <= 32, "{} marks kept", marks.len());
        // Stuck on one item, the estimate grows.
        let later = left(secs(400), &marks, done, 2000).unwrap();
        assert!(later > estimate, "{later:?} after {estimate:?}");
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

    #[test]
    fn under_json_each_message_is_an_event() {
        let mut out = Vec::new();
        {
            let mut console = Console::for_kind(&mut out, Kind::Json);
            writeln!(console, "Added: \u{1b}[32mSong\u{1b}[0m \"one\"").unwrap();
            write!(console, "\nlast").unwrap();
        }
        let text = String::from_utf8(out).unwrap();
        let events: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(events.len(), 2, "{text}");
        assert_eq!(events[0]["text"], "Added: Song \"one\"");
        assert_eq!(events[1]["event"], "message");
    }
}
