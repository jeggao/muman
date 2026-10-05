//! Messages, colors and prompts.
//!
//! Every message is written with its ANSI style; `run` wraps stdout and
//! stderr in `anstream` streams, which strip the styles when the stream
//! is no terminal or `NO_COLOR` is set, and translate them for the
//! legacy Windows console. Nothing here holds process-wide state, so
//! tests capture styled text in a buffer and strip it with [`plain`].

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
        write!(f, "{style}{}{style:#}", self.text)
    }
}

/// `text` without its styles.
#[must_use]
pub fn plain(text: &str) -> String {
    anstream::adapter::strip_str(text).to_string()
}

/// `n` bytes in the largest binary unit that keeps it at least one.
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn bytes(n: u64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", units[unit])
    }
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

pub fn trace<W: Write>(w: &mut W, msg: &str) -> io::Result<()> {
    writeln!(w, "{}", Style::Muted.paint(&format!("trace: {msg}")))
}

/// How the user is asked. `run` passes [`InquirePrompter`] when both
/// ends are a terminal and none otherwise; tests pass `MockPrompter`.
pub trait Prompter {
    /// A yes-or-no question; anything but yes, cancel included, is no.
    fn ask(&mut self, question: &str) -> io::Result<bool>;

    /// Pick any of `items`, every one picked to begin with. Returns the
    /// indices picked, in order.
    fn choose(&mut self, question: &str, items: &[String]) -> io::Result<Vec<usize>>;
}

#[derive(Debug, Default)]
pub struct InquirePrompter;

impl Prompter for InquirePrompter {
    fn ask(&mut self, question: &str) -> io::Result<bool> {
        let answer = inquire::Confirm::new(question)
            .with_default(false)
            .prompt_skippable()
            .map_err(io::Error::other)?;
        Ok(answer == Some(true))
    }

    fn choose(&mut self, question: &str, items: &[String]) -> io::Result<Vec<usize>> {
        let picked = inquire::MultiSelect::new(question, items.to_vec())
            .with_all_selected_by_default()
            .with_help_message("space toggles, → all, ← none, type to filter, enter accepts")
            .raw_prompt_skippable()
            .map_err(io::Error::other)?;
        Ok(picked
            .unwrap_or_default()
            .into_iter()
            .map(|o| o.index)
            .collect())
    }
}

/// Answers from queues, in order; running out panics, surfacing the
/// question a test did not expect.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct MockPrompter {
    answers: std::collections::VecDeque<bool>,
    choices: std::collections::VecDeque<Vec<usize>>,
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
}

#[cfg(test)]
impl Prompter for MockPrompter {
    fn ask(&mut self, _: &str) -> io::Result<bool> {
        Ok(self
            .answers
            .pop_front()
            .expect("MockPrompter ran out of answers"))
    }

    fn choose(&mut self, _: &str, _: &[String]) -> io::Result<Vec<usize>> {
        Ok(self
            .choices
            .pop_front()
            .expect("MockPrompter ran out of choices"))
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
    fn a_trace_says_so() {
        let mut out = Vec::new();
        trace(&mut out, "x=1").unwrap();
        assert_eq!(plain(&String::from_utf8(out).unwrap()), "trace: x=1\n");
    }

    #[test]
    fn a_mock_answers_in_order() {
        let mut p = MockPrompter::answering([true, false]);
        assert!(p.ask("?").unwrap());
        assert!(!p.ask("?").unwrap());
        p.push_choice([0, 2]);
        assert_eq!(p.choose("?", &[]).unwrap(), [0, 2]);
    }
}
