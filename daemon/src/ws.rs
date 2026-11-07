//! The screen's WebSocket: typed events in, pane output and daemon events out.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Weak};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, ORIGIN, RANGE};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tower::ServiceExt;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::ServeFile;

use crate::core::Daemon;
use crate::learn::{MarkView, Stage};
use crate::osc::Osc;
use crate::pane::Pane;
use crate::state::{Direction, Id, PaneId, Workspace};
use crate::team::PaneTeam;

/// Messages a slow screen may have queued before pane output waits for it.
const OUTBOX: usize = 256;

#[derive(Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum ClientMsg {
    NewTab {
        group: Option<Id>,
        cwd: Option<String>,
    },
    Split {
        pane: PaneId,
        direction: Direction,
    },
    Focus {
        pane: PaneId,
    },
    AddGroup {
        name: String,
        path: Option<String>,
    },
    RenameGroup {
        group: Id,
        name: String,
    },
    MarkSeen {
        pane: PaneId,
    },
    SetTitle {
        pane: PaneId,
        title: Option<String>,
    },
    Attach {
        pane: PaneId,
    },
    Detach {
        pane: PaneId,
    },
    Input {
        pane: PaneId,
        data: String,
    },
    Resize {
        pane: PaneId,
        cols: u16,
        rows: u16,
    },
    ClosePane {
        pane: PaneId,
    },
    ViewSeen {
        view: Id,
    },
    CloseView {
        view: Id,
    },
    ReadView {
        view: Id,
    },
    ReadRecord {
        pane: PaneId,
        mtime: Option<u64>,
    },
    ReadDecisions {
        pane: PaneId,
        mtime: Option<u64>,
    },
    ReadLearn,
    ReadLetters,
    /// The user's letter to the agent in `to`.
    LetterSend {
        to: PaneId,
        text: String,
    },
    /// Sends a dropped or untaken letter again.
    LetterRetry {
        id: u32,
    },
    /// ⌘⇧L with `text` selected in `pane`.
    LearnMark {
        pane: PaneId,
        text: String,
    },
    LearnStage {
        id: u32,
        stage: Stage,
    },
    LearnHard {
        id: u32,
        hard: String,
        retitle: bool,
    },
    LearnStudy {
        id: u32,
    },
    LearnNote {
        id: u32,
    },
    LearnOpenNote {
        id: u32,
    },
    /// The 학습 지도.
    ReadLearnMap,
    /// A study note picked on the map, for the document lane.
    LearnOpenStudyNote {
        path: String,
    },
    LearnTake {
        proposal: u32,
    },
    LearnSkip {
        proposal: u32,
    },
    /// One of mark `id`'s pictures, by file name in its material folder.
    LearnOpenMaterial {
        id: u32,
        name: String,
    },
    TitleCandidates {
        pane: PaneId,
    },
    /// Naru AI for the repository `pane` is in.
    Ai {
        pane: PaneId,
    },
    AiAct {
        pane: PaneId,
        act: crate::ai::Act,
    },
    /// Every agent's accounts and limits.
    Accounts,
    AccountAct {
        act: crate::accounts::AccountAct,
    },
    SearchConversations {
        query: String,
    },
    ResumeConversation {
        session_id: String,
        cwd: String,
        /// In the focused pane when it is a plain shell.
        here: bool,
    },
    /// Brain's note; `mtime` is the version the screen has, to skip an unchanged note.
    BrainRead {
        mtime: Option<u64>,
    },
    BrainWrite {
        body: String,
        base: Option<u64>,
    },
    BrainOpen,
    /// A web link the user clicked in a lane document, for the default browser.
    OpenLink {
        url: String,
    },
    /// A file path the user ⌘-clicked in a pane's output; relative to that pane's folder.
    OpenPath {
        pane: PaneId,
        path: String,
    },
    /// What a pane's checkout changed since its last commit (code map).
    CodeMap {
        pane: PaneId,
    },
    /// Open pull requests of the projects' repositories (PR panel).
    Prs,
    /// One level of the Meta 광고 tree: campaigns, or the children of `parent`.
    MetaAds {
        account: Option<String>,
        level: crate::meta_ads::Level,
        parent: Option<String>,
        preset: String,
    },
    /// A pause, resume or budget change the user confirmed.
    MetaAdsAct {
        account: String,
        id: String,
        act: crate::meta_ads::Act,
    },
    /// Saves the Meta system user token, or forgets it (`None`).
    MetaAdsToken {
        token: Option<String>,
    },
    /// The outside folders (Obsidian notes) set in 설정 → 경로.
    Paths,
    PathsSet {
        paths: crate::paths::Paths,
    },
    /// The last seven days' spend by project, at API prices (cost panel).
    Costs,
    /// The user approves the plan `pane`'s agent put up for review.
    PlanApprove {
        pane: PaneId,
    },
    /// Ports programs started in panes listen on (dev server panel).
    Servers,
    /// Stops a dev server the panel listed: SIGTERM, then SIGKILL five seconds later.
    StopServer {
        pid: u32,
    },
    /// The Warp fork's project thumbnails, for a screen taking them in once.
    LegacyThumbnails,
    /// A screen setting changed; `None` forgets it.
    UiSet {
        key: String,
        value: Option<String>,
    },
    /// A session card's background: a base64 PNG or JPEG, or `None` to clear it.
    SetBackground {
        tab: Id,
        image: Option<String>,
    },
    ReadBackground {
        tab: Id,
    },
    /// The 서재's books.
    ReadLibrary,
    /// A PDF to shelve: its file name, page count, a JPEG of its first page in base64, the text of
    /// every page, and the file as either its path on this Mac or base64 (a browser screen has no
    /// path).
    LibraryAdd {
        name: String,
        pages: u32,
        cover: String,
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        pdf: Option<String>,
        #[serde(default)]
        texts: Vec<String>,
    },
    /// The text of every page of a book shelved before its text was kept.
    LibraryTexts {
        book: String,
        texts: Vec<String>,
    },
    LibraryRemove {
        book: String,
    },
    /// The reader opened the book.
    LibraryOpen {
        book: String,
    },
    /// The reader moved to `page`.
    LibraryPage {
        book: String,
        page: u32,
    },
    /// `text` is the page's text the reader extracted; a saved translation answers unless `again`.
    LibraryTranslate {
        book: String,
        page: u32,
        text: String,
        again: bool,
    },
    /// Opens 마케팅 모드: checks organic-lab's server and starts or resumes its session.
    MarketingOpen,
    /// Asks only whether organic-lab's server answers.
    MarketingCheck,
    /// Runs organic-lab's server in a background tab, on the user's press.
    MarketingStartServer,
    /// Starts or resumes the book's claude session.
    LibraryDiscuss {
        book: String,
    },
    LibraryOpenNote {
        book: String,
    },
    /// Asks the book's session about a passage of `page`.
    LibraryAsk {
        book: String,
        page: u32,
        quote: String,
        question: String,
    },
    /// The book's underlines and figures.
    ReadLibraryMarks {
        book: String,
    },
    /// Underlines `quote` on the translated `page`; `prefix` and `suffix` find it again.
    LibraryMark {
        book: String,
        page: u32,
        quote: String,
        prefix: String,
        suffix: String,
    },
    LibraryUnmark {
        book: String,
        mark: String,
    },
    /// Draws a figure of `quote` after block `block` of the translated `page`.
    LibraryDraw {
        book: String,
        page: u32,
        block: u32,
        quote: String,
        context: String,
        prompt: String,
    },
    LibraryRedraw {
        book: String,
        figure: String,
    },
    LibraryUndraw {
        book: String,
        figure: String,
    },
}

#[derive(Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerMsg<'a> {
    /// The whole workspace, sent on connect and after every change.
    // ponytail: whole state per change; send diffs if it grows past a few hundred KB.
    State {
        workspace: &'a Workspace,
        team: &'a BTreeMap<PaneId, PaneTeam>,
    },
    /// Everything the pane kept; the screen starts the terminal over before writing it.
    Replay {
        pane: PaneId,
        data: String,
    },
    Output {
        pane: PaneId,
        data: String,
    },
    Osc {
        pane: PaneId,
        osc: &'a Osc,
    },
    /// A document lane file, answering `read_view`. Images come as base64 `data`; text, and the
    /// reason for kind `error`, as `text`.
    ViewBody {
        view: Id,
        kind: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime: Option<&'static str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<String>,
    },
    Record {
        pane: PaneId,
        #[serde(flatten)]
        record: crate::record::Record,
    },
    Decisions {
        pane: PaneId,
        #[serde(flatten)]
        decisions: crate::record::Decisions,
    },
    /// The 학습함 marks, after any change and on `read_learn`.
    Learn {
        marks: Vec<MarkView>,
        /// What Memory reads found the user did not know, for the user to take or pass.
        proposals: Vec<crate::learn::Proposal>,
        /// Memory reads sessions in some repository, so proposals can come.
        memory_on: bool,
        error: Option<String>,
    },
    /// Every letter on record; answers `read_letters` and goes to every screen when one changes.
    Letters {
        letters: Vec<crate::letters::View<'a>>,
    },
    /// Answers `read_learn_map`, and goes to every screen after a grouping. The marks and proposals
    /// on it are the ones `Learn` sends.
    LearnMap {
        /// `None` until the first grouping.
        map: Option<crate::learn_map::Map>,
        notes: Vec<crate::learn_map::Note>,
        /// The study folder is not there: the outside disk is not connected.
        offline: bool,
    },
    /// Answers `learn_mark`: the new mark, whose hard part the screen asks for next.
    LearnMarked {
        id: u32,
    },
    /// A 학습함 action that failed, shown under its mark.
    LearnFailure {
        id: Option<u32>,
        message: String,
    },
    CodeMap {
        pane: PaneId,
        map: Option<crate::code_map::CodeMap>,
        error: Option<String>,
    },
    Prs {
        repos: Vec<crate::devtools::RepoPrs>,
    },
    MetaAds {
        #[serde(flatten)]
        view: crate::meta_ads::View,
    },
    /// Answers `paths` and `paths_set`; `paths` is what is saved, also after a refused save.
    Paths {
        paths: Option<crate::paths::Paths>,
        error: Option<String>,
    },
    /// Answers `meta_ads_act` and `meta_ads_token`; the screen reads the tree again after it.
    MetaAdsDone {
        id: Option<String>,
        error: Option<String>,
    },
    Costs {
        costs: crate::cost::Costs,
    },
    Servers {
        servers: Vec<crate::devtools::Server>,
        error: Option<String>,
    },
    /// Answers `title_candidates`; `error` when no ideas came.
    TitleCandidates {
        pane: PaneId,
        titles: Vec<String>,
        error: Option<String>,
    },
    /// Answers `ai` and `ai_act`; no `view` when the pane is not in a git repository.
    Ai {
        pane: PaneId,
        view: Option<Box<crate::ai::View>>,
        error: Option<String>,
    },
    /// Answers `accounts` and `account_act`.
    Accounts {
        agents: Vec<crate::accounts::AgentView>,
        notice: Option<String>,
        /// A pane the action opened, which the screen focuses.
        opened: Option<PaneId>,
        error: Option<String>,
    },
    /// Answers `search_conversations`; `query` tells a late answer from the current one.
    Conversations {
        query: String,
        found: Vec<crate::conversations::Conversation>,
    },
    Brain {
        path: String,
        note: crate::brain::Note,
    },
    BrainWritten {
        written: crate::brain::Written,
    },
    /// The screen settings, sent on connect before the state.
    // ponytail: other screens hear of a change only when they reconnect.
    Ui {
        values: BTreeMap<String, String>,
    },
    /// Answers `read_background`: the image at `path` in base64, or why it cannot be shown.
    Background {
        path: String,
        mime: Option<&'static str>,
        data: Option<String>,
        error: Option<String>,
    },
    /// Base64 PNG by group id.
    LegacyThumbnails {
        thumbnails: BTreeMap<Id, String>,
    },
    /// The 서재's books, on `read_library` and after a book comes, goes or starts its session.
    // ponytail: page moves are not sent on; another screen sees them on its next `read_library`.
    Library {
        books: Vec<crate::library::BookView>,
        error: Option<String>,
    },
    LibraryTranslation {
        book: String,
        page: u32,
        text: Option<String>,
        error: Option<String>,
    },
    /// How many pages of the book are translated, after each one.
    LibraryProgress {
        book: String,
        translated: u32,
    },
    /// The book's underlines and figures, on `read_library_marks` and after each change.
    LibraryMarks {
        book: String,
        marks: Vec<crate::library::Mark>,
        figures: Vec<crate::library::FigureView>,
    },
    /// Answers `marketing_open` and `marketing_check`; `pane` is the organic-lab session.
    Marketing {
        url: Option<String>,
        up: bool,
        pane: Option<PaneId>,
        error: Option<String>,
    },
    /// Answers `library_discuss` with the pane of the book's session.
    LibraryDiscussion {
        book: String,
        pane: PaneId,
    },
    /// A 서재 action that failed.
    LibraryFailure {
        message: String,
    },
    /// A short line the screen shows for a moment.
    Notice {
        text: String,
    },
    Error {
        message: String,
    },
}

impl ServerMsg<'_> {
    pub fn text(&self) -> Arc<str> {
        serde_json::to_string(self)
            .expect("server messages serialize")
            .into()
    }
}

pub async fn handler(
    ws: WebSocketUpgrade,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    State(daemon): State<Arc<Daemon>>,
) -> Response {
    let origin = headers.get(ORIGIN).and_then(|value| value.to_str().ok());
    if query.get("token") != Some(&daemon.token) || !origin_allowed(origin) {
        return StatusCode::FORBIDDEN.into_response();
    }
    // A PDF from a browser screen comes as one base64 message.
    let limit = crate::library::MAX_BOOK / 3 * 4 + (1 << 20);
    ws.max_message_size(limit)
        .max_frame_size(limit)
        .on_upgrade(move |socket| serve(socket, daemon))
}

/// The PDF of a shelved book, served in ranges so the reader fetches only the pages it draws.
pub async fn book_pdf(
    Path(id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    State(daemon): State<Arc<Daemon>>,
    request: Request,
) -> Response {
    let origin = request
        .headers()
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok());
    if query.get("token") != Some(&daemon.token) || !origin_allowed(origin) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let root = daemon.library_root();
    // Only a shelved id becomes a path, so nothing outside the 서재 can be asked for.
    if crate::library::book(&root, &id).is_err() {
        return StatusCode::NOT_FOUND.into_response();
    }
    match ServeFile::new(crate::library::pdf_path(&root, &id))
        .oneshot(request)
        .await
    {
        Ok(response) => response.into_response(),
        Err(never) => match never {},
    }
}

/// The Tauri screen is another origin than the daemon, and pdf.js reads the range headers.
pub fn book_cors() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _| {
            origin_allowed(Some(origin.to_str().unwrap_or("")))
        }))
        .allow_methods([Method::GET])
        .allow_headers([RANGE])
        .expose_headers([ACCEPT_RANGES, CONTENT_RANGE, CONTENT_LENGTH])
}

/// Pages from anywhere else are refused even before the token is checked against them.
fn origin_allowed(origin: Option<&str>) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    origin == "tauri://localhost"
        || origin == "http://tauri.localhost"
        || ["http://localhost:", "http://127.0.0.1:"]
            .iter()
            .any(|prefix| {
                origin
                    .strip_prefix(prefix)
                    .is_some_and(|port| port.parse::<u16>().is_ok())
            })
}

async fn serve(socket: WebSocket, daemon: Arc<Daemon>) {
    let (mut sink, mut stream) = socket.split();
    let (outbox, mut queued) = mpsc::channel::<Arc<str>>(OUTBOX);
    let writer = tokio::spawn(async move {
        while let Some(text) = queued.recv().await {
            if sink
                .send(Message::Text(text.as_ref().into()))
                .await
                .is_err()
            {
                return;
            }
        }
    });
    // Subscribed before the state, so a change in between is sent twice, never not at all.
    let mut events = daemon.events.subscribe();
    let ui = match crate::ui::read(&crate::ui::path(&daemon.state_file)) {
        Ok(values) => ServerMsg::Ui { values },
        Err(error) => ServerMsg::Error {
            message: format!("{error:#}"),
        },
    };
    let _ = outbox.send(ui.text()).await;
    let _ = outbox.send(daemon.state_message()).await;
    let events_task = {
        let outbox = outbox.clone();
        let daemon = daemon.clone();
        tokio::spawn(async move {
            loop {
                let text = match events.recv().await {
                    Ok(text) => text,
                    Err(broadcast::error::RecvError::Lagged(_)) => daemon.state_message(),
                    Err(broadcast::error::RecvError::Closed) => return,
                };
                if outbox.send(text).await.is_err() {
                    return;
                }
            }
        })
    };
    let mut attached: HashMap<PaneId, JoinHandle<()>> = HashMap::new();
    while let Some(Ok(message)) = stream.next().await {
        let Message::Text(text) = message else {
            continue;
        };
        let result = match serde_json::from_str::<ClientMsg>(&text) {
            Ok(message) => handle(message, &daemon, &outbox, &mut attached),
            Err(error) => Err(error.into()),
        };
        if let Err(error) = result {
            let message = ServerMsg::Error {
                message: error.to_string(),
            };
            let _ = outbox.send(message.text()).await;
        }
    }
    attached.values().for_each(JoinHandle::abort);
    events_task.abort();
    writer.abort();
}

fn handle(
    message: ClientMsg,
    daemon: &Arc<Daemon>,
    outbox: &mpsc::Sender<Arc<str>>,
    attached: &mut HashMap<PaneId, JoinHandle<()>>,
) -> anyhow::Result<()> {
    match message {
        ClientMsg::NewTab { group, cwd } => {
            daemon.new_tab(group, cwd, None)?;
        }
        ClientMsg::Split { pane, direction } => {
            daemon.split(pane, direction)?;
        }
        ClientMsg::Focus { pane } => daemon.focus(pane)?,
        ClientMsg::AddGroup { name, path } => daemon.add_group(name, path),
        ClientMsg::RenameGroup { group, name } => daemon.rename_group(group, &name)?,
        ClientMsg::MarkSeen { pane } => daemon.mark_seen(pane)?,
        ClientMsg::SetTitle { pane, title } => daemon.set_title(pane, title)?,
        ClientMsg::Attach { pane } => {
            let forwarder =
                forward_output(pane, Arc::downgrade(&daemon.pane(pane)?), outbox.clone());
            if let Some(previous) = attached.insert(pane, forwarder) {
                previous.abort();
            }
        }
        ClientMsg::Detach { pane } => {
            if let Some(forwarder) = attached.remove(&pane) {
                forwarder.abort();
            }
        }
        ClientMsg::Input { pane, data } => daemon.input(pane, data)?,
        ClientMsg::Resize { pane, cols, rows } => daemon.resize(pane, cols, rows)?,
        ClientMsg::ClosePane { pane } => daemon.close_pane(pane)?,
        ClientMsg::ViewSeen { view } => daemon.view_seen(view)?,
        ClientMsg::CloseView { view } => daemon.close_view(view),
        ClientMsg::ReadRecord { pane, mtime } => {
            let record = ServerMsg::Record {
                pane,
                record: daemon.record(pane, mtime),
            };
            outbox
                .try_send(record.text())
                .map_err(|_| anyhow::anyhow!("the screen is not keeping up"))?;
        }
        ClientMsg::ReadDecisions { pane, mtime } => {
            let decisions = ServerMsg::Decisions {
                pane,
                decisions: daemon.decisions(pane, mtime),
            };
            outbox
                .try_send(decisions.text())
                .map_err(|_| anyhow::anyhow!("the screen is not keeping up"))?;
        }
        ClientMsg::ReadLearn => reply(outbox, daemon.learn_message())?,
        ClientMsg::ReadLetters => reply(outbox, daemon.letters_message())?,
        ClientMsg::LetterSend { to, text } => daemon.letter_send(to, &text)?,
        ClientMsg::LetterRetry { id } => daemon.letter_retry(id)?,
        ClientMsg::LearnMark { pane, text } => {
            let message = match daemon.learn_mark_selection(pane, &text) {
                Ok(id) => ServerMsg::LearnMarked { id },
                Err(message) => ServerMsg::LearnFailure { id: None, message },
            };
            reply(outbox, message.text())?;
        }
        ClientMsg::LearnStage { id, stage } => learn(outbox, id, daemon.learn_stage(id, stage))?,
        ClientMsg::LearnHard { id, hard, retitle } => {
            learn(outbox, id, daemon.learn_hard(id, &hard, retitle))?
        }
        ClientMsg::LearnStudy { id } => learn(outbox, id, daemon.learn_study(id))?,
        ClientMsg::LearnNote { id } => learn(outbox, id, daemon.learn_note(id))?,
        ClientMsg::LearnOpenNote { id } => learn(outbox, id, daemon.learn_open_note(id))?,
        ClientMsg::ReadLearnMap => {
            let daemon = daemon.clone();
            let outbox = outbox.clone();
            // Reads every note's head; on the external disk that takes a while.
            tokio::task::spawn_blocking(move || {
                let _ = outbox.try_send(daemon.learn_map_message());
            });
        }
        ClientMsg::LearnOpenStudyNote { path } => {
            notice_failure(outbox, daemon.learn_open_study_note(&path))?
        }
        ClientMsg::LearnTake { proposal } => notice_failure(outbox, daemon.learn_take(proposal))?,
        ClientMsg::LearnSkip { proposal } => notice_failure(outbox, daemon.learn_skip(proposal))?,
        ClientMsg::LearnOpenMaterial { id, name } => {
            learn(outbox, id, daemon.learn_open_material(id, &name))?
        }
        ClientMsg::OpenLink { url } => open_link(&url)?,
        ClientMsg::OpenPath { pane, path } => daemon.open_terminal_path(pane, &path),
        ClientMsg::CodeMap { pane } => {
            let record = daemon.core().workspace.panes.get(&pane).cloned();
            let outbox = outbox.clone();
            tokio::task::spawn_blocking(move || {
                let result = match record {
                    Some(record) if record.node.is_some() => {
                        Err(anyhow::anyhow!("원격 pane의 코드는 아직 볼 수 없어요"))
                    }
                    Some(record) => crate::code_map::for_pane(record.cwd.as_deref()),
                    None => Err(anyhow::anyhow!("pane이 닫혔어요")),
                };
                let (map, error) = match result {
                    Ok(map) => (Some(map), None),
                    Err(error) => (None, Some(format!("{error:#}"))),
                };
                let _ = outbox.try_send(ServerMsg::CodeMap { pane, map, error }.text());
            });
        }
        ClientMsg::MetaAds {
            account,
            level,
            parent,
            preset,
        } => {
            let naru_dir = daemon.naru_dir.clone();
            let outbox = outbox.clone();
            tokio::task::spawn_blocking(move || {
                let view = crate::meta_ads::view(&naru_dir, account, level, parent, preset);
                let _ = outbox.try_send(ServerMsg::MetaAds { view }.text());
            });
        }
        ClientMsg::MetaAdsAct { account, id, act } => {
            let naru_dir = daemon.naru_dir.clone();
            let outbox = outbox.clone();
            tokio::task::spawn_blocking(move || {
                let error = crate::meta_ads::change(&naru_dir, &account, &id, &act)
                    .err()
                    .map(|error| format!("{error:#}"));
                let _ = outbox.try_send(
                    ServerMsg::MetaAdsDone {
                        id: Some(id),
                        error,
                    }
                    .text(),
                );
            });
        }
        ClientMsg::MetaAdsToken { token } => {
            let naru_dir = daemon.naru_dir.clone();
            let outbox = outbox.clone();
            tokio::task::spawn_blocking(move || {
                let error = crate::meta_ads::set_token(&naru_dir, token)
                    .err()
                    .map(|error| format!("{error:#}"));
                let _ = outbox.try_send(ServerMsg::MetaAdsDone { id: None, error }.text());
            });
        }
        ClientMsg::Paths => {
            let (paths, error) = match crate::paths::load() {
                Ok(paths) => (Some(paths), None),
                Err(error) => (None, Some(error)),
            };
            reply(outbox, ServerMsg::Paths { paths, error }.text())?
        }
        ClientMsg::PathsSet { paths } => {
            let (paths, error) = match crate::paths::save(paths) {
                Ok(paths) => (Some(paths), None),
                Err(error) => (crate::paths::load().ok(), Some(error)),
            };
            reply(outbox, ServerMsg::Paths { paths, error }.text())?
        }
        ClientMsg::Prs => {
            let workspace = daemon.core().workspace.clone();
            let gh = crate::accounts::program(&daemon.home, "gh");
            let outbox = outbox.clone();
            tokio::task::spawn_blocking(move || {
                let repos = crate::devtools::open_prs(&workspace, gh.as_deref());
                let _ = outbox.try_send(ServerMsg::Prs { repos }.text());
            });
        }
        ClientMsg::PlanApprove { pane } => daemon.plan_approve(pane)?,
        ClientMsg::Costs => {
            let workspace = daemon.core().workspace.clone();
            let home = daemon.home.clone();
            let outbox = outbox.clone();
            tokio::task::spawn_blocking(move || {
                let costs = crate::cost::costs(&home, &workspace);
                let _ = outbox.try_send(ServerMsg::Costs { costs }.text());
            });
        }
        ClientMsg::Servers => {
            let shells = daemon.shells();
            let outbox = outbox.clone();
            tokio::task::spawn_blocking(move || {
                let (servers, error) = match crate::devtools::servers(&shells) {
                    Ok(servers) => (servers, None),
                    Err(error) => (Vec::new(), Some(format!("{error:#}"))),
                };
                let _ = outbox.try_send(ServerMsg::Servers { servers, error }.text());
            });
        }
        ClientMsg::StopServer { pid } => {
            let shells = daemon.shells();
            tokio::task::spawn_blocking(move || stop_server(pid, &shells));
        }
        ClientMsg::TitleCandidates { pane } => {
            let daemon = daemon.clone();
            let outbox = outbox.clone();
            tokio::task::spawn_blocking(move || {
                let (titles, error) = match daemon.title_candidates(pane) {
                    Ok(titles) => (titles, None),
                    Err(error) => (Vec::new(), Some(format!("{error:#}"))),
                };
                let reply = ServerMsg::TitleCandidates {
                    pane,
                    titles,
                    error,
                };
                let _ = outbox.try_send(reply.text());
            });
        }
        ClientMsg::Ai { pane } => {
            let daemon = daemon.clone();
            let outbox = outbox.clone();
            tokio::task::spawn_blocking(move || {
                let _ = outbox.try_send(ai_reply(&daemon, pane, Ok(())).text());
            });
        }
        ClientMsg::AiAct { pane, act } => {
            let daemon = daemon.clone();
            let outbox = outbox.clone();
            tokio::task::spawn_blocking(move || {
                let done = daemon.ai_act(pane, act);
                let _ = outbox.try_send(ai_reply(&daemon, pane, done).text());
            });
        }
        ClientMsg::Accounts => {
            let daemon = daemon.clone();
            let outbox = outbox.clone();
            tokio::task::spawn_blocking(move || {
                let _ = outbox.try_send(accounts_reply(&daemon, Ok(Default::default())).text());
            });
        }
        ClientMsg::AccountAct { act } => {
            let daemon = daemon.clone();
            let outbox = outbox.clone();
            tokio::task::spawn_blocking(move || {
                let done = daemon.account_act(act);
                let _ = outbox.try_send(accounts_reply(&daemon, done).text());
            });
        }
        ClientMsg::SearchConversations { query } => {
            let daemon = daemon.clone();
            let outbox = outbox.clone();
            tokio::task::spawn_blocking(move || {
                let found = daemon.search_conversations(&query);
                let _ = outbox.try_send(ServerMsg::Conversations { query, found }.text());
            });
        }
        ClientMsg::ResumeConversation {
            session_id,
            cwd,
            here,
        } => daemon.resume_conversation(&session_id, std::path::Path::new(&cwd), here)?,
        // ponytail: note IO on the socket task; the note is capped at 2 MiB.
        ClientMsg::BrainRead { mtime } => {
            let (path, note) = match crate::paths::brain_note() {
                Ok(path) => {
                    let note = crate::brain::read(&path, mtime);
                    (path.to_string_lossy().into_owned(), note)
                }
                Err(message) => (String::new(), crate::brain::Note::Error { message }),
            };
            reply(outbox, ServerMsg::Brain { path, note }.text())?
        }
        ClientMsg::BrainWrite { body, base } => {
            let written = match crate::paths::brain_note() {
                Ok(path) => crate::brain::write(&path, &body, base),
                Err(message) => crate::brain::Written::Error { message },
            };
            reply(outbox, ServerMsg::BrainWritten { written }.text())?
        }
        ClientMsg::BrainOpen => {
            let path = crate::paths::brain_note().map_err(anyhow::Error::msg)?;
            let url = format!(
                "obsidian://open?path={}",
                crate::learn::percent_encode(&path.to_string_lossy())
            );
            std::process::Command::new("open").arg(url).spawn()?;
        }
        ClientMsg::UiSet { key, value } => {
            crate::ui::set(&crate::ui::path(&daemon.state_file), &key, value.as_deref())?
        }
        ClientMsg::SetBackground { tab, image } => {
            let dir = crate::background::dir(&daemon.state_file);
            let saved = match image {
                Some(data) => Some(crate::background::save(&dir, &BASE64.decode(data)?)?),
                None => None,
            };
            let mut core = daemon.core();
            let Some(entry) = core.workspace.tabs.iter_mut().find(|entry| entry.id == tab) else {
                drop(core);
                if let Some(saved) = &saved {
                    crate::background::remove_own(&dir, saved);
                }
                anyhow::bail!("no session {tab} to put a background on");
            };
            let old = std::mem::replace(
                &mut entry.background,
                saved.map(|path| path.to_string_lossy().into_owned()),
            );
            daemon.changed(&core);
            drop(core);
            if let Some(old) = old {
                crate::background::remove_own(&dir, std::path::Path::new(&old));
            }
        }
        // ponytail: reads on the socket task, like documents; images are capped at 8 MiB.
        ClientMsg::ReadBackground { tab } => {
            let path = daemon
                .core()
                .workspace
                .tabs
                .iter()
                .find(|entry| entry.id == tab)
                .and_then(|entry| entry.background.clone())
                .ok_or_else(|| anyhow::anyhow!("session {tab} has no background"))?;
            let message = match crate::background::read(std::path::Path::new(&path)) {
                Ok((mime, bytes)) => ServerMsg::Background {
                    path,
                    mime: Some(mime),
                    data: Some(BASE64.encode(bytes)),
                    error: None,
                },
                Err(error) => ServerMsg::Background {
                    path,
                    mime: None,
                    data: None,
                    error: Some(format!("{error:#}")),
                },
            };
            reply(outbox, message.text())?
        }
        // Off the socket task: each book's note is looked up on the vault's outside disk, which can
        // take seconds to wake while every pane's output waits behind it.
        ClientMsg::ReadLibrary => library_job(daemon, outbox, |daemon, outbox| {
            reply(outbox, daemon.library_message())
        }),
        ClientMsg::LibraryAdd {
            name,
            pages,
            cover,
            path,
            pdf,
            texts,
        } => library_job(daemon, outbox, move |daemon, _| {
            let root = daemon.library_root();
            let pdf = match (path, pdf) {
                (Some(path), None) => crate::library::read_picked(std::path::Path::new(&path))?,
                (None, Some(pdf)) => BASE64.decode(pdf)?,
                _ => anyhow::bail!("{name}: 파일 경로와 내용 중 하나만 와야 해요"),
            };
            let book = crate::library::add(&root, &name, pages, &BASE64.decode(cover)?, &pdf)?;
            let kept = (!texts.is_empty())
                .then(|| crate::library::save_texts(&root, &book.id, texts))
                .transpose();
            daemon.library_changed();
            crate::library::queue::wake();
            kept.map(drop)
        }),
        ClientMsg::LibraryTexts { book, texts } => library_job(daemon, outbox, move |daemon, _| {
            crate::library::save_texts(&daemon.library_root(), &book, texts)?;
            daemon.library_changed();
            crate::library::queue::wake();
            Ok(())
        }),
        ClientMsg::LibraryRemove { book } => library_job(daemon, outbox, move |daemon, _| {
            crate::library::remove(&daemon.library_root(), &book)?;
            daemon.library_changed();
            Ok(())
        }),
        ClientMsg::LibraryOpen { book } => library_job(daemon, outbox, move |daemon, _| {
            crate::library::open(&daemon.library_root(), &book)
        }),
        ClientMsg::LibraryPage { book, page } => {
            if let Err(error) = crate::library::set_page(&daemon.library_root(), &book, page) {
                let message = format!("{error:#}");
                reply(outbox, ServerMsg::LibraryFailure { message }.text())?
            }
        }
        ClientMsg::LibraryTranslate {
            book,
            page,
            text,
            again,
        } => daemon.library_translate(book, page, text, again),
        ClientMsg::MarketingOpen => marketing_job(daemon, outbox, |daemon| {
            let opened = daemon.marketing_open()?;
            Ok((Some(opened.url), opened.up, Some(opened.pane)))
        }),
        ClientMsg::MarketingCheck => marketing_job(daemon, outbox, |daemon| {
            Ok((None, daemon.marketing_server_up()?, None))
        }),
        ClientMsg::MarketingStartServer => marketing_job(daemon, outbox, |daemon| {
            daemon.marketing_start_server()?;
            Ok((None, false, None))
        }),
        ClientMsg::LibraryDiscuss { book } => library_job(daemon, outbox, move |daemon, outbox| {
            let pane = daemon.library_discuss(&book, None)?;
            reply(outbox, ServerMsg::LibraryDiscussion { book, pane }.text())
        }),
        ClientMsg::LibraryOpenNote { book } => library_job(daemon, outbox, move |daemon, _| {
            daemon.library_open_note(&book)
        }),
        ClientMsg::LibraryAsk {
            book,
            page,
            quote,
            question,
        } => library_job(daemon, outbox, move |daemon, outbox| {
            let pane = daemon.library_ask(&book, page, &quote, &question)?;
            reply(outbox, ServerMsg::LibraryDiscussion { book, pane }.text())
        }),
        ClientMsg::ReadLibraryMarks { book } => {
            library_job(daemon, outbox, move |daemon, outbox| {
                reply(outbox, daemon.library_marks_message(&book)?)
            })
        }
        ClientMsg::LibraryMark {
            book,
            page,
            quote,
            prefix,
            suffix,
        } => library_job(daemon, outbox, move |daemon, _| {
            daemon.library_mark(&book, page, &quote, &prefix, &suffix)
        }),
        ClientMsg::LibraryUnmark { book, mark } => library_job(daemon, outbox, move |daemon, _| {
            daemon.library_unmark(&book, &mark)
        }),
        ClientMsg::LibraryDraw {
            book,
            page,
            block,
            quote,
            context,
            prompt,
        } => library_job(daemon, outbox, move |daemon, _| {
            daemon.library_draw(&book, page, block, &quote, &context, &prompt)
        }),
        ClientMsg::LibraryRedraw { book, figure } => {
            library_job(daemon, outbox, move |daemon, _| {
                daemon.library_redraw(&book, &figure)
            })
        }
        ClientMsg::LibraryUndraw { book, figure } => {
            library_job(daemon, outbox, move |daemon, _| {
                daemon.library_undraw(&book, &figure)
            })
        }
        ClientMsg::LegacyThumbnails => {
            let groups = daemon.core().workspace.groups.clone();
            let thumbnails = crate::import::thumbnails(&daemon.home, &groups);
            reply(outbox, ServerMsg::LegacyThumbnails { thumbnails }.text())?
        }
        ClientMsg::ReadView { view } => {
            let body = daemon
                .view_path(view)
                .and_then(|path| view_body(view, &path, daemon.page_in_lane(&path)))
                .unwrap_or_else(|error| ServerMsg::ViewBody {
                    view,
                    kind: "error",
                    mime: None,
                    text: Some(error.to_string()),
                    data: None,
                });
            outbox
                .try_send(body.text())
                .map_err(|_| anyhow::anyhow!("the screen is not keeping up"))?;
        }
    }
    Ok(())
}

fn ai_reply(daemon: &Daemon, pane: PaneId, done: anyhow::Result<()>) -> ServerMsg<'static> {
    let (view, error) = match done.and_then(|()| daemon.ai_view(pane)) {
        Ok(view) => (view.map(Box::new), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    ServerMsg::Ai { pane, view, error }
}

fn accounts_reply(
    daemon: &Arc<Daemon>,
    done: anyhow::Result<crate::accounts::ActDone>,
) -> ServerMsg<'static> {
    let agents = daemon.accounts_view();
    let (done, error) = match done {
        Ok(done) => (done, None),
        Err(error) => (Default::default(), Some(format!("{error:#}"))),
    };
    ServerMsg::Accounts {
        agents,
        notice: done.notice,
        opened: done.opened,
        error,
    }
}

/// Only a process the panel would list now: the pid came from the screen.
fn stop_server(pid: u32, shells: &[(PaneId, u32)]) {
    let listed = || {
        crate::devtools::servers(shells)
            .is_ok_and(|servers| servers.iter().any(|server| server.pid == pid))
    };
    if !listed() {
        return;
    }
    // SAFETY: kill takes no pointers.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    std::thread::sleep(std::time::Duration::from_secs(5));
    // Checked again so a pid reused in the meantime is left alone.
    if listed() {
        // SAFETY: as above.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    }
}

fn reply(outbox: &mpsc::Sender<Arc<str>>, text: Arc<str>) -> anyhow::Result<()> {
    outbox
        .try_send(text)
        .map_err(|_| anyhow::anyhow!("the screen is not keeping up"))
}

/// Runs a 서재 action off the socket task, telling this screen when it failed.
/// Runs a 마케팅 job off the socket task; a failure comes back on the same message.
fn marketing_job(
    daemon: &Arc<Daemon>,
    outbox: &mpsc::Sender<Arc<str>>,
    job: impl FnOnce(&Arc<Daemon>) -> anyhow::Result<(Option<String>, bool, Option<PaneId>)>
    + Send
    + 'static,
) {
    let daemon = daemon.clone();
    let outbox = outbox.clone();
    tokio::task::spawn_blocking(move || {
        let message = match job(&daemon) {
            Ok((url, up, pane)) => ServerMsg::Marketing {
                url,
                up,
                pane,
                error: None,
            },
            Err(error) => ServerMsg::Marketing {
                url: None,
                up: false,
                pane: None,
                error: Some(format!("{error:#}")),
            },
        };
        let _ = outbox.try_send(message.text());
    });
}

fn library_job(
    daemon: &Arc<Daemon>,
    outbox: &mpsc::Sender<Arc<str>>,
    job: impl FnOnce(&Arc<Daemon>, &mpsc::Sender<Arc<str>>) -> anyhow::Result<()> + Send + 'static,
) {
    let daemon = daemon.clone();
    let outbox = outbox.clone();
    tokio::task::spawn_blocking(move || {
        if let Err(error) = job(&daemon, &outbox) {
            let message = format!("{error:#}");
            let _ = outbox.try_send(ServerMsg::LibraryFailure { message }.text());
        }
    });
}

/// Tells this screen when a 학습함 action on mark `id` failed.
fn learn(
    outbox: &mpsc::Sender<Arc<str>>,
    id: u32,
    result: Result<(), String>,
) -> anyhow::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(message) => reply(
            outbox,
            ServerMsg::LearnFailure {
                id: Some(id),
                message,
            }
            .text(),
        ),
    }
}

/// Tells this screen when a 학습함 action that belongs to no mark failed.
fn notice_failure(
    outbox: &mpsc::Sender<Arc<str>>,
    result: Result<(), String>,
) -> anyhow::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(message) => reply(outbox, ServerMsg::LearnFailure { id: None, message }.text()),
    }
}

/// Only web pages: a lane document is agent-written, so other schemes could start anything.
fn open_link(url: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        url.starts_with("https://") || url.starts_with("http://"),
        "only web links open from a document: {url}"
    );
    std::process::Command::new("open").arg(url).spawn()?;
    Ok(())
}

/// Largest file the lane shows.
pub(crate) const VIEW_LIMIT: u64 = 32 << 20;

/// Reads a lane file for the screen: markdown, an image, or UTF-8 text. Nothing in it is run here;
/// an HTML `page` the screen shows in a sandboxed frame.
// ponytail: reads on the socket task; move to spawn_blocking if large files stall input.
fn view_body(view: Id, path: &std::path::Path, page: bool) -> anyhow::Result<ServerMsg<'static>> {
    let size = std::fs::metadata(path)?.len();
    anyhow::ensure!(
        size <= VIEW_LIMIT,
        "{} is too large to show ({size} bytes)",
        path.display()
    );
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let image = match extension.as_deref() {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        Some("svg") => Some("image/svg+xml"),
        Some("bmp") => Some("image/bmp"),
        _ => None,
    };
    let bytes = std::fs::read(path)?;
    if let Some(mime) = image {
        return Ok(ServerMsg::ViewBody {
            view,
            kind: "image",
            mime: Some(mime),
            text: None,
            data: Some(BASE64.encode(bytes)),
        });
    }
    let text =
        String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("{} is not text", path.display()))?;
    let kind = match extension.as_deref() {
        Some("md" | "markdown" | "mdx") => "markdown",
        Some("html" | "htm") if page => "html",
        _ => "text",
    };
    Ok(ServerMsg::ViewBody {
        view,
        kind,
        mime: None,
        text: Some(text),
        data: None,
    })
}

/// Sends the pane's replay, then its live output. A screen too slow to keep up gets a fresh
/// replay instead of a gap.
fn forward_output(
    pane_id: PaneId,
    pane: Weak<Pane>,
    outbox: mpsc::Sender<Arc<str>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let Some((replay, mut live)) = pane.upgrade().map(|pane| pane.attach()) else {
                return;
            };
            let replay = ServerMsg::Replay {
                pane: pane_id,
                data: BASE64.encode(replay),
            };
            if outbox.send(replay.text()).await.is_err() {
                return;
            }
            loop {
                match live.recv().await {
                    Ok(chunk) => {
                        let output = ServerMsg::Output {
                            pane: pane_id,
                            data: BASE64.encode(chunk),
                        };
                        if outbox.send(output.text()).await.is_err() {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => break,
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        }
    })
}

#[cfg(test)]
#[path = "ws_tests.rs"]
mod tests;
