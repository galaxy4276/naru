//! A pane's screen as text, kept by feeding its output through alacritty_terminal: what
//! `naru agent read` prints and what an agent's input line holds before a letter is typed in.

use alacritty_terminal::Term;
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, TermMode};
use alacritty_terminal::vte::ansi::Processor;

/// Lines kept above the screen: `naru agent read` prints at most this many.
pub const HISTORY: usize = 2000;
/// The prompt marks Claude Code (`>`, `❯`) and Codex (`›`) start their input line with.
const PROMPTS: [char; 3] = ['>', '❯', '›'];
/// How many non-blank rows from the bottom are searched for the input line.
const SCAN_ROWS: usize = 24;

struct Size {
    columns: usize,
    lines: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

pub struct Screen {
    term: Term<VoidListener>,
    parser: Processor,
}

/// What an agent's input line shows.
#[derive(Debug, PartialEq, Eq)]
pub enum Draft {
    /// No input line on screen: a menu, a prompt of the agent's own, or an unknown layout.
    Unseen,
    Empty,
    /// A letter Naru typed, or a paste the agent folded into a `[Pasted …]` stand-in.
    Letter,
    /// Text someone else typed.
    Other,
}

impl Screen {
    pub fn new(columns: u16, lines: u16) -> Screen {
        let config = Config {
            scrolling_history: HISTORY,
            ..Config::default()
        };
        Screen {
            term: Term::new(config, &size(columns, lines), VoidListener),
            parser: Processor::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, columns: u16, lines: u16) {
        self.term.resize(size(columns, lines));
    }

    pub fn bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    fn alt_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    /// The row's characters; dim cells (an input's placeholder) blanked when `blank_dim`.
    fn row(&self, line: Line, blank_dim: bool) -> (String, bool) {
        let row = &self.term.grid()[line];
        let columns = self.term.grid().columns();
        let text = (0..columns)
            .map(|column| &row[Column(column)])
            .filter(|cell| {
                !cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            })
            .map(|cell| {
                if blank_dim && cell.flags.contains(Flags::DIM) {
                    ' '
                } else {
                    cell.c
                }
            })
            .collect::<String>();
        let wraps = row[Column(columns - 1)].flags.contains(Flags::WRAPLINE);
        (text, wraps)
    }

    fn lines(&self) -> impl Iterator<Item = Line> {
        let grid = self.term.grid();
        // The alternate screen keeps no history.
        let top = if self.alt_screen() {
            Line(0)
        } else {
            grid.topmost_line()
        };
        (top.0..=grid.bottommost_line().0).map(Line)
    }

    /// The last `count` lines the pane shows, wrapped rows joined, blank rows at the bottom left
    /// out.
    pub fn last_lines(&self, count: usize) -> String {
        let mut lines = vec![String::new()];
        for line in self.lines() {
            let (text, wraps) = self.row(line, false);
            let current = lines.last_mut().expect("one line at least");
            if wraps {
                current.push_str(&text);
            } else {
                current.push_str(text.trim_end());
                lines.push(String::new());
            }
        }
        while lines.last().is_some_and(|line| line.trim().is_empty()) {
            lines.pop();
        }
        lines[lines.len().saturating_sub(count)..].join("\n")
    }

    /// What the bottom-most agent input line on screen holds.
    pub fn draft(&self) -> Draft {
        let mut rows = Vec::new();
        for line in self.lines().collect::<Vec<_>>().into_iter().rev() {
            let (text, _) = self.row(line, true);
            let text = text.trim_end().to_owned();
            if text.is_empty() && rows.is_empty() {
                continue;
            }
            rows.push(text);
            if rows.len() >= SCAN_ROWS {
                break;
            }
        }
        rows.reverse();
        read_draft(&rows)
    }
}

fn size(columns: u16, lines: u16) -> Size {
    Size {
        columns: usize::from(columns.max(2)),
        lines: usize::from(lines.max(1)),
    }
}

/// Reads the bottom-most input line of `rows` (top to bottom, placeholders already blanked).
pub fn read_draft(rows: &[String]) -> Draft {
    let Some(text) = rows.iter().rev().find_map(|row| {
        let line = row.trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '│' | '┃'));
        let rest = line.strip_prefix(PROMPTS)?;
        (rest.is_empty() || rest.starts_with(char::is_whitespace))
            .then(|| rest.trim().trim_end_matches(['│', '┃']).trim())
    }) else {
        return Draft::Unseen;
    };
    if text.is_empty() {
        Draft::Empty
    } else if text.starts_with("[Naru letter") || text.starts_with("[Pasted") {
        Draft::Letter
    } else {
        Draft::Other
    }
}

#[cfg(test)]
#[path = "grid_tests.rs"]
mod tests;
