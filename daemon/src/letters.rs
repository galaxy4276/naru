//! The record of letters agents and the user write each other: numbered (l7) and kept for a week
//! with the agents' short ids in `~/.naru/team.json`, so a restart keeps both and a reply names
//! the letter it answers.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::state::PaneId;
use crate::team::Kind;

/// How long a letter stays on record.
const KEEP: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// The most letters kept between two parties, newest first.
const PER_PAIR: usize = 200;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Waiting for its recipient to stop working.
    Held,
    /// Typed into the recipient's input, not yet taken in.
    Typed,
    Taken,
    /// Typed in and never taken, even after a second try.
    Untaken,
    Dropped,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Record {
    pub id: u32,
    /// `None` is the user.
    pub from: Option<PaneId>,
    pub to: PaneId,
    /// The parties' ids and titles when it was sent, so the record still reads once their panes
    /// close.
    pub from_name: String,
    pub to_name: String,
    pub from_title: String,
    pub to_title: String,
    pub kind: Kind,
    pub text: String,
    /// Unix seconds.
    pub at: u64,
    pub reply_to: Option<u32>,
    pub state: State,
    /// Why it was dropped.
    pub why: Option<String>,
    pub state_at: u64,
}

/// What `~/.naru/team.json` holds.
#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Saved {
    pub next_agent: u32,
    pub agents: BTreeMap<PaneId, u32>,
    pub next_letter: u32,
    pub letters: Vec<Record>,
}

/// A record as screens get it.
#[derive(Serialize)]
pub struct View<'a> {
    #[serde(flatten)]
    record: &'a Record,
    /// The first letter that replied to it.
    answered_by: Option<u32>,
}

#[derive(Default)]
pub struct Letters {
    next: u32,
    all: Vec<Record>,
}

impl Letters {
    pub fn new(next: u32, all: Vec<Record>) -> Self {
        let next = all.iter().map(|record| record.id).fold(next, u32::max);
        let mut letters = Letters { next, all };
        letters.prune(unix_now());
        letters
    }

    pub fn next(&self) -> u32 {
        self.next
    }

    pub fn all(&self) -> &[Record] {
        &self.all
    }

    /// Numbers `record` and keeps it, returning its number.
    pub fn add(&mut self, mut record: Record) -> u32 {
        self.next += 1;
        record.id = self.next;
        self.all.push(record);
        self.prune(unix_now());
        self.next
    }

    pub fn get(&self, id: u32) -> Option<&Record> {
        self.all.iter().find(|record| record.id == id)
    }

    /// `false` when it was already so, or is no longer on record.
    pub fn set(&mut self, id: u32, state: State, why: Option<&str>) -> bool {
        let Some(record) = self.all.iter_mut().find(|record| record.id == id) else {
            return false;
        };
        let why = why.map(str::to_owned);
        if record.state == state && record.why == why {
            return false;
        }
        record.state = state;
        record.why = why;
        record.state_at = unix_now();
        true
    }

    pub fn views(&self) -> Vec<View<'_>> {
        let mut answers: HashMap<u32, u32> = HashMap::new();
        for record in &self.all {
            if let Some(original) = record.reply_to {
                answers.entry(original).or_insert(record.id);
            }
        }
        self.all
            .iter()
            .map(|record| View {
                record,
                answered_by: answers.get(&record.id).copied(),
            })
            .collect()
    }

    fn prune(&mut self, now: u64) {
        let oldest = now.saturating_sub(KEEP.as_secs());
        self.all.retain(|record| record.at >= oldest);
        let mut per_pair: HashMap<(Option<PaneId>, Option<PaneId>), usize> = HashMap::new();
        let mut keep = vec![true; self.all.len()];
        for (index, record) in self.all.iter().enumerate().rev() {
            let (a, b) = (record.from, Some(record.to));
            let count = per_pair.entry((a.min(b), a.max(b))).or_default();
            *count += 1;
            keep[index] = *count <= PER_PAIR;
        }
        let mut keep = keep.into_iter();
        self.all.retain(|_| keep.next().unwrap_or(true));
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn path(naru_dir: &Path) -> PathBuf {
    naru_dir.join("team.json")
}

/// The saved ids and letters; an unreadable file is moved aside rather than overwritten.
pub fn load(naru_dir: &Path) -> Saved {
    let path = path(naru_dir);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            let aside = path.with_extension("json.bad");
            eprintln!(
                "cannot parse {} ({error}), moved to {}",
                path.display(),
                aside.display()
            );
            let _ = std::fs::rename(&path, &aside);
            Saved::default()
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Saved::default(),
        Err(error) => {
            eprintln!("cannot read {}: {error}", path.display());
            Saved::default()
        }
    }
}

pub fn save(naru_dir: &Path, bytes: &[u8]) {
    let path = path(naru_dir);
    if let Err(error) = crate::suggestions::write_private(&path, bytes) {
        eprintln!("cannot write {}: {error}", path.display());
    }
}

#[cfg(test)]
#[path = "letters_tests.rs"]
mod tests;
