//! The PDF 서재: books Naru keeps a copy of, the page each was left at, page translations, and the
//! claude session per book that discusses it and keeps its note in the Obsidian vault.

mod marks;
pub mod queue;

pub use marks::{FigureView, Mark};

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::agent::Agent;
use crate::core::Daemon;
use crate::state::PaneId;
use crate::ws::ServerMsg;

/// Largest PDF the 서재 takes.
pub const MAX_BOOK: usize = 150 << 20;
const MAX_COVER: usize = 2 << 20;
/// Page text sent for translation, in characters; a page of prose is a few thousand.
const MAX_PAGE_TEXT: usize = 20_000;
const TRANSLATE_MODEL: &str = "claude-sonnet-5-5";
const TRANSLATE_TIMEOUT: Duration = Duration::from_secs(240);
/// Longest passage or question taken from the reader, in characters.
const MAX_QUOTE: usize = 2_000;
/// How much of the page before a translation sees, in characters.
const BEFORE_CHARS: usize = 400;
const NOTE_FOLDER: &str = "learning/서재";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Book {
    pub id: String,
    pub title: String,
    pub pages: u32,
    /// The page the reader last showed, from 1.
    pub page: u32,
    /// Unix milliseconds.
    pub added: u64,
    /// When the reader last opened it, in Unix milliseconds.
    pub opened: Option<u64>,
    /// The claude session that discusses the book.
    pub session: Option<String>,
    /// SHA-256 of the PDF, so the same file is not shelved twice.
    pub sha256: String,
    /// Pages with text to translate, once the screen has sent the book's text.
    #[serde(default)]
    pub translatable: Option<u32>,
}

#[derive(Default, Serialize, Deserialize)]
struct Store {
    books: Vec<Book>,
}

/// A book as the screen shows it.
#[derive(Serialize)]
pub struct BookView {
    #[serde(flatten)]
    book: Book,
    /// Base64 JPEG of the first page.
    cover: Option<String>,
    note: String,
    note_exists: bool,
    /// Pages translated so far.
    translated: u32,
}

static STORE: Mutex<()> = Mutex::new(());
/// Translations under way, by book and page.
static TRANSLATING: Mutex<BTreeSet<(String, u32)>> = Mutex::new(BTreeSet::new());
/// The pane each book's session runs in, for sessions started since the daemon did.
static PANES: Mutex<Option<HashMap<String, PaneId>>> = Mutex::new(None);

pub fn root(state_file: &Path) -> PathBuf {
    state_file.with_file_name("library")
}

fn dir(root: &Path, id: &str) -> PathBuf {
    root.join(id)
}

pub fn pdf_path(root: &Path, id: &str) -> PathBuf {
    dir(root, id).join("book.pdf")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

fn load(root: &Path) -> anyhow::Result<Store> {
    let path = root.join("library.json");
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("{} is not a 서재 list", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(error) => Err(error).with_context(|| format!("cannot read {}", path.display())),
    }
}

/// Loads the list, lets `change` edit it and saves it, under the store lock. An unreadable list is
/// left as it is.
fn edit<T>(root: &Path, change: impl FnOnce(&mut Store) -> anyhow::Result<T>) -> anyhow::Result<T> {
    let _guard = STORE.lock().expect("library lock poisoned");
    let mut store = load(root)?;
    let done = change(&mut store)?;
    let path = root.join("library.json");
    crate::suggestions::write_private(&path, &serde_json::to_vec_pretty(&store)?)
        .with_context(|| format!("cannot save {}", path.display()))?;
    Ok(done)
}

pub fn book(root: &Path, id: &str) -> anyhow::Result<Book> {
    let _guard = STORE.lock().expect("library lock poisoned");
    load(root)?
        .books
        .into_iter()
        .find(|book| book.id == id)
        .with_context(|| format!("서재에 없는 책이에요: {id}"))
}

/// The title a file name gives: its name without `.pdf`.
fn title_of(name: &str) -> anyhow::Result<String> {
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name).trim();
    let title = match name.len().checked_sub(4) {
        Some(at) if name.is_char_boundary(at) && name[at..].eq_ignore_ascii_case(".pdf") => {
            &name[..at]
        }
        _ => name,
    }
    .trim();
    ensure!(!title.is_empty(), "파일 이름이 비어 있어요");
    Ok(title.to_owned())
}

/// Copies a PDF into the 서재 with its first page as the cover.
pub fn add(root: &Path, name: &str, pages: u32, cover: &[u8], pdf: &[u8]) -> anyhow::Result<Book> {
    let title = title_of(name)?;
    ensure!(pdf.starts_with(b"%PDF-"), "{title}: PDF 파일이 아니에요");
    ensure!(pdf.len() <= MAX_BOOK, "{title}: 150MB보다 커서 넣지 못해요");
    ensure!(pages >= 1, "{title}: 쪽이 없어요");
    ensure!(
        cover.starts_with(&[0xff, 0xd8, 0xff]) && cover.len() <= MAX_COVER,
        "{title}: 표지 이미지가 JPEG가 아니에요"
    );
    let sha256: String = Sha256::digest(pdf)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    edit(root, |store| {
        if let Some(same) = store.books.iter().find(|book| book.sha256 == sha256) {
            bail!("이미 서재에 있어요: {}", same.title);
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let folder = dir(root, &id);
        let written = crate::suggestions::write_private(&pdf_path(root, &id), pdf)
            .and_then(|()| crate::suggestions::write_private(&folder.join("cover.jpg"), cover))
            .and_then(|()| crate::suggestions::write_private(&folder.join("page"), b"1"));
        if let Err(error) = written {
            let _ = std::fs::remove_dir_all(&folder);
            return Err(error)
                .with_context(|| format!("cannot save {title} in {}", folder.display()));
        }
        let book = Book {
            id,
            title,
            pages,
            page: 1,
            added: now_ms(),
            opened: None,
            session: None,
            sha256,
            translatable: None,
        };
        store.books.push(book.clone());
        Ok(book)
    })
}

/// Takes the book off the shelf and deletes Naru's copy; its note in the vault stays.
pub fn remove(root: &Path, id: &str) -> anyhow::Result<()> {
    edit(root, |store| {
        let at = store
            .books
            .iter()
            .position(|book| book.id == id)
            .with_context(|| format!("서재에 없는 책이에요: {id}"))?;
        store.books.remove(at);
        Ok(())
    })?;
    let folder = dir(root, id);
    std::fs::remove_dir_all(&folder).with_context(|| format!("cannot delete {}", folder.display()))
}

/// Remembers where the reader is, and tells the book's session through its `page` file.
pub fn set_page(root: &Path, id: &str, page: u32) -> anyhow::Result<()> {
    edit(root, |store| {
        let book = store
            .books
            .iter_mut()
            .find(|book| book.id == id)
            .with_context(|| format!("서재에 없는 책이에요: {id}"))?;
        ensure!(
            (1..=book.pages).contains(&page),
            "{}: {page}쪽은 없어요",
            book.title
        );
        book.page = page;
        book.opened = Some(now_ms());
        Ok(())
    })?;
    let path = dir(root, id).join("page");
    crate::suggestions::write_private(&path, page.to_string().as_bytes())
        .with_context(|| format!("cannot write {}", path.display()))
}

/// Marks the book as the one read last.
pub fn open(root: &Path, id: &str) -> anyhow::Result<()> {
    edit(root, |store| {
        let book = store
            .books
            .iter_mut()
            .find(|book| book.id == id)
            .with_context(|| format!("서재에 없는 책이에요: {id}"))?;
        book.opened = Some(now_ms());
        Ok(())
    })
}

/// A PDF the user picked on this Mac, refused before reading when it is too large to shelve.
pub fn read_picked(path: &Path) -> anyhow::Result<Vec<u8>> {
    ensure!(
        path.is_absolute(),
        "파일 경로가 올바르지 않아요: {}",
        path.display()
    );
    let size = std::fs::metadata(path)
        .with_context(|| format!("파일을 찾지 못했어요: {}", path.display()))?
        .len();
    ensure!(
        size <= MAX_BOOK as u64,
        "{}: 150MB보다 커서 넣지 못해요",
        path.display()
    );
    std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))
}

fn set_session(root: &Path, id: &str, session: &str) -> anyhow::Result<()> {
    edit(root, |store| {
        let book = store
            .books
            .iter_mut()
            .find(|book| book.id == id)
            .with_context(|| format!("서재에 없는 책이에요: {id}"))?;
        book.session = Some(session.to_owned());
        Ok(())
    })
}

fn translation_path(root: &Path, id: &str, page: u32) -> PathBuf {
    dir(root, id)
        .join("translations")
        .join(format!("{page}.md"))
}

pub fn translation(root: &Path, id: &str, page: u32) -> anyhow::Result<Option<String>> {
    let path = translation_path(root, id, page);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("cannot read {}", path.display())),
    }
}

fn save_translation(root: &Path, id: &str, page: u32, text: &str) -> anyhow::Result<()> {
    // A book taken off the shelf meanwhile must not come back as a folder of translations.
    ensure!(pdf_path(root, id).is_file(), "서재에서 빠진 책이에요: {id}");
    let path = translation_path(root, id, page);
    crate::suggestions::write_private(&path, text.as_bytes())
        .with_context(|| format!("cannot save {}", path.display()))
}

fn translated_pages(root: &Path, id: &str) -> BTreeSet<u32> {
    let Ok(entries) = std::fs::read_dir(dir(root, id).join("translations")) else {
        return BTreeSet::new();
    };
    entries
        .filter_map(|entry| {
            let name = entry.ok()?.file_name();
            name.to_str()?.strip_suffix(".md")?.parse().ok()
        })
        .collect()
}

fn texts_path(root: &Path, id: &str) -> PathBuf {
    dir(root, id).join("texts.json")
}

/// The text of every page, as the screen extracted it, once it has.
fn texts(root: &Path, id: &str) -> anyhow::Result<Option<Vec<String>>> {
    let path = texts_path(root, id);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .with_context(|| format!("{} is not a list of page texts", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("cannot read {}", path.display())),
    }
}

/// Keeps the text of every page, which the background translation works from.
pub fn save_texts(root: &Path, id: &str, texts: Vec<String>) -> anyhow::Result<()> {
    let texts: Vec<String> = texts
        .into_iter()
        .map(|text| text.chars().take(MAX_PAGE_TEXT).collect())
        .collect();
    let translatable = texts.iter().filter(|text| !text.trim().is_empty()).count() as u32;
    edit(root, |store| {
        let book = store
            .books
            .iter_mut()
            .find(|book| book.id == id)
            .with_context(|| format!("서재에 없는 책이에요: {id}"))?;
        ensure!(
            texts.len() == book.pages as usize,
            "{}: 쪽은 {}개인데 글자는 {}쪽 분량이 왔어요",
            book.title,
            book.pages,
            texts.len()
        );
        let path = texts_path(root, id);
        crate::suggestions::write_private(&path, &serde_json::to_vec(&texts)?)
            .with_context(|| format!("cannot save {}", path.display()))?;
        book.translatable = Some(translatable);
        Ok(())
    })
}

fn translate_prompt(title: &str, page: u32, text: &str, before: Option<&str>) -> String {
    let text: String = text.chars().take(MAX_PAGE_TEXT).collect();
    let before = before
        .map(|before| {
            let tail: Vec<char> = before.trim().chars().collect();
            let tail: String = tail[tail.len().saturating_sub(BEFORE_CHARS)..]
                .iter()
                .collect();
            format!(
                "\nThe page before ended like this. It is context for a sentence that runs on; do \
                 not translate it:\n<before>\n{tail}\n</before>\n"
            )
        })
        .unwrap_or_default();
    format!(
        "Translate page {page} of the book \"{title}\" into natural Korean.\n\
         - Keep its structure in markdown: headings as ##, paragraphs, lists.\n\
         - The text was extracted from a PDF: rejoin words and sentences broken across lines, and \
         drop running headers, footers and the page number.\n\
         - Keep names and established English terms where a Korean reader expects them; give the \
         English in parentheses after a key term the first time it appears.\n\
         - Output only the translation, with no preface or notes.\n{before}\n<page>\n{text}\n</page>\n"
    )
}

/// Translates one page with claude and keeps it.
fn translate(
    home: &Path,
    root: &Path,
    book: &Book,
    page: u32,
    text: &str,
    before: Option<&str>,
) -> anyhow::Result<String> {
    let claude =
        crate::titles::claude_path(home).context("번역할 claude가 ~/.local/bin/claude에 없어요")?;
    let prompt = translate_prompt(&book.title, page, text, before);
    let translated =
        crate::titles::ask_claude(&claude, TRANSLATE_MODEL, &prompt, TRANSLATE_TIMEOUT)?;
    let translated = translated.trim().to_owned();
    ensure!(!translated.is_empty(), "번역이 비어 있어요");
    save_translation(root, &book.id, page, &translated)?;
    Ok(translated)
}

/// The note file for a book under the vault's notes folder (설정 → 경로). Characters a file name
/// cannot hold become spaces.
pub fn note_path(vault: &Path, title: &str) -> anyhow::Result<PathBuf> {
    let name: String = title
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|#^[]".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let name: String = name.chars().take(80).collect();
    let name = name.trim().trim_start_matches('.').trim();
    ensure!(
        !name.is_empty(),
        "\"{title}\"로는 노트 파일 이름을 만들 수 없어요"
    );
    Ok(vault
        .join(NOTE_FOLDER)
        .join(format!("{name}.md")))
}

fn task(book: &Book, root: &Path, vault: &Path, note: &Path, today: &str) -> String {
    let pdf = pdf_path(root, &book.id);
    let page = dir(root, &book.id).join("page");
    format!(
        "You are the reading partner for one book in the user's Naru 서재 (PDF library). Speak Korean.\n\n\
         Book: \"{title}\", {pages} pages, PDF at {pdf}. Read pages with the Read tool's `pages` \
         parameter, a few at a time.\n\
         The page the user is looking at right now is the number in {page}. Read that file whenever \
         the user says 이 쪽, 여기, 지금 or does not name a page.\n\n\
         Knowledge note: {note}. What you and the user work out about this book accumulates there.\n\
         - At the start of each conversation, read the note if it exists.\n\
         - When a discussion settles something (a key idea, the user's own application, a \
         disagreement, a question to come back to), update the note. Organize it by chapter, cite \
         pages like (42쪽), and keep the user's own conclusions under \"내 적용\". Merge and rewrite \
         rather than appending duplicates.\n\
         - If the note does not exist, create it starting with this frontmatter:\n\
         ---\ntitle: {title}\nproject: learning\ntype: 학습\nreading: 읽기전\ncreated: {today}\n---\n\
         - If {vault} does not exist, the outside disk is disconnected: tell the user the note could \
         not be saved, keep the points in the conversation until it is back, and never write the note \
         anywhere else.\n\n\
         Now read the note if it exists, then say in two or three lines what it holds so far (or that \
         it is empty) and which page the user is on, and wait for the user.\n",
        title = book.title,
        pages = book.pages,
        pdf = pdf.display(),
        page = page.display(),
        note = note.display(),
        vault = vault.display(),
    )
}

/// A question about a passage as one line for the book's session. Control characters go, so the
/// text can neither leave a bracketed paste nor run as a second command line.
fn ask_text(page: u32, quote: &str, question: &str) -> anyhow::Result<String> {
    let clean = |text: &str, max: usize| -> String {
        let text: String = text
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .take(max)
            .collect();
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    };
    let quote = clean(quote, MAX_QUOTE);
    let question = clean(question, MAX_QUOTE);
    ensure!(!quote.is_empty(), "물어볼 부분을 골라 주세요");
    ensure!(!question.is_empty(), "무엇을 물어볼지 적어 주세요");
    Ok(format!("{page}쪽의 \"{quote}\"에 대해: {question}"))
}

impl Daemon {
    pub fn library_root(&self) -> PathBuf {
        root(&self.state_file)
    }

    /// Every book with its cover, for `library` messages.
    pub fn library_message(&self) -> Arc<str> {
        let root = self.library_root();
        let loaded = {
            let _guard = STORE.lock().expect("library lock poisoned");
            load(&root)
        };
        let (books, error) = match loaded {
            Ok(store) => (store.books, None),
            Err(error) => (Vec::new(), Some(format!("{error:#}"))),
        };
        let vault = crate::paths::library_notes().ok();
        let books = books
            .into_iter()
            .map(|book| {
                let cover = std::fs::read(dir(&root, &book.id).join("cover.jpg"))
                    .ok()
                    .map(|bytes| BASE64.encode(bytes));
                let note = vault.as_deref().and_then(|vault| note_path(vault, &book.title).ok());
                // ponytail: lists each book's translations folder per message; count once if the
                // shelf grows past a few dozen books.
                let translated = translated_pages(&root, &book.id).len() as u32;
                BookView {
                    translated,
                    note_exists: note.as_deref().is_some_and(Path::is_file),
                    note: note
                        .map(|path| path.display().to_string())
                        .unwrap_or_default(),
                    cover,
                    book,
                }
            })
            .collect();
        ServerMsg::Library { books, error }.text()
    }

    pub fn library_changed(&self) {
        let _ = self.events.send(self.library_message());
    }

    /// The live pane of the book's session: the one this run started, or any pane whose claude
    /// reports the session.
    fn book_pane(&self, book: &Book) -> Option<PaneId> {
        let opened = PANES
            .lock()
            .expect("library panes lock")
            .as_ref()
            .and_then(|panes| panes.get(&book.id).copied());
        let core = self.core();
        opened
            .filter(|pane| core.live.contains_key(pane))
            .or_else(|| {
                let session = book.session.as_deref()?;
                core.workspace.panes.iter().find_map(|(pane, record)| {
                    (record.session.as_ref()?.session_id.as_deref() == Some(session))
                        .then_some(*pane)
                })
            })
    }

    /// The pane of the book's claude session, starting it, or resuming it after its pane closed, in
    /// a card of the 책 논의 project.
    pub fn library_discuss(
        self: &Arc<Self>,
        id: &str,
        ask: Option<&str>,
    ) -> anyhow::Result<PaneId> {
        let root = self.library_root();
        let book = book(&root, id)?;
        if let Some(pane) = self.book_pane(&book) {
            return Ok(pane);
        }
        let launcher = {
            let core = self.core();
            crate::conversations::usual_launcher(
                core.workspace
                    .panes
                    .values()
                    .filter_map(|record| record.session.as_ref()),
            )
        };
        // A session whose start never reached claude has no transcript; it starts again under the
        // same id.
        let resume = book.session.as_deref().is_some_and(|session| {
            crate::transcript::locate(Agent::Claude, None, Some(session), &self.home).is_some()
        });
        let session = book
            .session
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        ensure!(
            uuid::Uuid::parse_str(&session).is_ok() && crate::agent::is_plain_word(&session),
            "저장된 세션 id가 이상해요: {session}"
        );
        let command = if resume {
            let resume = Agent::Claude
                .resume_command(launcher.as_deref(), &session)
                .with_context(|| format!("cannot resume session {session}"))?;
            match ask {
                Some(ask) => format!("{resume} {}", shell_words::quote(ask)),
                None => resume,
            }
        } else {
            let vault = crate::paths::library_notes().map_err(anyhow::Error::msg)?;
            let note = note_path(&vault, &book.title)?;
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let path = dir(&root, id).join("task.md");
            crate::suggestions::write_private(&path, task(&book, &root, &vault, &note, &today).as_bytes())
                .with_context(|| format!("cannot write {}", path.display()))?;
            let mut prompt = format!("Read {} and do what it says.", path.display());
            if let Some(ask) = ask {
                prompt.push_str(&format!(" Then answer the user's question: {ask}"));
            }
            format!(
                "{} --session-id {session} {}",
                launcher.as_deref().unwrap_or("claude"),
                shell_words::quote(&prompt)
            )
        };
        set_session(&root, id, &session)?;
        let folder = root.to_string_lossy().into_owned();
        {
            let mut core = self.core();
            if core.workspace.group_for(&folder).is_none() {
                // Not "서재": that is the shelf screen the rail's book icon opens.
                core.workspace.add_group("책 논의".to_owned(), Some(folder));
                self.changed(&core);
            }
        }
        let title: String = book.title.chars().take(30).collect();
        let pane = self.background_tab(
            dir(&root, id).to_string_lossy().into_owned(),
            command,
            format!("서재 · {title}"),
        )?;
        PANES
            .lock()
            .expect("library panes lock")
            .get_or_insert_default()
            .insert(id.to_owned(), pane);
        self.library_changed();
        Ok(pane)
    }

    /// Sends every screen the page's translation: the saved one unless `again`, else a new one
    /// from claude, made in the background.
    pub fn library_translate(self: &Arc<Self>, id: String, page: u32, text: String, again: bool) {
        let root = self.library_root();
        let book = match book(&root, &id) {
            Ok(book) => book,
            Err(error) => return self.library_translated(&id, page, Err(error)),
        };
        if !again {
            match translation(&root, &id, page) {
                Ok(Some(saved)) => return self.library_translated(&id, page, Ok(saved)),
                Ok(None) => {}
                Err(error) => return self.library_translated(&id, page, Err(error)),
            }
        }
        if text.trim().is_empty() {
            // ponytail: scanned pages carry no text; send the page image to claude if they matter.
            let error = anyhow::anyhow!(
                "{page}쪽에는 글자 데이터가 없어요. 스캔한 쪽은 아직 번역하지 못해요"
            );
            return self.library_translated(&id, page, Err(error));
        }
        queue::forget_failure(&id, page);
        if !TRANSLATING
            .lock()
            .expect("translating lock")
            .insert((id.clone(), page))
        {
            return;
        }
        let before = page.checked_sub(2).and_then(|at| {
            texts(&root, &id)
                .ok()
                .flatten()?
                .into_iter()
                .nth(at as usize)
        });
        let daemon = self.clone();
        std::thread::spawn(move || {
            let result = translate(&daemon.home, &root, &book, page, &text, before.as_deref());
            TRANSLATING
                .lock()
                .expect("translating lock")
                .remove(&(id.clone(), page));
            daemon.library_translated(&id, page, result);
        });
    }

    /// Tells every screen how a page's translation went and how far the book is.
    fn library_translated(&self, id: &str, page: u32, result: anyhow::Result<String>) {
        let (text, error) = match result {
            Ok(text) => (Some(text), None),
            Err(error) => (None, Some(format!("{error:#}"))),
        };
        let translated = text.is_some();
        let message = ServerMsg::LibraryTranslation {
            book: id.to_owned(),
            page,
            text,
            error,
        };
        let _ = self.events.send(message.text());
        if translated {
            let progress = ServerMsg::LibraryProgress {
                book: id.to_owned(),
                translated: translated_pages(&self.library_root(), id).len() as u32,
            };
            let _ = self.events.send(progress.text());
        }
    }

    /// Asks the book's session about a passage: typed into its pane when it runs, else as the
    /// first message of the session it starts or resumes.
    pub fn library_ask(
        self: &Arc<Self>,
        id: &str,
        page: u32,
        quote: &str,
        question: &str,
    ) -> anyhow::Result<PaneId> {
        let text = ask_text(page, quote, question)?;
        let book = book(&self.library_root(), id)?;
        let Some(pane) = self.book_pane(&book) else {
            return self.library_discuss(id, Some(&text));
        };
        self.input(pane, format!("\x1b[200~{text}\x1b[201~"))?;
        // Enter right behind a paste can land before the agent has taken the paste in.
        let daemon = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            let _ = daemon.input(pane, "\r".to_owned());
        });
        Ok(pane)
    }

    pub fn library_open_note(&self, id: &str) -> anyhow::Result<()> {
        let book = book(&self.library_root(), id)?;
        let vault = crate::paths::library_notes().map_err(anyhow::Error::msg)?;
        let note = note_path(&vault, &book.title)?;
        ensure!(
            note.is_file(),
            "아직 노트가 없어요. 논의에서 정리하면 생겨요"
        );
        let url = format!(
            "obsidian://open?path={}",
            crate::learn::percent_encode(&note.to_string_lossy())
        );
        std::process::Command::new("open").arg(url).spawn()?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "library_tests.rs"]
mod tests;
