//! Messages, colors and prompts.
//!
//! Every message is written with its ANSI style; `run` wraps stdout and
//! stderr in `anstream` streams, which strip the styles when the stream
//! is no terminal or `NO_COLOR` is set, and translate them for the
//! legacy Windows console. Nothing here holds process-wide state, so
//! tests capture styled text in a buffer and strip it with [`plain`].
//!
//! Much of what muman says carries text from elsewhere: a video's or a
//! playlist's title, a torrent's name, a line yt-dlp wrote, a service's
//! error. Such text can hold what a terminal acts on rather than shows,
//! and `anstream` passes it to a terminal as it is: an `OSC` sequence
//! retitles the window, writes the clipboard or makes a link that reads
//! as one address and opens another, and a `CSI` one or a carriage
//! return clears or overwrites what muman said. So every text muman
//! prints is made [`printable`] on its way out, and only the text, never
//! muman's own styles: by [`Painted`], which every message here and every
//! styled piece goes through, by [`line()`] and [`painted`] for lines of
//! no style or of several, by [`InquirePrompter`] for a prompt's question
//! and items, and by `progress` for what a step says is under way.
//! Fixing each source instead would miss the next one added, and a rule
//! in `tests/architecture.rs` keeps other code from writing around them.
//!
//! [`printable`] shows each control character but a line break and a
//! tab as U+FFFD, so the rest of a sequence shows as the text it is.
//! The bidirectional embeddings, overrides and isolates go too: one
//! left open in a title turns the rest of the line around, muman's own
//! words and paths after it included, so a line can read as something
//! it does not say. A right-to-left title needs none of them to show
//! in its order, and the marks that only set a neutral character's
//! direction stay.

use std::borrow::Cow;
use std::fmt;
use std::io::{self, Write};

use anstyle::{AnsiColor, Effects};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Info,
    Success,
    Warning,
    Danger,
    Accent,
    Path,
    Muted,
    /// No style, for a piece of a line that has others.
    Plain,
}

impl Style {
    fn style(self) -> anstyle::Style {
        let fg = |c: AnsiColor| anstyle::Style::new().fg_color(Some(c.into()));
        match self {
            Self::Info => fg(AnsiColor::Cyan).effects(Effects::DIMMED),
            Self::Success => fg(AnsiColor::Green).effects(Effects::BOLD),
            Self::Warning => fg(AnsiColor::Magenta),
            Self::Danger => fg(AnsiColor::Red).effects(Effects::BOLD),
            Self::Accent => fg(AnsiColor::Cyan),
            Self::Path => fg(AnsiColor::Yellow),
            Self::Muted => anstyle::Style::new().effects(Effects::DIMMED),
            Self::Plain => anstyle::Style::new(),
        }
    }

    #[must_use]
    pub fn paint(self, text: &str) -> Painted<'_> {
        Painted { style: self, text }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Painted<'a> {
    style: Style,
    text: &'a str,
}

impl fmt::Display for Painted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let style = self.style.style();
        write!(f, "{style}{}{style:#}", printable(self.text))
    }
}

/// `text` as a terminal shows it rather than acts on it: each control
/// character but a line break and a tab, and each bidirectional
/// embedding, override and isolate, shown as U+FFFD. A carriage return
/// ending a line, as a Windows program writes one, is dropped.
#[must_use]
pub fn printable(text: &str) -> Cow<'_, str> {
    if !text.chars().any(acts) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' && chars.peek() == Some(&'\n') {
            continue;
        }
        out.push(if acts(c) {
            char::REPLACEMENT_CHARACTER
        } else {
            c
        });
    }
    Cow::Owned(out)
}

/// Whether a terminal acts on `c`, or it turns the text after it around.
fn acts(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t')
        || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// `text` without its styles.
#[must_use]
pub fn plain(text: &str) -> String {
    anstream::adapter::strip_str(text).to_string()
}

/// `n` bytes in the largest binary unit that keeps it at least one.
#[must_use]
pub fn bytes(n: u64) -> String {
    crate::units::Size(n).to_string()
}

pub fn info<W: Write>(w: &mut W, msg: &str) -> io::Result<()> {
    writeln!(w, "{}", Style::Info.paint(msg))
}

pub fn success<W: Write>(w: &mut W, msg: &str) -> io::Result<()> {
    writeln!(w, "{}", Style::Success.paint(msg))
}

pub fn warning<W: Write>(w: &mut W, msg: &str) -> io::Result<()> {
    writeln!(w, "{}", Style::Warning.paint(msg))
}

pub fn error<W: Write>(w: &mut W, msg: &str) -> io::Result<()> {
    writeln!(w, "{}", Style::Danger.paint(msg))
}

/// A line another program wrote, as a hook's, muted.
pub fn relayed<W: Write>(w: &mut W, msg: &str) -> io::Result<()> {
    writeln!(w, "{}", Style::Muted.paint(msg))
}

pub fn trace<W: Write>(w: &mut W, msg: &str) -> io::Result<()> {
    writeln!(w, "{}", Style::Muted.paint(&format!("trace: {msg}")))
}

/// A line of no style, as a listing's or a report's.
pub fn line<W: Write>(w: &mut W, msg: &str) -> io::Result<()> {
    writeln!(w, "{}", Style::Plain.paint(msg))
}

/// A line of `pieces`, each in its own style.
pub fn painted<W: Write>(w: &mut W, pieces: &[Painted<'_>]) -> io::Result<()> {
    for piece in pieces {
        write!(w, "{piece}")?;
    }
    writeln!(w)
}

/// How the user is asked. `run` passes [`InquirePrompter`] when both
/// ends are a terminal and none otherwise; tests pass `MockPrompter`.
pub trait Prompter {
    /// A yes-or-no question; anything but yes, cancel included, is no.
    fn ask(&mut self, question: &str) -> io::Result<bool>;

    /// Pick any of `items`, those in `picked` picked to begin with.
    /// Returns the indices picked, in order.
    fn choose(
        &mut self,
        question: &str,
        items: &[String],
        picked: &[usize],
    ) -> io::Result<Vec<usize>>;

    /// Pick one of `items`; `None` when the user cancels.
    fn select(&mut self, question: &str, items: &[String]) -> io::Result<Option<usize>>;

    /// A line of text, `help` beneath it; `None` when the user cancels
    /// or types nothing.
    fn text(&mut self, question: &str, help: &str) -> io::Result<Option<String>>;

    /// A line of text that starts as `initial`, `help` beneath it, taken
    /// only once `check` finds nothing wrong with it, else asked again
    /// with what `check` says. `None` when the user cancels; `Some("")`
    /// when the line is emptied.
    fn text_with(
        &mut self,
        question: &str,
        help: &str,
        initial: &str,
        check: Check,
    ) -> io::Result<Option<String>>;

    /// The columns an item may take on the screen.
    fn width(&self) -> usize {
        80
    }
}

/// What is wrong with a line typed, or `None` when it is taken.
pub type Check = std::rc::Rc<dyn Fn(&str) -> Option<String>>;

#[derive(Debug, Default)]
pub struct InquirePrompter;

impl Prompter for InquirePrompter {
    fn ask(&mut self, question: &str) -> io::Result<bool> {
        let question = printable(question);
        let answer = crate::progress::suspend(|| {
            inquire::Confirm::new(&question)
                .with_default(false)
                .prompt_skippable()
        })
        .map_err(io::Error::other)?;
        Ok(answer == Some(true))
    }

    fn choose(
        &mut self,
        question: &str,
        items: &[String],
        picked: &[usize],
    ) -> io::Result<Vec<usize>> {
        let (question, items) = (printable(question), printable_all(items));
        let picked = crate::progress::suspend(|| {
            inquire::MultiSelect::new(&question, items)
                .with_default(picked)
                .with_help_message("space toggles, → all, ← none, type to filter, enter accepts")
                .raw_prompt_skippable()
        })
        .map_err(io::Error::other)?;
        Ok(picked
            .unwrap_or_default()
            .into_iter()
            .map(|o| o.index)
            .collect())
    }

    fn select(&mut self, question: &str, items: &[String]) -> io::Result<Option<usize>> {
        let (question, items) = (printable(question), printable_all(items));
        let picked = crate::progress::suspend(|| {
            inquire::Select::new(&question, items)
                .with_help_message("↑↓ moves, type to filter, enter picks, esc for none")
                .raw_prompt_skippable()
        })
        .map_err(io::Error::other)?;
        Ok(picked.map(|o| o.index))
    }

    fn text(&mut self, question: &str, help: &str) -> io::Result<Option<String>> {
        let (question, help) = (printable(question), printable(help));
        let typed = crate::progress::suspend(|| {
            inquire::Text::new(&question)
                .with_help_message(&help)
                .prompt_skippable()
        })
        .map_err(io::Error::other)?;
        Ok(typed.filter(|t| !t.trim().is_empty()))
    }

    fn text_with(
        &mut self,
        question: &str,
        help: &str,
        initial: &str,
        check: Check,
    ) -> io::Result<Option<String>> {
        let (question, help, initial) = (printable(question), printable(help), printable(initial));
        let validator = move |typed: &str| {
            Ok(match check(typed) {
                Some(why) => {
                    inquire::validator::Validation::Invalid(printable(&why).into_owned().into())
                }
                None => inquire::validator::Validation::Valid,
            })
        };
        crate::progress::suspend(|| {
            inquire::Text::new(&question)
                .with_help_message(&help)
                .with_initial_value(&initial)
                .with_validator(validator)
                .prompt_skippable()
        })
        .map_err(io::Error::other)
    }

    fn width(&self) -> usize {
        crate::progress::columns()
    }
}

/// Each of `items` made [`printable`], in order, so a pick's index is
/// the item's.
fn printable_all(items: &[String]) -> Vec<String> {
    items.iter().map(|i| printable(i).into_owned()).collect()
}

/// Answers from queues, in order; running out panics, surfacing the
/// question a test did not expect.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct MockPrompter {
    answers: std::collections::VecDeque<bool>,
    choices: std::collections::VecDeque<Vec<usize>>,
    selections: std::collections::VecDeque<Option<usize>>,
    texts: std::collections::VecDeque<Option<String>>,
    /// Every question asked, in order, each with the items it offered.
    pub asked: Vec<String>,
    /// The items ticked to begin with, for each `choose`, in order.
    pub ticked: Vec<Vec<usize>>,
    /// The width it says items may take; none set is the trait's own.
    width: Option<usize>,
}

#[cfg(test)]
impl MockPrompter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn answering<I: IntoIterator<Item = bool>>(answers: I) -> Self {
        Self {
            answers: answers.into_iter().collect(),
            ..Self::default()
        }
    }

    pub fn push_choice<I: IntoIterator<Item = usize>>(&mut self, picked: I) -> &mut Self {
        self.choices.push_back(picked.into_iter().collect());
        self
    }

    pub fn push_select(&mut self, picked: Option<usize>) -> &mut Self {
        self.selections.push_back(picked);
        self
    }

    pub fn push_text(&mut self, typed: Option<&str>) -> &mut Self {
        self.texts.push_back(typed.map(String::from));
        self
    }

    #[must_use]
    pub fn with_width(mut self, width: usize) -> Self {
        self.width = Some(width);
        self
    }

    fn heard(&mut self, question: &str, items: &[String]) {
        let mut asked = question.to_string();
        for item in items {
            asked.push_str("\n  ");
            asked.push_str(item);
        }
        self.asked.push(asked);
    }
}

#[cfg(test)]
impl Prompter for MockPrompter {
    fn ask(&mut self, question: &str) -> io::Result<bool> {
        self.heard(question, &[]);
        Ok(self
            .answers
            .pop_front()
            .unwrap_or_else(|| panic!("MockPrompter ran out of answers at {question:?}")))
    }

    fn choose(
        &mut self,
        question: &str,
        items: &[String],
        picked: &[usize],
    ) -> io::Result<Vec<usize>> {
        self.heard(question, items);
        self.ticked.push(picked.to_vec());
        Ok(self
            .choices
            .pop_front()
            .unwrap_or_else(|| panic!("MockPrompter ran out of choices at {question:?}")))
    }

    fn select(&mut self, question: &str, items: &[String]) -> io::Result<Option<usize>> {
        self.heard(question, items);
        Ok(self
            .selections
            .pop_front()
            .unwrap_or_else(|| panic!("MockPrompter ran out of selections at {question:?}")))
    }

    fn text(&mut self, question: &str, _: &str) -> io::Result<Option<String>> {
        self.heard(question, &[]);
        Ok(self
            .texts
            .pop_front()
            .unwrap_or_else(|| panic!("MockPrompter ran out of texts at {question:?}")))
    }

    fn text_with(
        &mut self,
        question: &str,
        _: &str,
        initial: &str,
        check: Check,
    ) -> io::Result<Option<String>> {
        self.heard(&format!("{question} [{initial}]"), &[]);
        loop {
            let typed = self
                .texts
                .pop_front()
                .unwrap_or_else(|| panic!("MockPrompter ran out of texts at {question:?}"));
            let Some(typed) = typed else {
                return Ok(None);
            };
            match check(&typed) {
                Some(why) => self.asked.push(format!("refused: {why}")),
                None => return Ok(Some(typed)),
            }
        }
    }

    fn width(&self) -> usize {
        self.width.unwrap_or(80)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_styled_and_strips_to_its_text() {
        let mut out = Vec::new();
        warning(&mut out, "careful").unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with('\x1b'), "{text:?}");
        assert_eq!(plain(&text), "careful\n");
    }

    #[test]
    fn what_a_terminal_acts_on_is_shown_rather_than_passed_on() {
        let cases = [
            ("Song \x1b]0;X\x07", "Song �]0;X�"),
            ("\x1b]52;c;Zm9v\x1b\\", "�]52;c;Zm9v�\\"),
            (
                "\x1b]8;;x.example\x07Link\x1b]8;;\x07",
                "�]8;;x.example�Link�]8;;�",
            ),
            ("\x1b[2J\x1b[1;31mRed", "�[2J�[1;31mRed"),
            ("\u{9b}2J", "�2J"),
            ("Nul\0, bell\x07, back\x08", "Nul�, bell�, back�"),
            ("Over\rwritten", "Over�written"),
            ("Rooms \u{202e}tlaS fo\u{202c}", "Rooms �tlaS fo�"),
            ("\u{2067}Isolated\u{2069}", "�Isolated�"),
        ];
        for (text, shown) in cases {
            assert_eq!(printable(text), shown, "{text:?}");
        }
        assert!(matches!(
            printable("Paper Comets\tB\u{200f}\nnext"),
            Cow::Borrowed("Paper Comets\tB\u{200f}\nnext")
        ));
        assert_eq!(printable("line\r\nnext\r\n"), "line\nnext\n");
    }

    #[test]
    fn a_message_s_text_is_printable_and_its_style_kept() {
        let mut out = Vec::new();
        error(&mut out, "Lantern \x1b]0;X\x07Weather").unwrap();
        painted(
            &mut out,
            &[
                Style::Plain.paint("  → "),
                Style::Path.paint("\x1b[2Ja.flac"),
            ],
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            !text.contains("\x1b]") && !text.contains("\x1b[2J"),
            "{text:?}"
        );
        assert!(text.starts_with("\x1b[1m\x1b[31m"), "{text:?}");
        assert_eq!(plain(&text), "Lantern �]0;X�Weather\n  → �[2Ja.flac\n");
    }

    #[test]
    fn a_trace_says_so() {
        let mut out = Vec::new();
        trace(&mut out, "x=1").unwrap();
        assert_eq!(plain(&String::from_utf8(out).unwrap()), "trace: x=1\n");
    }

    #[test]
    fn a_line_checked_is_asked_again_until_it_is_taken() {
        let mut p = MockPrompter::new();
        p.push_text(Some("spring"))
            .push_text(Some("2019"))
            .push_text(None);
        let check: Check = std::rc::Rc::new(|t: &str| {
            (!t.chars().all(|c| c.is_ascii_digit())).then(|| format!("{t} is no year"))
        });
        let typed = p.text_with("Date:", "", "2018", check.clone()).unwrap();
        assert_eq!(typed.as_deref(), Some("2019"));
        assert_eq!(p.asked, ["Date: [2018]", "refused: spring is no year"]);
        assert_eq!(p.text_with("Date:", "", "", check).unwrap(), None);
    }

    #[test]
    fn a_mock_answers_in_order() {
        let mut p = MockPrompter::answering([true, false]);
        assert!(p.ask("?").unwrap());
        assert!(!p.ask("?").unwrap());
        p.push_choice([0, 2]);
        assert_eq!(p.choose("?", &[], &[]).unwrap(), [0, 2]);
        p.push_select(Some(1)).push_text(Some("artist:venn"));
        assert_eq!(
            p.select("Which?", &["a".into(), "b".into()]).unwrap(),
            Some(1)
        );
        assert_eq!(
            p.text("Search:", "").unwrap().as_deref(),
            Some("artist:venn")
        );
        assert_eq!(p.asked[3], "Which?\n  a\n  b");
    }
}
