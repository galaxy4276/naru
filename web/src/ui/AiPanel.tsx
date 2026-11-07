import { useEffect, useState } from 'react';
import { Check, ChevronDown, ChevronRight, Settings } from 'lucide-react';
import type {
  AiAct,
  AiSettings,
  AiView,
  Amount,
  CardKind,
  Connection,
  PaneId,
  Signal,
  SuggestionLine,
  SuggestionProject,
  SuggestionRecord,
  Workspace,
} from '@/connection';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { askLine, basename, timeLabel, waitLabel } from './model';
import { SidePanel } from './SidePanel';

const OPEN_POLL_MS = 3000;
const CLOSED_POLL_MS = 15000;
const WATCH_SESSIONS = 5;
const EVIDENCE_ROWS = 10;
const FIRST_CARDS = 5;
const LOG_ROWS = 10;
const UNUSED_SECONDS = 30 * 24 * 60 * 60;

const kindLabel: Record<string, string> = {
  no_doc: '문서 없음',
  searched_again: '다시 찾음',
  wrong_guess: '잘못 짐작',
  tool_friction: '도구·설정 불편',
};

const signalLabel: Record<Signal, string> = {
  improvement: '개선할 점',
  failed_tool: '실패한 명령',
  agent_wait: '에이전트 막힘',
  label_missed: '세션에 남은 문제',
  correction: '내 정정',
  repeated_request: '반복한 요청',
  memory_ignored: '안 지킨 기억',
};

const basisTitle: Record<Signal, (tag: string) => string> = {
  improvement: (tag) => `같은 곳에서 계속 헤매요: ${tag}`,
  failed_tool: (tag) => `같은 명령이 계속 실패해요: ${tag}`,
  agent_wait: (tag) => `에이전트가 같은 곳에서 계속 멈춰요: ${tag}`,
  label_missed: (tag) => `같은 문제가 계속 남아요: ${tag}`,
  correction: (tag) => `같은 정정을 계속 하고 있어요: ${tag}`,
  repeated_request: (tag) => `같은 요청을 계속 하고 있어요: ${tag}`,
  memory_ignored: (tag) => `에이전트가 받은 기억을 계속 놓쳐요: ${tag}`,
};

const cardKindLabel: Record<CardKind, string> = {
  rule: '규칙',
  decision: '결정',
  lesson: '배운 것',
  fact: '사실',
  preference: '선호',
};

const amounts: [Amount, string, string][] = [
  ['few', '적게', '크게 반복된 확실한 것만 · 주 1개까지'],
  ['normal', '보통', '여러 세션에서 시간을 잡아먹은 것 · 주 1~3개'],
  ['many', '많이', '약하게 반복되는 것도 · 주 5개까지'],
];

const SUGGESTION_STEPS = [
  '세션에서 에이전트가 자주 막히는 곳을 찾아요.',
  '같은 실수가 반복되지 않게 고칠 문서와 문구를 제안해요.',
  '고르면 에이전트가 그 파일을 고쳐 두고, 바뀐 것을 보여 줘요.',
];

const MEMORY_STEPS = [
  '세션이 30분 조용하거나 pane이 닫히면 대화에서 정한 것을 기억으로 남겨요.',
  '새 세션은 기억 목록을, 요청마다 맞는 기억의 본문을 받아요.',
  '문서와 다른 기억은 어느 쪽이 맞는지 물어봐요.',
];

/**
 * Naru AI for the repository of `pane`, asked again every few seconds: often while the panel is
 * open, rarely for the rail's badge.
 */
export function useAi(conn: Connection | null, pane: PaneId | null, open: boolean) {
  const [state, setState] = useState<{ pane: PaneId | null; view: AiView | null; error: string | null }>({
    pane: null,
    view: null,
    error: null,
  });
  useEffect(() => {
    if (!conn || !pane) return;
    const off = conn.onMessage((message) => {
      if (message.t !== 'ai' || message.pane !== pane) return;
      // A failed press keeps showing what was there.
      setState((old) => ({ pane, view: message.error && old.pane === pane ? old.view : message.view, error: message.error }));
    });
    const ask = () => conn.send({ t: 'ai', pane });
    ask();
    const timer = setInterval(ask, open ? OPEN_POLL_MS : CLOSED_POLL_MS);
    return () => {
      off();
      clearInterval(timer);
    };
  }, [conn, pane, open]);
  return state.pane === pane ? state : { pane, view: null, error: null };
}

function lineTitle(line: SuggestionLine): string {
  if (line.discomfort) return line.discomfort;
  return line.basis ? basisTitle[line.basis[0]](line.basis[1]) : '';
}

function sessionsSince(project: SuggestionProject, at: number): number {
  return Object.values(project.sessions).filter((last) => last > at).length;
}

/** A line's records, newest first, one per session. */
function evidence(line: SuggestionLine, project: SuggestionProject): SuggestionRecord[] {
  const seen = new Set<string>();
  return line.records
    .map((id) => project.records.find((record) => record.id === id))
    .filter((record): record is SuggestionRecord => record !== undefined)
    .sort((a, b) => b.at - a.at)
    .filter((record) => !seen.has(record.session) && seen.add(record.session))
    .slice(0, EVIDENCE_ROWS);
}

function Switch({ on, onChange, label, disabled }: { on: boolean; onChange: () => void; label: string; disabled?: boolean }) {
  return (
    <button
      role="switch"
      aria-checked={on}
      aria-label={label}
      disabled={disabled}
      onClick={onChange}
      className={cn(
        'relative h-5 w-9 shrink-0 rounded-full transition-colors disabled:opacity-40',
        on ? 'bg-primary' : 'bg-input',
      )}
    >
      <i className={cn('absolute top-0.5 size-4 rounded-full bg-background shadow transition-all', on ? 'left-[18px]' : 'left-0.5')} />
    </button>
  );
}

function Explainer({ title, steps, children }: { title: string; steps: string[]; children?: React.ReactNode }) {
  return (
    <div className="mx-3 rounded-xl bg-muted px-4 py-4">
      <div className="flex items-center gap-2">
        <b className="flex-1 font-semibold">{title}</b>
        {children}
      </div>
      <ol className="mt-2 space-y-1 text-[12px] text-muted-foreground">
        {steps.map((step) => (
          <li key={step}>{step}</li>
        ))}
      </ol>
    </div>
  );
}

function Toggle({ open, label, onClick }: { open: boolean; label: string; onClick: () => void }) {
  return (
    <button onClick={onClick} className="flex items-center gap-1 px-4 pt-3 pb-1 text-[12px] font-semibold text-muted-foreground hover:text-foreground">
      {open ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
      {label}
    </button>
  );
}

/** A section that starts folded and opens on its label. */
function Fold({ label, children }: { label: string; children: React.ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Toggle open={open} label={label} onClick={() => setOpen((was) => !was)} />
      {open && children}
    </>
  );
}

function Buttons({ children }: { children: React.ReactNode }) {
  return <div className="mt-2 flex flex-wrap items-center gap-1.5">{children}</div>;
}

function Note({ children, turn }: { children: React.ReactNode; turn?: boolean }) {
  return <p className={cn('text-[12px] text-muted-foreground', turn && 'text-turn')}>{children}</p>;
}

interface Props {
  conn: Connection;
  ws: Workspace;
  pane: PaneId | null;
  view: AiView | null;
  error: string | null;
  onOpenSession: (pane: PaneId) => void;
  onClose: () => void;
}

/** Naru AI: 제안 (where agents keep losing time, and the fix) and Memory (what was settled). */
export function AiPanel({ conn, ws, pane, view, error, onOpenSession, onClose }: Props) {
  const [tab, setTab] = useState<'suggestions' | 'memory'>('suggestions');
  const [sheet, setSheet] = useState(false);
  const [intro, setIntro] = useState(false);
  const act = (a: AiAct) => pane && conn.send({ t: 'ai_act', pane, act: a });
  const settings = view?.project.settings;
  const set = (edit: Partial<AiSettings>) => settings && act({ a: 'settings', settings: { ...settings, ...edit } });

  // The first open says what the tool sends where, once per project.
  useEffect(() => {
    if (settings?.enabled && !settings.intro_seen) {
      setIntro(true);
      set({ intro_seen: true });
    }
  }, [settings?.enabled, settings?.intro_seen]);

  const name = view ? basename(view.root) : null;
  const conflicts = view && settings?.enabled && settings.memory ? view.memory.conflicts.length : 0;
  const head = view && settings && (
    <Button variant="ghost" size="icon-xs" onClick={() => setSheet((open) => !open)} aria-label="Naru AI 설정" aria-pressed={sheet}>
      <Settings />
    </Button>
  );

  let body: React.ReactNode;
  if (!view || !settings) {
    body = <p className="px-4 py-6 text-center text-muted-foreground">{error ?? '앞 pane이 git 저장소에 있을 때 Naru AI가 보여요.'}</p>;
  } else if (sheet) {
    body = <Sheet view={view} set={set} act={act} />;
  } else if (!settings.enabled) {
    body = (
      <Explainer title="Naru AI가 꺼져 있어요" steps={['제안 · 에이전트가 자주 막히는 곳을 찾아 고칠 것을 제안해요', 'Memory · 대화에서 정한 것을 기억해 모든 에이전트에게 줘요']}>
        <Switch on={false} label="Naru AI 켜기" onChange={() => set({ enabled: true })} />
      </Explainer>
    );
  } else {
    body = (
      <>
        {intro && (
          <div className="mx-3 mb-2 rounded-xl border px-4 py-3">
            <b className="font-semibold">조사에는 내 구독 사용량을 써요.</b>
            <Note>세션 일부와 저장소 파일을 claude로 보내요. 비밀 값은 가려서 보내요.</Note>
          </div>
        )}
        <div className="mx-3 mb-2 grid grid-cols-2 gap-1 rounded-[10px] bg-muted p-1 text-[12.5px]">
          {(
            [
              ['suggestions', '제안', view.choosable.length],
              ['memory', 'Memory', conflicts],
            ] as const
          ).map(([key, label, count]) => (
            <button
              key={key}
              onClick={() => setTab(key)}
              className={cn('flex h-7 items-center justify-center gap-1.5 rounded-lg text-muted-foreground', tab === key && 'bg-background font-semibold text-foreground')}
            >
              {label}
              {count > 0 && <span className="min-w-4 rounded-full bg-turn px-1 text-[10.5px] leading-4 font-semibold text-white">{count}</span>}
            </button>
          ))}
        </div>
        {tab === 'suggestions' ? (
          <Suggestions ws={ws} view={view} act={act} set={set} onOpenSession={onOpenSession} />
        ) : (
          <MemoryTab view={view} act={act} set={set} />
        )}
      </>
    );
  }

  return (
    <SidePanel title="Naru AI" sub={name ?? undefined} onClose={onClose} actions={head}>
      {error && view && <p className="px-4 pb-2 text-[12px] text-destructive">{error}</p>}
      <div className="pb-4">{body}</div>
    </SidePanel>
  );
}

type Send = (a: AiAct) => void;
type Set = (edit: Partial<AiSettings>) => void;

/** ⚙: what 제안 and Memory share, then 제안's settings, then Memory's. */
function Sheet({ view, set, act }: { view: AiView; set: Set; act: Send }) {
  const settings = view.project.settings;
  const [forgetArmed, setForgetArmed] = useState(false);
  const skipped = view.project.lines.filter((line) => line.stage === 'skipped').reverse().slice(0, 50);
  const row = 'flex items-center gap-3 px-4 py-2';
  return (
    <div className="text-[13px]">
      <div className={row}>
        <b className="flex-1 font-semibold">Naru AI</b>
        <Switch on={settings.enabled} label="Naru AI" onChange={() => set({ enabled: !settings.enabled })} />
      </div>
      <div className={row}>
        <span className="flex-1">사용하는 AI</span>
        <span className="text-muted-foreground">claude · sonnet</span>
      </div>
      <div className="px-4 pb-2">
        <Note>조사와 기억 분석에는 내 구독 사용량을 써요. 세션 일부와 저장소 파일을 보내고, 비밀 값은 가려서 보내요.</Note>
      </div>

      <div className="mx-3 my-2 border-t" />
      <div className={row}>
        <b className="flex-1 font-semibold">제안</b>
        <Switch on={settings.suggestions} disabled={!settings.enabled} label="제안" onChange={() => set({ suggestions: !settings.suggestions })} />
      </div>
      <div className="px-4">
        <Note>세션에서 에이전트가 자주 막히는 곳을 찾아 고칠 것을 제안해요</Note>
      </div>
      <div className="px-4 pt-3">
        <span>제안 양</span>
        <div className="mt-1.5 grid grid-cols-3 gap-1 rounded-[10px] bg-muted p-1 text-[12.5px]">
          {amounts.map(([amount, label]) => (
            <button
              key={amount}
              onClick={() => set({ amount })}
              className={cn('h-7 rounded-lg text-muted-foreground', settings.amount === amount && 'bg-background font-semibold text-foreground')}
            >
              {label}
            </button>
          ))}
        </div>
        <Note>{amounts.find(([amount]) => amount === settings.amount)?.[2]}</Note>
      </div>
      <div className={cn(row, 'pt-3')}>
        <span className="flex-1">반영 방식 · 자동으로 준비해 두기</span>
        <Switch on={settings.auto_prepare} label="자동으로 준비해 두기" onChange={() => set({ auto_prepare: !settings.auto_prepare })} />
      </div>
      <div className="px-4">
        <Note>{settings.auto_prepare ? '알아서 그 파일을 고쳐 두고, 그대로 둘지는 내가 정해요' : '버튼을 누르면 그 파일을 고쳐 두거나 이슈를 열어요'}</Note>
      </div>
      <div className={cn(row, 'pt-3')}>
        <span className="flex-1">읽을 곳 · 세션 기록</span>
        <Switch on={settings.read_sessions} label="세션 기록 읽기" onChange={() => set({ read_sessions: !settings.read_sessions })} />
      </div>
      <div className="px-4">
        <Note>실패한 명령 · 에이전트 막힘 · 내 정정 · 반복한 요청 · 안 지킨 기억 · 세션에 남은 문제 · 개선할 점</Note>
        {!settings.memory && <Note>Memory가 꺼져 있어 내 정정과 반복한 요청은 읽지 않아요</Note>}
      </div>
      {skipped.length > 0 && (
        <>
          <Fold label={`넘어간 제안 ${skipped.length}개`}>
            {skipped.map((line) => (
              <div key={line.id} className="flex items-center gap-2 px-4 py-1.5">
                <div className="min-w-0 flex-1">
                  <p className="truncate">{lineTitle(line)}</p>
                  <Note>{line.skips === 1 && line.skipped_with !== null ? '다시 쌓이면 한 번 올라와요' : '다시 올라오지 않아요'}</Note>
                </div>
                <Button variant="outline" size="xs" onClick={() => act({ a: 'restore', line: line.id })}>
                  되살리기
                </Button>
              </div>
            ))}
          </Fold>
        </>
      )}

      <div className="mx-3 my-2 border-t" />
      <div className={row}>
        <b className="flex-1 font-semibold">Memory</b>
        <Switch on={settings.memory} disabled={!settings.enabled} label="Memory" onChange={() => set({ memory: !settings.memory })} />
      </div>
      <div className="px-4">
        <Note>대화에서 정한 것을 기억해 모든 에이전트에게 줘요</Note>
        <Button
          variant={forgetArmed ? 'destructive' : 'outline'}
          size="xs"
          className="mt-2"
          onClick={() => {
            if (forgetArmed) act({ a: 'forget' });
            setForgetArmed((armed) => !armed);
          }}
        >
          {forgetArmed ? '한 번 더 누르면 이 프로젝트의 기억을 지워요' : '이 프로젝트의 기억 지우기'}
        </Button>
      </div>
    </div>
  );
}

function Suggestions({ ws, view, act, set, onOpenSession }: { ws: Workspace; view: AiView; act: Send; set: Set; onOpenSession: (pane: PaneId) => void }) {
  const project = view.project;
  if (!project.settings.suggestions) {
    return (
      <Explainer title="제안이 꺼져 있어요" steps={SUGGESTION_STEPS}>
        <Switch on={false} label="제안 켜기" onChange={() => set({ suggestions: true })} />
      </Explainer>
    );
  }
  const picks = new Set(view.choosable);
  const choose = project.lines.filter((line) => picks.has(line.id));
  const applying = project.lines.filter((line) => line.stage === 'fixing' || line.stage === 'issue');
  const applied = project.lines.filter((line) => line.stage === 'applied' || line.stage === 'effective');
  const effective = applied.filter((line) => line.stage === 'effective').length;
  const status = view.investigating
    ? '조사하는 중…'
    : view.claude_missing
      ? '조사 못 함 · claude를 찾지 못했어요'
      : project.last_end === 'failed'
        ? '조사 못 함'
        : project.last_run_at
          ? `${timeLabel(project.last_run_at)} 조사 · 기록 ${project.records.length}개`
          : null;
  // A session is open while some pane of this machine runs it.
  const sessionPane = (session: string) =>
    Object.entries(ws.panes).find(([, record]) => !record.node && record.session?.session_id === session)?.[0] ?? null;
  const row = (line: SuggestionLine) => (
    <Row key={line.id} ws={ws} view={view} line={line} act={act} sessionPane={sessionPane} onOpenSession={onOpenSession} />
  );
  const empty = choose.length === 0 && applying.length === 0 && applied.length === 0;
  return (
    <div className="text-[13px]">
      {status && (
        <p className={cn('px-4 pb-2 text-[12px] text-muted-foreground', status.startsWith('조사 못 함') && 'text-turn')}>{status}</p>
      )}
      {empty && !view.investigating ? (
        <Explainer title={project.last_run ? '오늘은 제안 없음' : '다음에 Naru를 열 때 첫 조사'} steps={SUGGESTION_STEPS}>
          <span className="text-[12px] text-muted-foreground">기록 {project.records.length}개</span>
        </Explainer>
      ) : null}
      {choose.length > 0 && <h3 className="px-4 pt-2 pb-1 text-[12px] font-semibold text-muted-foreground">고를 것 {choose.length}</h3>}
      {choose.map(row)}
      {applying.length > 0 && <h3 className="px-4 pt-3 pb-1 text-[12px] font-semibold text-muted-foreground">반영 중 {applying.length}</h3>}
      {applying.map(row)}
      {applied.length > 0 && <Fold label={`반영됨 ${applied.length} · 효과 있었음 ${effective}`}>{applied.map(row)}</Fold>}
    </div>
  );
}

function Mark({ stage }: { stage: SuggestionLine['stage'] }) {
  if (stage === 'fixing') return <i className="mt-1 size-2.5 shrink-0 animate-spin rounded-full border-2 border-muted-foreground border-t-transparent" />;
  const on = stage === 'new' || stage === 'ready' || stage === 'no_change';
  return <i className={cn('mt-1.5 size-2 shrink-0 rounded-full', on ? 'bg-turn' : 'bg-muted-foreground/40')} />;
}

function Row({
  ws,
  view,
  line,
  act,
  sessionPane,
  onOpenSession,
}: {
  ws: Workspace;
  view: AiView;
  line: SuggestionLine;
  act: Send;
  sessionPane: (session: string) => PaneId | null;
  onOpenSession: (pane: PaneId) => void;
}) {
  const [open, setOpen] = useState(false);
  const project = view.project;
  const id = line.id;
  const fix = line.fix;
  const press = (a: AiAct, label: React.ReactNode, variant?: 'outline' | 'ghost') => (
    <Button variant={variant} size="xs" onClick={() => act(a)}>
      {label}
    </Button>
  );
  const skip = press({ a: 'skip', line: id }, '넘어가기', 'ghost');
  let detail: React.ReactNode = null;
  if (fix?.kind === 'doc' && line.stage === 'new') {
    detail = (
      <>
        <p className="mt-1 font-mono text-[11px] text-muted-foreground">{fix.path}</p>
        <p className="mt-1 rounded-lg bg-muted px-2.5 py-2 text-[12px] whitespace-pre-wrap">{fix.draft}</p>
        <Buttons>
          {press({ a: 'fix', line: id }, `${basename(fix.path)} 고치기`)}
          {skip}
        </Buttons>
      </>
    );
  } else if (fix?.kind === 'issue' && line.stage === 'new') {
    detail = (
      <Buttons>
        {view.github ? press({ a: 'issue', line: id }, '이슈로 남기기') : <Note>GitHub 연결 필요</Note>}
        {skip}
      </Buttons>
    );
  } else if (fix?.kind === 'doc' && line.stage === 'ready' && line.change) {
    const change = line.change;
    detail = (
      <Buttons>
        {press({ a: 'show_diff', line: id }, `바뀐 것 보기 · ${fix.path} +${change.added} −${change.removed}`, 'outline')}
        {press({ a: 'keep', line: id }, '그대로 두기')}
        {press({ a: 'revert', line: id }, '되돌리기', 'ghost')}
      </Buttons>
    );
  } else if (line.stage === 'no_change') {
    detail = (
      <>
        <Note>바뀐 것 없이 끝났어요</Note>
        <Buttons>
          {press({ a: 'fix', line: id }, '다시 시도')}
          {skip}
        </Buttons>
      </>
    );
  } else if (fix?.kind === 'global' && line.stage === 'new') {
    const refused = view.global.refused[id];
    detail = (
      <>
        <Note>Naru 전역 지시에 들어갈 줄</Note>
        <p className="mt-1 rounded-lg bg-muted px-2.5 py-2 text-[12px]">{fix.line}</p>
        {line.came_back && <Note turn>넣은 뒤에도 또 말했어요</Note>}
        {view.global.error ? (
          <Note turn>{view.global.error}</Note>
        ) : (
          <Buttons>
            {refused ? (
              <Note turn>{refused}</Note>
            ) : (
              <>
                {press({ a: 'apply_global', line: id, seen: view.global.stamp }, 'Naru 전역 지시에 넣기')}
                <span className="text-[11px] text-muted-foreground">
                  {view.global.lines + 1}번째 줄 · {view.global.bytes}B
                </span>
              </>
            )}
            {skip}
          </Buttons>
        )}
      </>
    );
  } else if (fix?.kind === 'global' && (line.stage === 'applied' || line.stage === 'effective')) {
    detail = (
      <div className="mt-1 flex items-center gap-2">
        <Note>{line.stage === 'effective' ? '전역 지시에 넣은 뒤로 다시 말하지 않았어요' : '전역 지시에 넣었어요 · 새 세션부터 받아요'}</Note>
        {press({ a: 'revert_global', line: id }, '되돌리기', 'ghost')}
      </div>
    );
  } else if (line.stage === 'fixing') {
    const agentPane = view.fixing[id];
    const session = agentPane ? ws.panes[agentPane]?.session : null;
    const question = session?.status.kind === 'blocked' ? askLine(session) : null;
    detail = (
      <div className="mt-1 flex items-center gap-2">
        {question ? <Note turn>에이전트가 물어요 · {question}</Note> : <Note>고치는 중 · {waitLabel(line.fixing_since && line.fixing_since * 1000, Date.now())}</Note>}
        {agentPane && (
          <Button variant="ghost" size="xs" onClick={() => onOpenSession(agentPane)}>
            {question ? '세션으로 가서 답하기' : '에이전트 ↗'}
          </Button>
        )}
      </div>
    );
  } else if (line.stage === 'issue') {
    detail = <Note>이슈 열림 · {line.issue}</Note>;
  } else if (line.stage === 'applied') {
    detail = (
      <Note>
        지켜보는 중 · 세션 {Math.min(WATCH_SESSIONS, line.applied_at ? sessionsSince(project, line.applied_at) : 0)}/{WATCH_SESSIONS}
      </Note>
    );
  } else if (line.stage === 'effective') {
    detail = (
      <p className="mt-1 flex items-center gap-1 text-[12px] font-semibold">
        <Check className="size-3.5" />
        효과 있었음
      </p>
    );
  }
  const records = open ? evidence(line, project) : [];
  return (
    <div className="mx-3 mb-2 flex gap-2.5 rounded-xl border px-3 py-2.5">
      <Mark stage={line.stage} />
      <div className="min-w-0 flex-1">
        <p className="font-semibold">{lineTitle(line)}</p>
        {line.revived && <Note turn>넘어갔지만 다시 생겼어요</Note>}
        {line.came_back && fix?.kind === 'doc' && <Note turn>문서로는 안 됐어요</Note>}
        {line.cause && <Note>놓친 것 · {line.cause}</Note>}
        {detail}
        {line.stage === 'new' && !line.auto && project.settings.auto_prepare && view.auto_waiting && (
          <Note>자동으로 준비한 수정의 결정을 기다려요</Note>
        )}
        {line.auto && line.stage !== 'applied' && line.stage !== 'effective' && (
          <span className="mt-1 inline-block rounded bg-muted px-1.5 text-[10.5px] text-muted-foreground">자동</span>
        )}
        {line.refusal && <Note turn>{line.refusal}</Note>}
        {line.records.length > 0 && (
          <button onClick={() => setOpen((was) => !was)} className="mt-1.5 text-[11.5px] text-muted-foreground hover:text-foreground">
            {open ? '근거 접기' : `근거 ${line.records.length}개 · 펼치기`}
          </button>
        )}
        {records.map((record) => {
          const at = sessionPane(record.session);
          return (
            <button
              key={record.id}
              disabled={!at}
              onClick={() => at && onOpenSession(at)}
              className={cn('mt-1.5 block w-full rounded-lg bg-muted px-2.5 py-1.5 text-left text-[12px]', !at && 'opacity-50')}
            >
              <span className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                <span className="rounded bg-background px-1">{record.signal === 'improvement' ? (kindLabel[record.kind] ?? kindLabel.tool_friction) : signalLabel[record.signal]}</span>
                {at ? '세션' : '세션 · 닫힘'}
                <span className="ml-auto">{timeLabel(record.at)}</span>
              </span>
              <span className="mt-0.5 block">{record.text}</span>
              {record.said && <span className="block text-muted-foreground">“{record.said.excerpt}”</span>}
              {record.given && <span className="block text-[11px] text-muted-foreground">{record.given === 'body' ? '본문으로 받음' : '목록으로 받음'}</span>}
            </button>
          );
        })}
      </div>
    </div>
  );
}

function MemoryTab({ view, act, set }: { view: AiView; act: Send; set: Set }) {
  const [moreOpen, setMoreOpen] = useState(false);
  if (!view.project.settings.memory) {
    return (
      <Explainer title="Memory가 꺼져 있어요" steps={MEMORY_STEPS}>
        <Switch on={false} label="Memory 켜기" onChange={() => set({ memory: true })} />
      </Explainer>
    );
  }
  const memory = view.memory;
  const used = (id: number) => view.used[id] ?? 0;
  const active = memory.cards.filter((card) => card.archived_at === null).sort((a, b) => used(b.id) - used(a.id));
  const archived = memory.cards.filter((card) => card.archived_at !== null);
  const now = Date.now() / 1000;
  const card = (id: number) => memory.cards.find((found) => found.id === id);
  return (
    <div className="text-[13px]">
      <div className="flex items-center px-4 pb-2">
        <b className="flex-1 font-semibold">기억 {active.length}</b>
        {view.capturing ? (
          <Note>세션을 읽는 중</Note>
        ) : (
          <Button variant="outline" size="xs" onClick={() => act({ a: 'check_now' })}>
            다시 확인
          </Button>
        )}
      </div>
      {memory.conflicts.map((conflict, index) => {
        const found = card(conflict.card);
        if (!found) return null;
        return (
          <div key={`${conflict.card}-${conflict.doc}`} className="mx-3 mb-2 rounded-xl bg-turn-soft px-3 py-2.5">
            <Note turn>문서와 다름 · 어느 쪽이 맞나요?</Note>
            <p className="font-semibold">
              m{found.id} {found.title}
            </p>
            <Note>
              {conflict.doc}: {conflict.says}
            </Note>
            <div className="mt-2 flex gap-1.5">
              <Button size="xs" onClick={() => act({ a: 'settle', index, card_right: true })}>
                기억이 맞음
              </Button>
              <Button variant="outline" size="xs" onClick={() => act({ a: 'settle', index, card_right: false })}>
                문서가 맞음
              </Button>
            </div>
          </div>
        );
      })}
      {active.length === 0 && archived.length === 0 && <Explainer title="아직 기억이 없어요" steps={MEMORY_STEPS} />}
      {active.slice(0, moreOpen ? active.length : FIRST_CARDS).map((found) => {
        const receipt = view.received[found.id];
        const hold = view.held[found.id];
        const openDocs = view.docs.filter((doc) => !found.promoted.includes(doc)).length;
        return (
          <div key={found.id} className="mx-3 mb-2 rounded-xl border px-3 py-2.5">
            <p className="font-semibold">
              m{found.id} {found.title}
            </p>
            <p className="mt-0.5 line-clamp-3 text-[12px] text-muted-foreground">{found.body}</p>
            {hold && (
              <p className="mt-1.5 rounded-lg bg-muted px-2 py-1 text-[12px]">
                <b>멈춤</b> · {timeLabel(hold.since)}에 에이전트가 사용자 말과 다르다고 알렸어요: {hold.reason}
              </p>
            )}
            <div className="mt-1.5 flex flex-wrap gap-1 text-[11px] text-muted-foreground">
              <span className="rounded bg-muted px-1.5">{cardKindLabel[found.kind]}</span>
              {receipt && (
                <span className="rounded bg-muted px-1.5">
                  {receipt.how === 'body' ? '본문으로 받음' : '목록으로 받음'} · {timeLabel(receipt.at)}
                </span>
              )}
              {now - used(found.id) >= UNUSED_SECONDS && <span className="rounded bg-turn-soft px-1.5 text-turn">30일 안 씀</span>}
              {found.promoted.length > 0 && <span className="rounded bg-muted px-1.5">{found.promoted.join(', ')}에 반영함</span>}
            </div>
            <div className="mt-2 flex gap-1.5">
              {openDocs > 0 && (
                <Button variant="outline" size="xs" onClick={() => act({ a: 'promote', card: found.id })}>
                  {openDocs}개 문서에 반영
                </Button>
              )}
              {hold && (
                <Button variant="outline" size="xs" onClick={() => act({ a: 'release', card: found.id })}>
                  다시 쓰기
                </Button>
              )}
              <Button variant="ghost" size="xs" onClick={() => act({ a: 'archive', card: found.id })}>
                보관
              </Button>
            </div>
          </div>
        );
      })}
      {active.length > FIRST_CARDS && (
        <Toggle open={moreOpen} label={moreOpen ? '접기' : `그 외 ${active.length - FIRST_CARDS}`} onClick={() => setMoreOpen((open) => !open)} />
      )}
      {archived.length > 0 && (
        <Fold label={`보관한 기억 ${archived.length}`}>
          {archived.map((found) => (
          <div key={found.id} className="flex items-center gap-2 px-4 py-1.5">
            <span className="min-w-0 flex-1 truncate text-muted-foreground">
              m{found.id} {found.title}
            </span>
            <Button variant="outline" size="xs" onClick={() => act({ a: 'revive', card: found.id })}>
              되살리기
            </Button>
          </div>
          ))}
        </Fold>
      )}
      {memory.log.length > 0 && (
        <Fold label={`바뀐 기록 ${memory.log.length}`}>
          {memory.log
          .slice(-LOG_ROWS)
          .reverse()
          .map((change) => (
            <div key={change.id} className="flex items-center gap-2 px-4 py-1">
              <span className="min-w-0 flex-1 truncate text-[12px] text-muted-foreground">
                {change.what} · {timeLabel(change.at)}
              </span>
              <Button variant="ghost" size="xs" onClick={() => act({ a: 'undo', change: change.id })}>
                되돌리기
              </Button>
            </div>
          ))}
        </Fold>
      )}
    </div>
  );
}
