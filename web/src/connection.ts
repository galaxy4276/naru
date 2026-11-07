export type PaneId = string;
export type Id = number;

export type Osc =
  | { kind: 'notify'; title: string; body: string }
  | { kind: 'prompt'; mark: string; exit_code: number | null }
  | { kind: 'command'; line: string }
  | { kind: 'cwd'; path: string }
  | { kind: 'title'; title: string };

export type Status =
  | { kind: 'in_progress' }
  | { kind: 'success' }
  | { kind: 'failed'; error_type: string | null; message: string | null }
  | { kind: 'blocked'; message: string | null }
  | { kind: 'cancelled' };

export interface Pr {
  number: number;
  title: string;
  url: string;
  branch: string;
  draft: boolean;
  checks: 'none' | 'pass' | 'running' | 'fail';
  /** Panes whose checkout is on this PR's branch. */
  panes: PaneId[];
}

export interface RepoPrs {
  repo: string;
  name: string;
  prs: Pr[];
  error: string | null;
}

export interface NaruPaths {
  brain_note: string | null;
  library_notes: string | null;
  study_root: string | null;
}

export type MetaLevel = 'campaign' | 'adset' | 'ad';
export type MetaPreset = 'today' | 'yesterday' | 'last_7d' | 'last_30d';

export interface MetaAdAccount {
  id: string;
  name: string;
  currency: string;
  /** Minor units per major unit; null for a currency whose budgets the panel will not change. */
  offset: number | null;
  active: boolean;
}

export interface MetaRow {
  id: string;
  name: string;
  level: MetaLevel;
  status: string;
  effective_status: string;
  /** `minor` is in the account currency's minor unit, as Meta keeps it. */
  budget: { kind: 'daily' | 'lifetime'; minor: number } | null;
  /** null when Meta has nothing for the period, never zeros. */
  metrics: { spend: number | null; impressions: number | null; clicks: number | null; ctr: number | null; cpc: number | null } | null;
  outcome: { label: string; count: number; cost: number | null } | null;
}

export interface MetaAdsView {
  token_tail: string | null;
  accounts: MetaAdAccount[] | null;
  account: string | null;
  preset: MetaPreset;
  level: MetaLevel;
  parent: string | null;
  rows: MetaRow[];
  error: string | null;
}

export type MetaAct = { kind: 'status'; from: string; to: 'ACTIVE' | 'PAUSED' } | { kind: 'budget'; field: 'daily' | 'lifetime'; from: number; to: number };

export interface ModelCost {
  model: string;
  /** `null` when the model has no price. */
  cost: number | null;
  calls: number;
  tokens: number;
}

export interface SessionCost {
  title: string;
  agent: string;
  cost: number;
  tokens: number;
  unpriced: number;
  pane: PaneId | null;
}

export interface ProjectCost {
  name: string;
  path: string | null;
  total: number;
  /** Calls with no price, left out of `total`. */
  unpriced: number;
  days: number[];
  models: ModelCost[];
  sessions: number;
  top: SessionCost[];
}

export interface Costs {
  days: string[];
  projects: ProjectCost[];
}

export interface CodeMapFile {
  path: string;
  status: 'changed' | 'new' | 'deleted';
  added: number;
  removed: number;
  /** The file as it is now. */
  lines: string[];
  /** 1-based lines of `lines` that are new. */
  marks: number[];
  /** Removed lines, shown after line `after` (0 = at the top). */
  gone: { after: number; lines: string[] }[];
  skipped: string | null;
}

export interface CodeMap {
  root: string;
  files: CodeMapFile[];
  /** `[importer, imported]` indexes into `files`. */
  edges: [number, number][];
  more: number;
}

export interface DevServer {
  port: number;
  pid: number;
  command: string;
  /** Seconds since it started. */
  age: number;
  pane: PaneId;
}

export interface Session {
  agent: string;
  status: Status;
  session_id: string | null;
  launcher: string | null;
  launch_command: string | null;
  cwd: string | null;
  project: string | null;
  query: string | null;
  response: string | null;
  transcript_path: string | null;
  summary: string | null;
  tool_name: string | null;
  tool_input_preview: string | null;
  plugin_version: string | null;
  seen: boolean;
  /** The user has looked at the pane since its status last changed. */
  read: boolean;
  /** When the status last changed, in Unix milliseconds. */
  since: number | null;
  remote: boolean;
  /** Background subagents still running after its turn ended; it is still at work. */
  subagents?: number;
}

export interface PaneRecord {
  cwd: string | null;
  /** A title the user typed; it wins over the automatic one. */
  title: string | null;
  /** The title haiku gave the pane's conversation. */
  auto_title: string | null;
  /** What the pane's agent is doing, in one line from haiku. */
  summary: string | null;
  session: Session | null;
  running: string | null;
  program_title: string | null;
  /** The pane whose agent started this one with `naru agent spawn --parent here`. */
  parent: PaneId | null;
  node: Node | null;
}

/** A pane running an ssh session to another machine (`naru node connect`). */
export interface Node {
  host: string;
  cwd: string | null;
  resume: string | null;
  first: string | null;
}

/** A file in the document lane, opened by an agent's `naru open` or `naru proof`. */
export interface View {
  id: Id;
  path: string;
  proof: boolean;
  from: PaneId | null;
  seen: boolean;
}

/** A pane's place in its agent team; only panes with something to show are listed. */
export interface PaneTeam {
  id: string | null;
  question: string | null;
  stalled: string | null;
  alert: string | null;
  held: number;
  asleep: boolean;
  not_taken: boolean;
  plan: PlanView | null;
  /** Its plan waits for the user's review while it rests, e.g. `계획 검토 · 4단계`. */
  review: string | null;
}

export type StepState = 'waiting' | 'working' | 'blocked' | 'reported' | 'closed';

export interface PlanView {
  title: string;
  review: boolean;
  steps: { key: string; title: string; state: StepState; child: PaneId | null; child_id: string | null }[];
}

export type LetterKind = 'request' | 'block' | 'report' | 'progress';
export type LetterState = 'held' | 'typed' | 'taken' | 'untaken' | 'dropped';

/** A letter on record, numbered l{id}. */
export interface Letter {
  id: number;
  /** `null` is the user. */
  from: PaneId | null;
  to: PaneId;
  /** Ids and titles when it was sent. */
  from_name: string;
  to_name: string;
  from_title: string;
  to_title: string;
  kind: LetterKind;
  text: string;
  /** Unix seconds. */
  at: number;
  reply_to: number | null;
  state: LetterState;
  why: string | null;
  state_at: number;
  answered_by: number | null;
}

/** One request in an agent's transcript and the agent's last words after it. */
export interface Turn {
  at: string | null;
  request: string;
  reply: string | null;
  /** It answers a question the agent ended its previous turn with. */
  answers: boolean;
}

export type Layout =
  | { kind: 'pane'; pane: PaneId }
  | { kind: 'split'; row: boolean; children: Layout[]; flex: number[] };

export interface Tab {
  id: Id;
  group: Id | null;
  title: string | null;
  color: string | null;
  pinned: boolean;
  background: string | null;
  layout: Layout;
  focused: PaneId;
}

export interface Group {
  id: Id;
  name: string;
  path: string | null;
  color: string | null;
  collapsed: boolean;
  pinned: boolean;
}

export interface Workspace {
  groups: Group[];
  tabs: Tab[];
  active_tab: Id | null;
  panes: Record<PaneId, PaneRecord>;
  views: View[];
}

export type Stage = 'new' | 'studying' | 'understood';

/** Something the user found hard while working, kept in the 학습함 to study later. */
export interface Mark {
  id: number;
  title: string;
  hard: string;
  excerpt: string;
  project: string | null;
  cwd: string | null;
  from_title: string;
  source: { agent: string; session_id: string; transcript: string | null; requests: number | null } | null;
  remote: boolean;
  /** Unix seconds. */
  at: number;
  stage: Stage;
  study_session: string | null;
  note: string | null;
  /** Where the material to study comes from, when it is not simply the conversation. */
  origin: string | null;
  note_exists: boolean;
  /** Pictures the learning session drew, by file name in the mark's material folder. */
  materials: string[];
}

/** What a Memory read found the user did not know, waiting in the 학습함 to be taken. */
export interface Proposal {
  id: number;
  title: string;
  /** The user's words, as they wrote them. */
  quote: string;
  why: 'question' | 'confused' | 'corrected' | 'unknown';
  project: string | null;
  cwd: string | null;
  source: { agent: string; session_id: string; transcript: string | null; requests: number | null };
  /** Unix seconds. */
  at: number;
}

/** `~/.naru/learn-map.json`. A link end is `n:<path>`, `m:<id>` or `p:<id>`. */
export interface LearnMapFile {
  built_on: string;
  notes_mtime: number;
  groups: { name: string; notes: string[]; marks: number[]; proposals: number[] }[];
  links: { a: string; b: string; why: string }[];
  error: string | null;
}

/** Brain's note as the daemon read it. */
export type BrainNote =
  | { state: 'offline' }
  | { state: 'unchanged'; mtime: number }
  | { state: 'found'; front: string; body: string; mtime: number }
  | { state: 'error'; message: string };

export type BrainWritten =
  | { state: 'saved'; mtime: number }
  | { state: 'offline' }
  | { state: 'changed'; front: string; body: string; mtime: number | null }
  | { state: 'error'; message: string };

/** A Claude conversation found by ⌘K search. */
export interface Conversation {
  session_id: string;
  cwd: string;
  first: string;
  last: string;
  /** Unix milliseconds of the transcript's last write. */
  modified: number;
}

/** What a 제안 record says happened. */
export type Signal = 'improvement' | 'failed_tool' | 'agent_wait' | 'label_missed' | 'correction' | 'repeated_request' | 'memory_ignored';

export interface SuggestionRecord {
  id: number;
  session: string;
  turn: string;
  signal: Signal;
  kind: string;
  text: string;
  /** Unix seconds. */
  at: number;
  fresh: boolean;
  said: { personal: boolean; line: string; excerpt: string } | null;
  given: string | null;
}

export type Fix =
  | { kind: 'doc'; title: string; path: string; draft: string }
  | { kind: 'issue'; title: string; body: string }
  | { kind: 'global'; line: string };

export type LineStage = 'new' | 'fixing' | 'ready' | 'no_change' | 'issue' | 'applied' | 'effective' | 'skipped' | 'struck';

export interface SuggestionLine {
  id: number;
  stage: LineStage;
  discomfort: string;
  cause: string;
  fix: Fix | null;
  records: number[];
  skipped_with: number | null;
  skips: number;
  revived: boolean;
  came_back: boolean;
  applied_at: number | null;
  fixing_since: number | null;
  change: { diff: string; added: number; removed: number; created: boolean } | null;
  issue: string | null;
  struck: string | null;
  refusal: string | null;
  held: boolean;
  raised_at: number | null;
  basis: [Signal, string] | null;
  auto: boolean;
  asked: boolean;
}

export type Amount = 'few' | 'normal' | 'many';

export interface AiSettings {
  enabled: boolean;
  suggestions: boolean;
  memory: boolean;
  amount: Amount;
  auto_prepare: boolean;
  read_sessions: boolean;
  intro_seen: boolean;
}

export interface SuggestionProject {
  root: string;
  last_run: string | null;
  last_run_at: number | null;
  last_end: string | null;
  records: SuggestionRecord[];
  lines: SuggestionLine[];
  /** Each summarized session's last turn time. */
  sessions: Record<string, number>;
  settings: AiSettings;
}

export type CardKind = 'rule' | 'decision' | 'lesson' | 'fact' | 'preference';

export interface MemoryCard {
  id: number;
  kind: CardKind;
  title: string;
  body: string;
  aliases: string[];
  sources: string[];
  at: number;
  archived_at: number | null;
  promoted: string[];
}

export interface MemoryStore {
  cards: MemoryCard[];
  conflicts: { card: number; doc: string; says: string }[];
  log: { id: number; at: number; what: string; card: number }[];
}

/** Naru AI for the repository of one pane. */
export interface AiView {
  root: string;
  project: SuggestionProject;
  memory: MemoryStore;
  /** Cards an agent reported out of date with `naru memory stale`; hooks stop giving them. */
  held: Record<number, { revision: number; reason: string; since: number; session: string }>;
  received: Record<number, { how: string; at: number }>;
  used: Record<number, number>;
  docs: string[];
  github: boolean;
  investigating: boolean;
  capturing: boolean;
  claude_missing: boolean;
  fixing: Record<number, PaneId>;
  global: { lines: number; bytes: number; stamp: string; refused: Record<number, string>; error: string | null };
  badge: number;
  /** Lines the user can pick from. */
  choosable: number[];
  auto_waiting: boolean;
}

export type AiAct =
  | { a: 'fix' | 'issue' | 'skip' | 'keep' | 'revert' | 'restore' | 'show_diff' | 'revert_global'; line: number }
  | { a: 'apply_global'; line: number; seen: string }
  | { a: 'settings'; settings: AiSettings }
  | { a: 'archive' | 'revive' | 'promote' | 'release'; card: number }
  | { a: 'undo'; change: number }
  | { a: 'settle'; index: number; card_right: boolean }
  | { a: 'check_now' | 'forget' };

export type AgentKind = 'claude' | 'codex' | 'agy' | 'grok';

/** One limit window; `percent` is 0–100 and `resets_at` is in unix seconds. */
export interface LimitWindow {
  minutes: number;
  percent: number;
  resets_at: number | null;
}

export interface AccountView {
  id: string;
  label: string;
  detail: string | null;
  active: boolean;
  /** Signed in but saved as no account. */
  unsaved: boolean;
  /** The limits last read while this account was signed in; `at` in unix seconds. */
  usage: { windows: LimitWindow[]; at: number } | null;
}

export interface AgentView {
  agent: AgentKind;
  installed: boolean;
  accounts: AccountView[];
  switchable: boolean;
  add: string | null;
  note: string | null;
  error: string | null;
}

export type AccountAct =
  | { a: 'switch' | 'remove'; agent: AgentKind; id: string }
  | { a: 'add' | 'save_current'; agent: AgentKind }
  | { a: 'rename'; agent: AgentKind; id: string; name: string };

export type ServerMsg =
  | { t: 'state'; workspace: Workspace; team: Record<PaneId, PaneTeam> }
  | { t: 'letters'; letters: Letter[] }
  | { t: 'replay' | 'output'; pane: PaneId; data: string }
  | { t: 'osc'; pane: PaneId; osc: Osc }
  | {
      t: 'record';
      pane: PaneId;
      mtime: number | null;
      turns: Turn[] | null;
      cut: boolean;
      reason: string | null;
    }
  | { t: 'decisions'; pane: PaneId; path: string | null; mtime: number | null; text: string | null }
  | { t: 'view_body'; view: Id; kind: 'markdown' | 'text' | 'image' | 'html' | 'error'; mime?: string; text?: string; data?: string }
  | { t: 'learn'; marks: Mark[]; proposals: Proposal[]; memory_on: boolean; error: string | null }
  | { t: 'learn_marked'; id: number }
  | {
      t: 'learn_map';
      map: LearnMapFile | null;
      notes: { path: string; title: string; mtime: number }[];
      offline: boolean;
    }
  | { t: 'learn_failure'; id: number | null; message: string }
  | { t: 'notice'; text: string }
  | { t: 'conversations'; query: string; found: Conversation[] }
  | { t: 'title_candidates'; pane: PaneId; titles: string[]; error: string | null }
  | { t: 'code_map'; pane: PaneId; map: CodeMap | null; error: string | null }
  | { t: 'prs'; repos: RepoPrs[] }
  | ({ t: 'meta_ads' } & MetaAdsView)
  | { t: 'meta_ads_done'; id: string | null; error: string | null }
  | { t: 'paths'; paths: NaruPaths | null; error: string | null }
  | { t: 'costs'; costs: Costs }
  | { t: 'servers'; servers: DevServer[]; error: string | null }
  | { t: 'ai'; pane: PaneId; view: AiView | null; error: string | null }
  | { t: 'accounts'; agents: AgentView[]; notice: string | null; opened: PaneId | null; error: string | null }
  | { t: 'brain'; path: string; note: BrainNote }
  | { t: 'brain_written'; written: BrainWritten }
  /** Base64 PNG by group id. */
  | { t: 'legacy_thumbnails'; thumbnails: Record<string, string> }
  /** The screen settings, on connect before the state. */
  | { t: 'ui'; values: Record<string, string> }
  /** Answers `read_background`: the image at `path` in base64, or why it cannot be shown. */
  | { t: 'background'; path: string; mime: string | null; data: string | null; error: string | null }
  | { t: 'library'; books: Book[]; error: string | null }
  | { t: 'library_translation'; book: string; page: number; text: string | null; error: string | null }
  /** Answers `library_discuss` with the pane of the book's session. */
  | { t: 'library_discussion'; book: string; pane: PaneId }
  | { t: 'library_failure'; message: string }
  /** Answers `marketing_open` and `marketing_check`; `pane` is organic-lab's session. */
  | { t: 'marketing'; url: string | null; up: boolean; pane: PaneId | null; error: string | null }
  | { t: 'library_progress'; book: string; translated: number }
  /** The book's underlines and figures, on `read_library_marks` and after each change. */
  | { t: 'library_marks'; book: string; marks: BookMark[]; figures: Figure[] }
  | { t: 'error'; message: string };

/** A PDF in the 서재. */
export interface Book {
  id: string;
  title: string;
  pages: number;
  /** The page the reader last showed, from 1. */
  page: number;
  added: number;
  /** When the reader last opened it, in Unix milliseconds. */
  opened: number | null;
  /** The claude session that discusses it. */
  session: string | null;
  /** Base64 JPEG of the first page. */
  cover: string | null;
  /** The book's note in the vault. */
  note: string;
  note_exists: boolean;
  /** Pages with text to translate, once the book's text was sent. */
  translatable: number | null;
  translated: number;
}

/** An underline on a translated page, found again by the text around it. */
export interface BookMark {
  id: string;
  page: number;
  quote: string;
  prefix: string;
  suffix: string;
  created: number;
}

/** A diagram drawn after a block of a translated page. */
export interface Figure {
  id: string;
  page: number;
  /** The block it follows when its quote is not found. */
  block: number;
  quote: string;
  context: string;
  prompt: string;
  status: 'drawing' | 'done' | 'failed';
  error: string | null;
  created: number;
  /** Base64 SVG once drawn. */
  svg: string | null;
}

export type ClientMsg =
  | { t: 'new_tab'; group?: Id; cwd?: string }
  | { t: 'split'; pane: PaneId; direction: 'right' | 'down' }
  | { t: 'add_group'; name: string; path?: string }
  | { t: 'rename_group'; group: Id; name: string }
  | { t: 'attach' | 'detach' | 'close_pane' | 'focus' | 'mark_seen'; pane: PaneId }
  | { t: 'set_title'; pane: PaneId; title: string | null }
  | { t: 'input'; pane: PaneId; data: string }
  | { t: 'resize'; pane: PaneId; cols: number; rows: number }
  | { t: 'view_seen' | 'close_view' | 'read_view'; view: Id }
  | { t: 'read_record' | 'read_decisions'; pane: PaneId; mtime: number | null }
  | { t: 'read_learn' }
  | { t: 'read_letters' }
  | { t: 'letter_send'; to: PaneId; text: string }
  | { t: 'letter_retry'; id: number }
  | { t: 'learn_mark'; pane: PaneId; text: string }
  | { t: 'learn_stage'; id: number; stage: Stage }
  | { t: 'learn_hard'; id: number; hard: string; retitle: boolean }
  | { t: 'learn_study' | 'learn_note' | 'learn_open_note'; id: number }
  | { t: 'learn_open_material'; id: number; name: string }
  | { t: 'learn_take' | 'learn_skip'; proposal: number }
  | { t: 'read_learn_map' }
  | { t: 'learn_open_study_note'; path: string }
  | { t: 'open_link'; url: string }
  | { t: 'open_path'; pane: PaneId; path: string }
  | { t: 'code_map'; pane: PaneId }
  | { t: 'prs' }
  | { t: 'meta_ads'; account: string | null; level: MetaLevel; parent: string | null; preset: MetaPreset }
  | { t: 'meta_ads_act'; account: string; id: string; act: MetaAct }
  | { t: 'meta_ads_token'; token: string | null }
  | { t: 'paths' }
  | { t: 'paths_set'; paths: NaruPaths }
  | { t: 'costs' }
  | { t: 'plan_approve'; pane: PaneId }
  | { t: 'servers' }
  | { t: 'stop_server'; pid: number }
  | { t: 'search_conversations'; query: string }
  | { t: 'title_candidates'; pane: PaneId }
  | { t: 'ai'; pane: PaneId }
  | { t: 'ai_act'; pane: PaneId; act: AiAct }
  | { t: 'resume_conversation'; session_id: string; cwd: string; here: boolean }
  | { t: 'brain_read'; mtime: number | null }
  | { t: 'brain_write'; body: string; base: number | null }
  | { t: 'brain_open' }
  | { t: 'accounts' }
  | { t: 'account_act'; act: AccountAct }
  | { t: 'legacy_thumbnails' }
  | { t: 'ui_set'; key: string; value: string | null }
  /** `image` is a base64 PNG or JPEG; `null` clears the background. */
  | { t: 'set_background'; tab: Id; image: string | null }
  | { t: 'read_background'; tab: Id }
  | { t: 'read_library' }
  /** `cover` is a base64 JPEG of the first page, `pdf` the file in base64. */
  | ({ t: 'library_add'; name: string; pages: number; cover: string; texts: string[] } & ({ path: string } | { pdf: string }))
  | { t: 'library_texts'; book: string; texts: string[] }
  | { t: 'read_library_marks'; book: string }
  | { t: 'library_mark'; book: string; page: number; quote: string; prefix: string; suffix: string }
  | { t: 'library_unmark'; book: string; mark: string }
  | { t: 'library_draw'; book: string; page: number; block: number; quote: string; context: string; prompt: string }
  | { t: 'library_redraw' | 'library_undraw'; book: string; figure: string }
  | { t: 'library_ask'; book: string; page: number; quote: string; question: string }
  | { t: 'library_remove' | 'library_open' | 'library_discuss' | 'library_open_note'; book: string }
  | { t: 'marketing_open' | 'marketing_check' | 'marketing_start_server' }
  | { t: 'library_page'; book: string; page: number }
  /** `text` is the page's extracted text; a saved translation answers unless `again`. */
  | { t: 'library_translate'; book: string; page: number; text: string; again: boolean };

/**
 * The daemon's WebSocket; reconnects on its own and tells listeners each time it opens. Listen
 * before `start`, or the first snapshot can arrive before anyone hears it.
 */
export class Connection {
  private ws?: WebSocket;
  private closed = false;
  private readonly messageListeners = new Set<(message: ServerMsg) => void>();
  private readonly openListeners = new Set<() => void>();
  private readonly closeListeners = new Set<() => void>();

  private readonly url: string;

  constructor(url: string) {
    this.url = url;
  }

  start(): void {
    this.closed = false;
    this.connect();
  }

  close(): void {
    this.closed = true;
    const ws = this.ws;
    this.ws = undefined;
    ws?.close();
  }

  get isOpen(): boolean {
    return this.ws?.readyState === WebSocket.OPEN;
  }

  /** Where the daemon serves a shelved book's PDF. */
  bookUrl(book: string): string {
    const url = new URL(this.url);
    url.protocol = url.protocol === 'wss:' ? 'https:' : 'http:';
    url.pathname = `/library/${encodeURIComponent(book)}/pdf`;
    return url.toString();
  }

  send(message: ClientMsg): void {
    if (this.isOpen) this.ws!.send(JSON.stringify(message));
  }

  onMessage(listener: (message: ServerMsg) => void): () => void {
    this.messageListeners.add(listener);
    return () => this.messageListeners.delete(listener);
  }

  onOpen(listener: () => void): () => void {
    this.openListeners.add(listener);
    return () => this.openListeners.delete(listener);
  }

  onClose(listener: () => void): () => void {
    this.closeListeners.add(listener);
    return () => this.closeListeners.delete(listener);
  }

  // A socket that is no longer `this.ws` stays silent: closed and restarted quickly (React runs
  // effects twice in development), the old socket's late close would otherwise open a second
  // live socket, and every pane attached on both would print its output twice.
  private connect(): void {
    const ws = new WebSocket(this.url);
    const current = () => ws === this.ws;
    ws.onopen = () => current() && this.openListeners.forEach((listener) => listener());
    ws.onmessage = (event) => {
      if (!current()) return;
      const message = JSON.parse(event.data) as ServerMsg;
      this.messageListeners.forEach((listener) => listener(message));
    };
    ws.onclose = () => {
      if (!current()) return;
      this.closeListeners.forEach((listener) => listener());
      setTimeout(() => current() && !this.closed && this.connect(), 1000);
    };
    this.ws = ws;
  }
}

export function decodeBase64(data: string): Uint8Array<ArrayBuffer> {
  return Uint8Array.from(atob(data), (char) => char.charCodeAt(0));
}
