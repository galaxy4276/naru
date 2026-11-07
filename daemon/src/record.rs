//! What the 기록 and 증명 rail panels read: a session's requests with the agent's last reply to
//! each, and the decision log in a pane's folder.

use std::time::UNIX_EPOCH;

use serde::Serialize;

use crate::core::Daemon;
use crate::state::PaneId;
use crate::transcript::{self, Turn};

/// Only the end of a long transcript is read; older requests drop off the list.
const RECORD_TAIL: u64 = 4 << 20;

#[derive(Serialize, Debug, PartialEq)]
pub struct Record {
    /// The transcript's modification time in Unix milliseconds; the screen sends it back to
    /// skip a reread of an unchanged file.
    pub mtime: Option<u64>,
    /// Requests oldest first; absent when the file is unchanged since `mtime`.
    pub turns: Option<Vec<Turn>>,
    /// The transcript was longer than the part read.
    pub cut: bool,
    /// Why there is nothing to show.
    pub reason: Option<&'static str>,
}

impl Record {
    fn none(reason: &'static str) -> Self {
        Record {
            mtime: None,
            turns: None,
            cut: false,
            reason: Some(reason),
        }
    }
}

impl Daemon {
    // ponytail: reads up to 4 MiB on the socket task every poll; cache turns per path if it shows.
    pub fn record(&self, pane: PaneId, known: Option<u64>) -> Record {
        let session = {
            let core = self.core();
            core.workspace
                .panes
                .get(&pane)
                .and_then(|record| record.session.clone())
        };
        let Some(session) = session else {
            return Record::none("에이전트 세션이 아니에요");
        };
        if session.remote {
            return Record::none("원격 노드의 기록은 읽지 못해요");
        }
        if !transcript::readable(session.agent) {
            return Record::none("Claude·Codex 세션만 기록을 보여 줘요");
        }
        let Some(path) = transcript::locate(
            session.agent,
            session.transcript_path.as_deref(),
            session.session_id.as_deref(),
            &self.home,
        ) else {
            return Record::none("아직 대화 기록 파일이 없어요");
        };
        let mtime = modified_ms(&path);
        if mtime.is_some() && mtime == known {
            return Record {
                mtime,
                turns: None,
                cut: false,
                reason: None,
            };
        }
        match transcript::read_tail(&path, RECORD_TAIL) {
            Ok((text, cut)) => Record {
                mtime,
                turns: Some(transcript::turns(session.agent, &text)),
                cut,
                reason: None,
            },
            Err(_) => Record::none("대화 기록 파일을 읽지 못했어요"),
        }
    }
}

/// Largest decision log the proof panel reads.
const DECISIONS_LIMIT: u64 = 8 << 20;

/// The decision log pstack's show-me-your-work keeps in a pane's folder.
#[derive(Serialize, Debug, PartialEq)]
pub struct Decisions {
    pub path: Option<String>,
    pub mtime: Option<u64>,
    /// The log's text; absent when unchanged since `mtime` or when there is no log.
    pub text: Option<String>,
}

fn modified_ms(path: &std::path::Path) -> Option<u64> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis() as u64)
}

/// `decisions.tsv` or the `.audit/*.tsv` in `dir` that changed last.
fn newest_decision_log(dir: &std::path::Path) -> Option<(std::path::PathBuf, u64)> {
    let audit = std::fs::read_dir(dir.join(".audit"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "tsv"));
    std::iter::once(dir.join("decisions.tsv"))
        .chain(audit)
        .filter_map(|path| Some((path.clone(), modified_ms(&path)?)))
        .max_by_key(|(_, modified)| *modified)
}

impl Daemon {
    pub fn decisions(&self, pane: PaneId, known: Option<u64>) -> Decisions {
        let cwd = {
            let core = self.core();
            core.workspace
                .panes
                .get(&pane)
                .filter(|record| record.node.is_none())
                .and_then(|record| record.cwd.clone())
        };
        let Some((path, mtime)) = cwd.and_then(|dir| newest_decision_log(dir.as_ref())) else {
            return Decisions {
                path: None,
                mtime: None,
                text: None,
            };
        };
        let text = (Some(mtime) != known)
            .then(|| {
                use std::io::Read;
                let mut text = String::new();
                std::fs::File::open(&path)
                    .and_then(|file| file.take(DECISIONS_LIMIT).read_to_string(&mut text))
                    .ok()
                    .map(|_| text)
            })
            .flatten();
        Decisions {
            path: Some(path.display().to_string()),
            mtime: Some(mtime),
            text,
        }
    }
}
