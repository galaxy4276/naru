//! Underlines and figures the reader puts on a book's translated pages. A figure is drawn by
//! claude with the user's diagram-design skill and kept as an SVG next to the book.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, ensure};
use base64::Engine;
use serde::{Deserialize, Serialize};

use super::{BASE64, Book, MAX_QUOTE, STORE, book, dir, now_ms};
use crate::core::Daemon;
use crate::ws::ServerMsg;

const FIGURE_MODEL: &str = "claude-sonnet-5-5";
const FIGURE_TIMEOUT: Duration = Duration::from_secs(900);
/// Text kept on each side of an underline to find it again, in characters.
const ROOM: usize = 64;
const MAX_CONTEXT: usize = 4_000;
const MAX_PROMPT: usize = 500;
const MAX_SVG: u64 = 2 << 20;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    pub id: String,
    pub page: u32,
    /// The underlined text, found again by the text around it.
    pub quote: String,
    pub prefix: String,
    pub suffix: String,
    pub created: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FigureStatus {
    Drawing,
    Done,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Figure {
    pub id: String,
    pub page: u32,
    /// The block of the page the figure follows when its text cannot be found any more.
    pub block: u32,
    /// The selected text the figure explains; it follows the block holding this.
    pub quote: String,
    /// The block the selection was in.
    pub context: String,
    /// What the reader asked for.
    pub prompt: String,
    pub status: FigureStatus,
    pub error: Option<String>,
    pub created: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Marks {
    marks: Vec<Mark>,
    figures: Vec<Figure>,
}

#[derive(Serialize)]
pub struct FigureView {
    #[serde(flatten)]
    figure: Figure,
    /// Base64 of the drawn SVG.
    svg: Option<String>,
}

/// Figures being drawn in this run; one marked drawing but not here stopped with the daemon.
static DRAWING: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn marks_path(root: &Path, id: &str) -> PathBuf {
    dir(root, id).join("marks.json")
}

fn figure_path(root: &Path, id: &str, figure: &str, extension: &str) -> PathBuf {
    dir(root, id)
        .join("figures")
        .join(format!("{figure}.{extension}"))
}

fn load_marks(root: &Path, id: &str) -> anyhow::Result<Marks> {
    let path = marks_path(root, id);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("{} is not a list of underlines", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Marks::default()),
        Err(error) => Err(error).with_context(|| format!("cannot read {}", path.display())),
    }
}

/// Lets `change` edit the book's underlines and figures, under the store lock. An unreadable file
/// is left as it is.
fn edit_marks<T>(
    root: &Path,
    id: &str,
    change: impl FnOnce(&mut Marks) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let _guard = STORE.lock().expect("library lock poisoned");
    ensure!(
        super::pdf_path(root, id).is_file(),
        "서재에 없는 책이에요: {id}"
    );
    let mut marks = load_marks(root, id)?;
    let done = change(&mut marks)?;
    let path = marks_path(root, id);
    crate::suggestions::write_private(&path, &serde_json::to_vec_pretty(&marks)?)
        .with_context(|| format!("cannot save {}", path.display()))?;
    Ok(done)
}

fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// The last `max` characters.
fn tail(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    chars[chars.len().saturating_sub(max)..].iter().collect()
}

fn checked_page(book: &Book, page: u32) -> anyhow::Result<()> {
    ensure!(
        (1..=book.pages).contains(&page),
        "{}: {page}쪽은 없어요",
        book.title
    );
    Ok(())
}

fn figure_prompt(title: &str, figure: &Figure, skill: &Path) -> String {
    format!(
        "You draw one figure for a reader of the book \"{title}\", page {page}, in the Naru 서재 \
         (PDF library).\n\
         Follow the diagram-design skill: read {skill}/SKILL.md, then only the references it points \
         to for the type you pick.\n\
         - Skip its first-time setup gate: keep the current style guide as it is and never ask \
         anything; nobody can answer.\n\
         - Pick the visual type that fits the request. Write every label in Korean.\n\
         - The figure sits between two paragraphs of a page, so keep it compact: about 640px wide, \
         at most 9 nodes.\n\n\
         The passage the reader selected:\n<selection>{quote}</selection>\n\n\
         The paragraph it is in:\n<paragraph>{context}</paragraph>\n\n\
         The reader asks: {prompt}\n\n\
         Output only the complete self-contained HTML file of the diagram, starting with \
         <!doctype html>, and nothing else.\n",
        page = figure.page,
        skill = skill.display(),
        quote = figure.quote,
        context = figure.context,
        prompt = figure.prompt,
    )
}

/// The HTML document in claude's answer, from its doctype or `<html>` to the last `</html>`.
fn html_of(answer: &str) -> Option<&str> {
    let lower = answer.to_ascii_lowercase();
    let start = lower
        .find("<!doctype html")
        .or_else(|| lower.find("<html"))?;
    let end = lower.rfind("</html>")? + "</html>".len();
    (end > start).then(|| &answer[start..end])
}

/// Has claude draw the figure and turns its HTML into a standalone SVG next to the book.
fn draw(home: &Path, root: &Path, book: &Book, figure: &Figure) -> anyhow::Result<()> {
    let claude = crate::titles::claude_path(home)
        .context("그림을 그릴 claude가 ~/.local/bin/claude에 없어요")?;
    let skill = home.join(".claude/skills/diagram-design");
    ensure!(
        skill.join("SKILL.md").is_file(),
        "diagram-design 스킬이 {}에 없어요",
        skill.display()
    );
    let answer = crate::titles::ask_claude_with(
        &claude,
        FIGURE_MODEL,
        &figure_prompt(&book.title, figure, &skill),
        FIGURE_TIMEOUT,
        &[
            OsStr::new("--allowedTools"),
            OsStr::new("Read,Glob,Grep"),
            OsStr::new("--add-dir"),
            skill.as_os_str(),
        ],
    )?;
    let html = html_of(&answer).context("claude가 그림 HTML을 돌려주지 않았어요")?;
    let html_path = figure_path(root, &book.id, &figure.id, "html");
    crate::suggestions::write_private(&html_path, html.as_bytes())
        .with_context(|| format!("cannot save {}", html_path.display()))?;
    let svg_path = figure_path(root, &book.id, &figure.id, "svg");
    let output = Command::new("python3")
        .arg(skill.join("scripts/export_svg.py"))
        .arg(&html_path)
        .arg(&svg_path)
        .arg("--system-fonts")
        .output()
        .context("SVG로 바꿀 python3를 실행하지 못했어요")?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    ensure!(
        output.status.success(),
        "그림을 SVG로 바꾸지 못했어요: {}",
        stderr.lines().last().unwrap_or("")
    );
    let size = std::fs::metadata(&svg_path)
        .with_context(|| format!("cannot read {}", svg_path.display()))?
        .len();
    ensure!(size <= MAX_SVG, "그림이 너무 커요 ({size} bytes)");
    Ok(())
}

impl Daemon {
    /// The book's underlines and figures, for `library_marks` messages.
    pub fn library_marks_message(&self, id: &str) -> anyhow::Result<Arc<str>> {
        let root = self.library_root();
        let marks = {
            let _guard = STORE.lock().expect("library lock poisoned");
            load_marks(&root, id)?
        };
        let drawing = DRAWING.lock().expect("library drawing lock").clone();
        let figures = marks
            .figures
            .into_iter()
            .map(|mut figure| {
                if figure.status == FigureStatus::Drawing && !drawing.contains(&figure.id) {
                    figure.status = FigureStatus::Failed;
                    figure.error = Some("Naru가 다시 켜지면서 그리기가 멈췄어요".to_owned());
                }
                let svg = (figure.status == FigureStatus::Done)
                    .then(|| std::fs::read(figure_path(&root, id, &figure.id, "svg")).ok())
                    .flatten()
                    .map(|bytes| BASE64.encode(bytes));
                FigureView { figure, svg }
            })
            .collect();
        Ok(ServerMsg::LibraryMarks {
            book: id.to_owned(),
            marks: marks.marks,
            figures,
        }
        .text())
    }

    fn library_marks_changed(&self, id: &str) {
        let message = match self.library_marks_message(id) {
            Ok(message) => message,
            Err(error) => ServerMsg::LibraryFailure {
                message: format!("{error:#}"),
            }
            .text(),
        };
        let _ = self.events.send(message);
    }

    pub fn library_mark(
        &self,
        id: &str,
        page: u32,
        quote: &str,
        prefix: &str,
        suffix: &str,
    ) -> anyhow::Result<()> {
        let root = self.library_root();
        checked_page(&book(&root, id)?, page)?;
        ensure!(!quote.trim().is_empty(), "밑줄 그을 글을 골라 주세요");
        ensure!(
            quote.chars().count() <= MAX_QUOTE,
            "밑줄은 {MAX_QUOTE}자까지 그을 수 있어요"
        );
        edit_marks(&root, id, |marks| {
            marks.marks.push(Mark {
                id: uuid::Uuid::new_v4().simple().to_string(),
                page,
                quote: quote.to_owned(),
                prefix: tail(prefix, ROOM),
                suffix: clip(suffix, ROOM),
                created: now_ms(),
            });
            Ok(())
        })?;
        self.library_marks_changed(id);
        Ok(())
    }

    pub fn library_unmark(&self, id: &str, mark: &str) -> anyhow::Result<()> {
        edit_marks(&self.library_root(), id, |marks| {
            marks.marks.retain(|each| each.id != mark);
            Ok(())
        })?;
        self.library_marks_changed(id);
        Ok(())
    }

    /// Puts a figure to be drawn after the selection's block and draws it in the background.
    pub fn library_draw(
        self: &Arc<Self>,
        id: &str,
        page: u32,
        block: u32,
        quote: &str,
        context: &str,
        prompt: &str,
    ) -> anyhow::Result<()> {
        let root = self.library_root();
        checked_page(&book(&root, id)?, page)?;
        ensure!(!quote.trim().is_empty(), "그림으로 그릴 글을 골라 주세요");
        ensure!(!prompt.trim().is_empty(), "어떤 그림을 그릴지 적어 주세요");
        let figure = Figure {
            id: uuid::Uuid::new_v4().simple().to_string(),
            page,
            block,
            quote: clip(quote, MAX_QUOTE),
            context: clip(context, MAX_CONTEXT),
            prompt: clip(prompt.trim(), MAX_PROMPT),
            status: FigureStatus::Drawing,
            error: None,
            created: now_ms(),
        };
        edit_marks(&root, id, |marks| {
            marks.figures.push(figure.clone());
            Ok(())
        })?;
        self.start_drawing(id, figure);
        Ok(())
    }

    pub fn library_redraw(self: &Arc<Self>, id: &str, figure: &str) -> anyhow::Result<()> {
        ensure!(
            !DRAWING
                .lock()
                .expect("library drawing lock")
                .contains(figure),
            "이미 그리고 있어요"
        );
        let found = edit_marks(&self.library_root(), id, |marks| {
            let found = marks
                .figures
                .iter_mut()
                .find(|each| each.id == figure)
                .context("이미 빠진 그림이에요")?;
            found.status = FigureStatus::Drawing;
            found.error = None;
            Ok(found.clone())
        })?;
        self.start_drawing(id, found);
        Ok(())
    }

    pub fn library_undraw(&self, id: &str, figure: &str) -> anyhow::Result<()> {
        let root = self.library_root();
        edit_marks(&root, id, |marks| {
            marks.figures.retain(|each| each.id != figure);
            Ok(())
        })?;
        // The id was checked against nothing; only names of the book's own figures are removed.
        if figure.chars().all(|c| c.is_ascii_hexdigit()) {
            for extension in ["html", "svg"] {
                let _ = std::fs::remove_file(figure_path(&root, id, figure, extension));
            }
        }
        self.library_marks_changed(id);
        Ok(())
    }

    fn start_drawing(self: &Arc<Self>, id: &str, figure: Figure) {
        DRAWING
            .lock()
            .expect("library drawing lock")
            .insert(figure.id.clone());
        self.library_marks_changed(id);
        let daemon = self.clone();
        let id = id.to_owned();
        std::thread::spawn(move || {
            let root = daemon.library_root();
            let result =
                book(&root, &id).and_then(|book| draw(&daemon.home, &root, &book, &figure));
            let kept = edit_marks(&root, &id, |marks| {
                let Some(found) = marks.figures.iter_mut().find(|each| each.id == figure.id) else {
                    return Ok(false);
                };
                match &result {
                    Ok(()) => found.status = FigureStatus::Done,
                    Err(error) => {
                        found.status = FigureStatus::Failed;
                        found.error = Some(format!("{error:#}"));
                    }
                }
                Ok(true)
            });
            if !matches!(kept, Ok(true)) {
                // Taken out while it was drawn.
                for extension in ["html", "svg"] {
                    let _ = std::fs::remove_file(figure_path(&root, &id, &figure.id, extension));
                }
            }
            DRAWING
                .lock()
                .expect("library drawing lock")
                .remove(&figure.id);
            daemon.library_marks_changed(&id);
        });
    }
}

#[cfg(test)]
#[path = "marks_tests.rs"]
mod tests;
