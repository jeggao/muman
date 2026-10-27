//! `muman set` with a query and nothing to set, on a terminal: the songs
//! the query picks changed field by field, each value shown with where
//! it came from and what the sources offer instead.
//!
//! `set NAME=VALUE` asks for the name and the value up front, and `edit`
//! for TOML, neither with what a tag would override in sight. Here a
//! menu lists each field with the value written now and where it came
//! from, a source, a tag set by hand or the album; a field opened shows
//! each source's offer, cleaned as the song's tags are, and the answers
//! name what each does: keep it, take an offer, type a value, choose
//! among the values, or clear what was set by hand so the sources decide
//! again. A value typed starts as the one written and is checked as
//! `set` checks it before it is taken. Pins are chosen among the song's
//! sources by what was measured of each.
//!
//! What is chosen is kept as the assignments `set` makes, one a field,
//! and nothing is written until it is reviewed: said song by song, then
//! asked about by name, then saved as one change `undo` reverses. A song
//! changed meanwhile, by hand or by another run, has the choices applied
//! again to it as it is now, rather than the work lost.

use std::collections::BTreeMap;
use std::io::Write;
use std::rc::Rc;

use anyhow::{Result, bail};
use unicode_width::UnicodeWidthStr;

use crate::change::{self, Ask, Assign, Read};
use crate::cli::Confirm;
use crate::dirs::Dirs;
use crate::manifest::{Edit, TAG_TEMPLATE};
use crate::reconcile::Resolver;
use crate::resolve::{self, Offered};
use crate::rows;
use crate::source::SourceKey;
use crate::tags::{self, Field};
use crate::ui::{Check, Prompter};

/// How many times choices are applied again to a song list that changed
/// while they were made, before giving up.
const TRIES: usize = 3;

/// The answers to a field that ask more of the user.
const TYPE: &str = "Type a value…";
const CHOOSE: &str = "Choose among the values…";

/// The widest a value shows in the menu, in columns.
const SHOWN: usize = 32;

/// Change the songs `terms` picks field by field on a terminal, then
/// save what was chosen. Returns whether anything changed.
pub fn edit<W: Write>(
    dirs: &Dirs,
    terms: &[String],
    confirm: &Confirm,
    p: &mut dyn Prompter,
    out: &mut W,
) -> Result<bool> {
    let mut read = Read::new(dirs, terms)?;
    let picked = read.pick(confirm.all, "set", Some(&mut *p))?;
    if picked.is_empty() {
        crate::ui::info(out, "Nothing set")?;
        return Ok(false);
    }
    let ids: Vec<SourceKey> = (picked.iter())
        .filter_map(|n| read.manifest.songs[*n].id().cloned())
        .collect();
    let chosen = {
        let m = &read.manifest;
        let resolver = Resolver::new(m, &m.settings, &read.state, &dirs.library)?;
        let mut session = Session {
            read: &read,
            picked: &picked,
            resolver,
            pending: Vec::new(),
        };
        session.run(p, out)?
    };
    let Some(chosen) = chosen.filter(|c| !c.is_empty()) else {
        crate::ui::info(out, "Nothing changed")?;
        return Ok(false);
    };
    for _ in 0..TRIES {
        let picked: Vec<usize> = (ids.iter())
            .filter_map(|id| read.manifest.songs.iter().position(|s| s.id() == Some(id)))
            .collect();
        let songs = change::rewrite(&read, &picked, &chosen, out)?;
        if songs.is_empty() {
            crate::ui::info(out, "Nothing changed: each song is so already")?;
            return Ok(false);
        }
        change::say_changed(songs.len(), out)?;
        let whose = match picked.as_slice() {
            [one] => read.views[*one].name(),
            _ => change::counted(songs.len(), "song"),
        };
        let ask = Ask::YesNo(format!(
            "Save {} to {whose}?",
            change::counted(chosen.len(), "change")
        ));
        if !change::confirmed(confirm, &ask, Some(&mut *p), out)? {
            return Ok(false);
        }
        let expected = read.songs(&picked);
        read.manifest.edit(Edit::Rewrite {
            songs,
            new: Vec::new(),
        });
        if read.manifest.save_if_unchanged(&expected)?.is_empty() {
            return Ok(true);
        }
        read.manifest.discard();
        crate::ui::info(
            out,
            "The song list changed while this was open; the changes are applied to it as it is now",
        )?;
        read = Read::new(dirs, terms)?;
    }
    bail!("the song list kept changing while this ran; nothing was changed, run it again")
}

/// What the menu offers.
enum Item {
    Field(String),
    Pin(&'static str),
    Review,
}

/// The songs being changed, and what was chosen for them so far.
struct Session<'a> {
    read: &'a Read,
    picked: &'a [usize],
    resolver: Resolver<'a>,
    /// One assignment a field or pin, the latest chosen.
    pending: Vec<Assign>,
}

impl Session<'_> {
    /// Ask until the user reviews what was chosen, `None` when they leave
    /// without saving.
    fn run<W: Write>(&mut self, p: &mut dyn Prompter, out: &mut W) -> Result<Option<Vec<Assign>>> {
        let first = self.picked[0];
        let view = &self.read.views[first];
        let head = match self.picked {
            [_] => change::named(view),
            many => format!("{} songs, the first {}", many.len(), change::named(view)),
        };
        crate::ui::info(out, &head)?;
        loop {
            let (items, labels) = self.menu();
            let question = match self.pending.len() {
                0 => "What to change?".to_string(),
                n => format!("What to change? ({} chosen)", change::counted(n, "change")),
            };
            match p.select(&question, &labels)?.map(|i| &items[i]) {
                None if self.pending.is_empty() => return Ok(None),
                None => {
                    let leave = format!(
                        "Leave without saving {}?",
                        change::counted(self.pending.len(), "change")
                    );
                    if p.ask(&leave)? {
                        return Ok(None);
                    }
                }
                Some(Item::Review) => return Ok(Some(std::mem::take(&mut self.pending))),
                Some(Item::Field(name)) => {
                    let name = name.clone();
                    self.field(&name, p, out)?;
                }
                Some(Item::Pin(pin)) => {
                    let pin = *pin;
                    self.pin(pin, p, out)?;
                }
            }
        }
    }

    /// Keep `a` in place of anything chosen for its field before.
    fn choose(&mut self, a: Assign) {
        self.pending.retain(|x| x.name() != a.name());
        self.pending.push(a);
    }

    /// What was chosen for `name`, if anything.
    fn chosen(&self, name: &str) -> Option<&Assign> {
        self.pending.iter().find(|a| a.name() == name)
    }

    /// The values song `n` writes for the field written as `key`, and where
    /// they came from in words.
    fn written(&self, n: usize, key: &str) -> (Vec<String>, String) {
        let Some(r) = self.read.resolved[n].as_ref() else {
            return (Vec::new(), String::new());
        };
        let at = r.plan.tags.iter().position(|(k, _)| k == key);
        at.map_or_else(
            || (Vec::new(), String::new()),
            |at| (r.plan.tags[at].1.clone(), origin(&r.why.tags[at].from)),
        )
    }

    /// Each field and pin, as the menu lists them.
    fn menu(&self) -> (Vec<Item>, Vec<String>) {
        let first = self.picked[0];
        let mut names: Vec<String> = TAG_TEMPLATE.iter().map(ToString::to_string).collect();
        if let Some(r) = self.read.resolved[first].as_ref() {
            for (key, _) in &r.plan.tags {
                let measured = Field::named(key).is_some_and(Field::is_loudness);
                if !measured && !names.iter().any(|n| tags::vorbis_key(n) == *key) {
                    names.push(key.to_ascii_lowercase());
                }
            }
        }
        for a in &self.pending {
            if let Assign::Tag(name, _) = a
                && !names.contains(name)
            {
                names.push(name.clone());
            }
        }
        let wide = names.iter().map(|n| n.width()).max().unwrap_or(0).max(6);
        let mut items = Vec::new();
        let mut labels = Vec::new();
        for name in names {
            let (value, from) = match self.chosen(&name) {
                Some(Assign::Tag(_, v)) if v.is_empty() => (
                    "what the sources offer".to_string(),
                    "✎ to be cleared".to_string(),
                ),
                Some(Assign::Tag(_, v)) => (show(v), "✎ to be set".to_string()),
                _ => self.across(&tags::vorbis_key(&name)),
            };
            labels.push(line(&name, wide, &value, &from));
            items.push(Item::Field(name));
        }
        if let [n] = self.picked {
            let song = &self.read.manifest.songs[*n];
            let plan = self.read.resolved[*n].as_ref().map(|r| &r.plan);
            for pin in ["audio", "cover", "lyrics"] {
                let now = match pin {
                    "audio" => plan.map(|p| p.audio.key.short()),
                    "cover" => plan.and_then(|p| p.cover.as_ref()).map(|c| c.key.short()),
                    _ => plan.and_then(|p| p.lyrics.as_ref()).map(|l| l.key.short()),
                };
                let pinned = match pin {
                    "audio" => song.audio.is_some(),
                    "cover" => song.cover.is_some(),
                    _ => song.lyrics.is_some(),
                };
                let from = match self.chosen(pin) {
                    Some(a) => format!("✎ {}", a.describe(&self.read.views[*n])),
                    None if pinned => "✎ pinned".to_string(),
                    None => "picked by measure".to_string(),
                };
                labels.push(line(
                    pin,
                    wide,
                    &now.unwrap_or_else(|| "none".into()),
                    &from,
                ));
                items.push(Item::Pin(pin));
            }
        }
        labels.push("Review and save".to_string());
        items.push(Item::Review);
        (items, labels)
    }

    /// The values the songs write for `key`, counted where they differ,
    /// and where the first song's came from.
    fn across(&self, key: &str) -> (String, String) {
        let mut counts: Vec<(Vec<String>, usize)> = Vec::new();
        for n in self.picked {
            let (values, _) = self.written(*n, key);
            match counts.iter_mut().find(|(v, _)| *v == values) {
                Some((_, c)) => *c += 1,
                None => counts.push((values, 1)),
            }
        }
        let (_, from) = self.written(self.picked[0], key);
        if let [(values, _)] = counts.as_slice() {
            return (show(values), from);
        }
        let each: Vec<String> = (counts.iter())
            .map(|(v, c)| {
                let v = if v.is_empty() { "none".into() } else { show(v) };
                format!("{v} ×{c}")
            })
            .collect();
        (each.join(" · "), String::new())
    }

    /// Show the field `name` with what is written and offered, and ask
    /// what becomes of it.
    fn field<W: Write>(&mut self, name: &str, p: &mut dyn Prompter, out: &mut W) -> Result<()> {
        let key = tags::vorbis_key(name);
        crate::ui::line(out, name)?;
        let mut written: Vec<Vec<String>> = Vec::new();
        let mut by_hand = false;
        for n in self.picked {
            let (values, from) = self.written(*n, &key);
            by_hand |= self.read.resolved[*n].as_ref().is_some_and(|r| {
                r.why
                    .tags
                    .iter()
                    .any(|w| w.key == key && w.from == "song.tags")
            });
            if !written.contains(&values) {
                crate::ui::line(out, &format!("  written  {}  ← {from}", show(&values)))?;
                written.push(values);
            }
        }
        let offered = self.offered(&key);
        for o in &offered {
            crate::ui::line(out, &format!("  offered  {}  ← {}", show(&o.values), o.key))?;
        }
        let now = written.first().cloned().unwrap_or_default();
        let mut labels = vec!["Keep it as it is".to_string()];
        let mut picks: Vec<Option<Assign>> = vec![None];
        for o in offered.iter().filter(|o| !written.contains(&o.values)) {
            labels.push(format!("Take “{}” from {}", show(&o.values), o.key));
            picks.push(Some(Assign::Tag(name.to_string(), o.values.clone())));
        }
        labels.push(TYPE.into());
        picks.push(None);
        let mut candidates: Vec<String> = Vec::new();
        for v in written
            .iter()
            .chain(offered.iter().map(|o| &o.values))
            .flatten()
        {
            if !candidates.contains(v) {
                candidates.push(v.clone());
            }
        }
        if candidates.len() > 1 {
            labels.push(CHOOSE.into());
            picks.push(None);
        }
        if by_hand {
            labels.push("Clear it, so the sources decide".into());
            picks.push(Some(Assign::Tag(name.to_string(), Vec::new())));
        }
        labels.push("Back".into());
        picks.push(None);
        let Some(i) = p.select(&format!("{name}:"), &labels)? else {
            return Ok(());
        };
        if i == 0 {
            self.pending.retain(|a| a.name() != name);
            return Ok(());
        }
        if let Some(a) = picks[i].clone() {
            self.choose(a);
        } else if labels[i] == TYPE {
            let named = name.to_string();
            let check: Check = Rc::new(move |typed: &str| {
                split(typed).iter().find_map(|v| tags::bad_value(&named, v))
            });
            let help = "several values are parted by \"; \"; enter sets it by hand, esc goes back";
            if let Some(typed) = p.text_with(&format!("{name}:"), help, &now.join("; "), check)? {
                self.choose(Assign::Tag(name.to_string(), split(&typed)));
            }
        } else if labels[i] == CHOOSE {
            let ticked: Vec<usize> = (0..candidates.len())
                .filter(|c| now.contains(&candidates[*c]))
                .collect();
            let question = format!("Which {name}?");
            let mut values: Vec<String> = (p.choose(&question, &candidates, &ticked)?.into_iter())
                .filter_map(|c| candidates.get(c).cloned())
                .collect();
            let question = format!("Add another {name}:");
            while let Some(more) = p.text(&question, "enter on nothing goes on")? {
                if let Some(why) = tags::bad_value(name, more.trim()) {
                    crate::ui::warning(out, &why)?;
                    continue;
                }
                values.push(more.trim().to_string());
            }
            self.choose(Assign::Tag(name.to_string(), values));
        }
        Ok(())
    }

    /// What each source of a lone song offers for the field written as
    /// `key`, each value once; nothing for several songs.
    fn offered(&self, key: &str) -> Vec<Offered> {
        let [n] = self.picked else {
            return Vec::new();
        };
        let Some(field) = Field::named(key) else {
            return Vec::new();
        };
        let song = &self.read.manifest.songs[*n];
        let mut offers: BTreeMap<Field, Vec<Offered>> =
            resolve::offers(&self.resolver.input(song, None));
        let mut out: Vec<Offered> = Vec::new();
        for o in offers.remove(&field).unwrap_or_default() {
            if !o.values.is_empty() && !out.iter().any(|x| x.values == o.values) {
                out.push(o);
            }
        }
        out
    }

    /// Ask which of the lone song's sources `pin` takes, by what was
    /// measured of each.
    fn pin<W: Write>(
        &mut self,
        pin: &'static str,
        p: &mut dyn Prompter,
        out: &mut W,
    ) -> Result<()> {
        let [n] = self.picked else {
            return Ok(());
        };
        let song = &self.read.manifest.songs[*n];
        let facts = &self.read.state.facts;
        let mut labels = vec!["Pick by measure".to_string()];
        let mut picks: Vec<Option<Assign>> = vec![Some(Assign::Pin(pin, None))];
        for key in &song.sources {
            let Some(f) = facts.get(key) else {
                continue;
            };
            let what = match pin {
                "audio" => f.audio.as_ref().map(|a| {
                    let length = f
                        .length()
                        .map(|s| rows::minutes((s * 1000.0).round() as i64));
                    format!(
                        "{}  {}",
                        rows::audio(a, f.length()),
                        length.unwrap_or_default()
                    )
                }),
                "cover" => (f.covers.first()).map(|c| {
                    c.quality
                        .as_ref()
                        .map_or_else(String::new, |q| format!("{}×{}", q.width, q.height))
                }),
                _ => (f.lyrics.first())
                    .map(|l| if l.timing.is_some() { "timed" } else { "plain" }.to_string()),
            };
            if let Some(what) = what {
                labels.push(format!("Pin {key}  {what}").trim_end().to_string());
                picks.push(Some(Assign::Pin(pin, Some(key.to_string()))));
            }
        }
        if pin == "lyrics" {
            labels.push("No lyrics".into());
            picks.push(Some(Assign::Pin(pin, Some("false".into()))));
        }
        labels.push("Back".into());
        picks.push(None);
        crate::ui::line(out, pin)?;
        if let Some(Some(a)) = p
            .select(&format!("Which source gives the {pin}?"), &labels)?
            .and_then(|i| picks.get(i).cloned())
        {
            self.choose(a);
        }
        Ok(())
    }
}

/// Where a written tag came from, in words.
fn origin(from: &str) -> String {
    match from {
        "song.tags" => "✎ set by hand".to_string(),
        "album.tags" => "the album's tags".to_string(),
        other => other.to_string(),
    }
}

/// `values` on one line, cut short.
fn show(values: &[String]) -> String {
    if values.is_empty() {
        return "none".to_string();
    }
    let joined = crate::ui::printable(&values.join("; ")).replace(['\n', '\t'], " ");
    rows::cut(&joined, SHOWN)
}

/// A menu row: the name padded to `wide`, the value, where it came from.
fn line(name: &str, wide: usize, value: &str, from: &str) -> String {
    let pad = " ".repeat(wide.saturating_sub(name.width()));
    let gap = " ".repeat(SHOWN.saturating_sub(value.width()));
    format!("{name}{pad}  {value}{gap}  {from}")
        .trim_end()
        .to_string()
}

/// The values typed, parted by ";".
fn split(typed: &str) -> Vec<String> {
    typed
        .split(';')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_typed_are_parted_by_semicolons() {
        assert_eq!(split("Folk; Acoustic ;"), ["Folk", "Acoustic"]);
        assert_eq!(split("  "), Vec::<String>::new());
        assert_eq!(origin("song.tags"), "✎ set by hand");
        assert_eq!(show(&[]), "none");
    }
}
