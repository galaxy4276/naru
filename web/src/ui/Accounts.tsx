import { useEffect, useRef, useState } from 'react';
import { Plus } from 'lucide-react';
import type { AccountAct, AccountView, AgentKind, AgentView, Connection, LimitWindow } from '@/connection';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { AGENTS, type Prefs, usePrefs } from './prefs';

const POLL_MS = 60_000;
/** Older limits say when they were read, even for the account in use. */
const STALE_MS = 10 * 60_000;
const FIVE_HOURS = 300;
const WEEK = 7 * 24 * 60;

/** Agents whose CLIs report no limits; their notes say so once instead of on every account. */
const NO_LIMITS: AgentKind[] = ['grok'];

export interface Accounts {
  agents: AgentView[];
  busy: boolean;
  act: (act: AccountAct) => void;
  refresh: () => void;
}

/** Every agent's accounts, read once a minute and after each action. */
export function useAccounts(conn: Connection | null, online: boolean, notify: (text: string) => void): Accounts {
  const [state, setState] = useState<{ agents: AgentView[]; busy: boolean }>({ agents: [], busy: false });
  const notifyRef = useRef(notify);
  notifyRef.current = notify;
  useEffect(() => {
    if (!conn || !online) return;
    const off = conn.onMessage((message) => {
      if (message.t !== 'accounts') return;
      setState({ agents: message.agents, busy: false });
      const text = message.error ?? message.notice;
      if (text) notifyRef.current(text);
      if (message.opened) conn.send({ t: 'focus', pane: message.opened });
    });
    const ask = () => conn.send({ t: 'accounts' });
    ask();
    const timer = setInterval(ask, POLL_MS);
    return () => {
      off();
      clearInterval(timer);
    };
  }, [conn, online]);
  return {
    ...state,
    act: (act) => {
      setState((old) => ({ ...old, busy: true }));
      conn?.send({ t: 'account_act', act });
    },
    refresh: () => conn?.send({ t: 'accounts' }),
  };
}

export function agentName(kind: AgentKind): string {
  return AGENTS.find((agent) => agent.kind === kind)?.name ?? kind;
}

export function AgentMark({ kind, off, small }: { kind: AgentKind; off?: boolean; small?: boolean }) {
  return (
    <span
      className={cn(
        'inline-grid shrink-0 place-items-center rounded-[5px] font-mono font-bold tracking-tight text-primary-foreground',
        small ? 'size-3.5 rounded-[4px] text-[7.5px]' : 'size-[18px] text-[9px]',
        off ? 'bg-muted-foreground/40' : 'bg-primary',
      )}
    >
      {AGENTS.find((agent) => agent.kind === kind)?.mark}
    </span>
  );
}

/** The windows the user chose to see, 5-hour first. */
export function shownWindows(windows: LimitWindow[], prefs: Prefs): LimitWindow[] {
  return windows.filter((w) => (w.minutes === FIVE_HOURS ? prefs.fiveHour : w.minutes === WEEK ? prefs.week : prefs.fiveHour || prefs.week));
}

export function windowName(minutes: number, short = false): string {
  if (minutes === WEEK) return short ? '주' : '주간';
  if (minutes % (24 * 60) === 0) return `${minutes / (24 * 60)}일`;
  return minutes % 60 === 0 ? `${minutes / 60}시간` : `${minutes}분`;
}

/** A window whose reset time has passed no longer says how much is used. */
export const hasReset = (w: LimitWindow, now: number) => w.resets_at !== null && w.resets_at * 1000 <= now;

export function limitTone(percent: number): 'err' | 'turn' | null {
  return percent >= 90 ? 'err' : percent >= 70 ? 'turn' : null;
}

export const toneText = (tone: 'err' | 'turn' | null) => (tone === 'err' ? 'text-destructive' : tone === 'turn' ? 'text-turn' : '');

function resetLabel(resetsAt: number | null, now: number): string {
  if (resetsAt === null) return '';
  const left = resetsAt * 1000 - now;
  if (left <= 0) return '풀렸어요';
  const minutes = Math.round(left / 60_000);
  if (minutes < 60) return `${minutes}분 뒤 풀림`;
  if (minutes < 24 * 60) return `${Math.floor(minutes / 60)}시간 ${minutes % 60}분 뒤 풀림`;
  const at = new Date(resetsAt * 1000);
  return `${at.getMonth() + 1}월 ${at.getDate()}일 ${String(at.getHours()).padStart(2, '0')}:${String(at.getMinutes()).padStart(2, '0')} 풀림`;
}

function agoLabel(at: number, now: number): string {
  const minutes = Math.floor((now - at * 1000) / 60_000);
  if (minutes < 2) return '방금 읽음';
  if (minutes < 60) return `${minutes}분 전에 읽음`;
  if (minutes < 48 * 60) return `${Math.floor(minutes / 60)}시간 전에 읽음`;
  return `${Math.floor(minutes / (24 * 60))}일 전에 읽음`;
}

function Meter({ percent, width }: { percent: number; width: number }) {
  const tone = limitTone(percent);
  return (
    <span className="inline-block h-[5px] overflow-hidden rounded-full bg-muted align-middle" style={{ width }}>
      <i
        className={cn('block h-full rounded-full', tone === 'err' ? 'bg-destructive' : tone === 'turn' ? 'bg-turn' : 'bg-primary')}
        style={{ width: `${Math.min(100, percent)}%` }}
      />
    </span>
  );
}

function Usage({ account, now, compact }: { account: AccountView; now: number; compact?: boolean }) {
  const prefs = usePrefs();
  const usage = account.usage;
  if (!usage) return <div className="mt-1 text-[11.5px] text-muted-foreground">한도를 아직 읽지 못했어요</div>;
  const windows = shownWindows(usage.windows, prefs);
  return (
    <div className={cn('mt-1 flex flex-wrap gap-x-3.5 gap-y-0.5 text-[11.5px] text-muted-foreground', compact && 'flex-col')}>
      {windows.map((w) => (
        <span key={w.minutes} className="inline-flex items-center gap-1.5 whitespace-nowrap">
          {windowName(w.minutes)}
          {hasReset(w, now) ? (
            <span>풀렸어요</span>
          ) : (
            <>
              <Meter percent={w.percent} width={compact ? 40 : 54} />
              <b className={cn('font-semibold text-foreground', toneText(limitTone(w.percent)))}>{Math.round(w.percent)}%</b>
              {w.resets_at !== null && <span>· {resetLabel(w.resets_at, now)}</span>}
            </>
          )}
        </span>
      ))}
      {(!account.active || now - usage.at * 1000 > STALE_MS) && <span>{agoLabel(usage.at, now)}</span>}
    </div>
  );
}

/** One agent's accounts with switching; `manage` adds renaming, removing and signing in. */
export function AccountList({ view, accounts, now, manage }: { view: AgentView; accounts: Accounts; now: number; manage?: boolean }) {
  const [renaming, setRenaming] = useState<string | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const unsaved = view.accounts.some((account) => account.unsaved);
  const act = (act: AccountAct) => accounts.act(act);

  return (
    <div>
      {view.accounts.length === 0 && (
        <div className="mb-1.5 rounded-xl bg-muted/60 px-3 py-2.5">
          <b className="font-semibold">{view.installed ? '로그인한 계정이 없어요' : `${agentName(view.agent)}가 설치돼 있지 않아요`}</b>
          {view.installed && <div className="mt-0.5 text-[11.5px] text-muted-foreground">로그인하면 계정과 한도가 여기에 나타나요</div>}
        </div>
      )}
      {view.accounts.map((account) => (
        <div
          key={account.id}
          className={cn('mb-1.5 flex items-center gap-2 rounded-xl px-3 py-2.5', account.active ? 'bg-background ring-[1.5px] ring-primary' : 'bg-muted/60')}
        >
          <div className="min-w-0 flex-1">
            {renaming === account.id ? (
              <input
                autoFocus
                defaultValue={account.label}
                maxLength={40}
                aria-label="계정 이름"
                className="h-6 w-full rounded-md bg-background px-1.5 ring-1 ring-border outline-none"
                onKeyDown={(event) => {
                  if (event.key === 'Escape') return (event.stopPropagation(), setRenaming(null));
                  if (event.key !== 'Enter' || event.nativeEvent.isComposing) return;
                  act({ a: 'rename', agent: view.agent, id: account.id, name: event.currentTarget.value });
                  setRenaming(null);
                }}
                onBlur={() => setRenaming(null)}
              />
            ) : (
              <div className="flex min-w-0 items-baseline gap-1.5">
                <b className="truncate font-semibold">{account.label}</b>
                {account.detail && <small className="truncate text-[11.5px] text-muted-foreground">{account.detail}</small>}
              </div>
            )}
            {!NO_LIMITS.includes(view.agent) && <Usage account={account} now={now} compact={!manage} />}
          </div>
          {account.unsaved ? (
            <Button size="xs" disabled={accounts.busy} onClick={() => act({ a: 'save_current', agent: view.agent })}>
              계정으로 저장
            </Button>
          ) : account.active ? (
            <span className="rounded-md bg-primary px-1.5 py-px text-[11px] font-bold whitespace-nowrap text-primary-foreground">쓰는 중</span>
          ) : (
            view.switchable && (
              <Button
                size="xs"
                disabled={accounts.busy || unsaved}
                title={unsaved ? '지금 로그인을 먼저 계정으로 저장해 주세요' : undefined}
                onClick={() => act({ a: 'switch', agent: view.agent, id: account.id })}
              >
                바꾸기
              </Button>
            )
          )}
          {manage && !account.unsaved && (
            <Button variant="ghost" size="xs" onClick={() => setRenaming(account.id)}>
              이름
            </Button>
          )}
          {manage && !account.active && (
            <Button
              variant="ghost"
              size="xs"
              className={cn(removing === account.id && 'text-destructive')}
              disabled={accounts.busy}
              onBlur={() => setRemoving(null)}
              onClick={() => {
                if (removing !== account.id) return setRemoving(account.id);
                setRemoving(null);
                act({ a: 'remove', agent: view.agent, id: account.id });
              }}
            >
              {removing === account.id ? '정말 빼기' : '빼기'}
            </Button>
          )}
        </div>
      ))}
      {view.error && <p className="mb-1.5 text-[12px] whitespace-pre-wrap text-destructive">{view.error}</p>}
      {manage && view.add && view.installed && (
        <Button variant="ghost" size="xs" disabled={accounts.busy} onClick={() => act({ a: 'add', agent: view.agent })}>
          <Plus />
          {view.add}
        </Button>
      )}
    </div>
  );
}

/** What switching changes for agents already running. */
export function switchNote(view: AgentView): string | null {
  const reopen = (name: string) => `바꾸면 새로 여는 ${name}부터 새 계정을 써요. 이미 실행 중인 ${name} 세션은 다시 열어야 바뀌어요.`;
  const switching: Record<AgentKind, string> = {
    claude: '바꾸면 실행 중인 Claude도 다음 요청부터 새 계정을 써요.',
    codex: reopen('Codex'),
    agy: reopen('agy'),
    grok: reopen('Grok'),
  };
  return [switching[view.agent], view.note].filter(Boolean).join(' ') || null;
}
