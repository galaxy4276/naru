import { useEffect, useState } from 'react';
import { Bell, FolderOpen, Keyboard, Megaphone, Minus, PanelBottom, Plus, Type, UserRound } from 'lucide-react';
import { Switch } from 'radix-ui';
import type { AgentKind, Connection, NaruPaths } from '@/connection';
import { Input } from '@/components/ui/input';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog';
import { cn } from '@/lib/utils';
import { type Accounts, AccountList, AgentMark, switchNote } from './Accounts';
import { AGENTS, NOTIFY_EVENTS, TERM_SIZE, TERMINAL_FONTS, UI_SIZE, setPrefs, usePrefs } from './prefs';
import { SHORTCUT_GROUPS } from './shortcuts';
import { askNotifyPermission } from './notify';

export type SettingsPage = 'screen' | 'accounts' | 'bar' | 'notify' | 'keys' | 'paths' | 'meta';

const PAGES: { key: SettingsPage; name: string; icon: typeof Type }[] = [
  { key: 'screen', name: '화면', icon: Type },
  { key: 'accounts', name: '에이전트 계정', icon: UserRound },
  { key: 'bar', name: '하단 바', icon: PanelBottom },
  { key: 'notify', name: '알림', icon: Bell },
  { key: 'keys', name: '단축키', icon: Keyboard },
  { key: 'paths', name: '경로', icon: FolderOpen },
  { key: 'meta', name: 'Meta 광고', icon: Megaphone },
];

export interface SettingsAt {
  page: SettingsPage;
  agent: AgentKind;
}

export function SettingsDialog({ conn, at, accounts, now, onAt, onTheme, onClose }: { conn: Connection; at: SettingsAt; accounts: Accounts; now: number; onAt: (at: SettingsAt) => void; onTheme: () => void; onClose: () => void }) {
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="flex h-[min(600px,85vh)] gap-0 overflow-hidden rounded-2xl border-0 p-0 sm:max-w-[820px]">
        <nav className="flex w-[188px] shrink-0 flex-col gap-0.5 bg-canvas p-3">
          <DialogTitle className="px-2 pt-1 pb-3 text-[15px]">설정</DialogTitle>
          {PAGES.map(({ key, name, icon: Icon }) => (
            <button
              key={key}
              onClick={() => onAt({ ...at, page: key })}
              className={cn('flex h-8 items-center gap-2 rounded-lg px-2 text-left', at.page === key ? 'bg-background font-semibold' : 'text-muted-foreground hover:text-foreground')}
            >
              <Icon className="size-[15px]" />
              {name}
            </button>
          ))}
        </nav>
        <div className="min-w-0 flex-1 overflow-y-auto px-7 py-6">
          {at.page === 'screen' && <ScreenPage onTheme={onTheme} />}
          {at.page === 'accounts' && <AccountsPage agent={at.agent} accounts={accounts} now={now} onAgent={(agent) => onAt({ ...at, agent })} />}
          {at.page === 'bar' && <BarPage accounts={accounts} />}
          {at.page === 'notify' && <NotifyPage />}
          {at.page === 'keys' && <KeysPage />}
          {at.page === 'paths' && <PathsPage conn={conn} />}
          {at.page === 'meta' && <MetaAdsPage conn={conn} />}
        </div>
      </DialogContent>
    </Dialog>
  );
}

function Head({ title, lead }: { title: string; lead: string }) {
  return (
    <>
      <h2 className="text-[17px] font-bold">{title}</h2>
      <DialogDescription className="mt-0.5 mb-3 text-[12.5px]">{lead}</DialogDescription>
    </>
  );
}

function Row({ title, sub, children }: { title: React.ReactNode; sub?: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-wrap items-center gap-x-3.5 gap-y-2 border-b border-border/50 py-3 last:border-b-0">
      <div className="min-w-[150px] flex-1">
        <b className="flex items-center gap-1.5 font-semibold whitespace-nowrap">{title}</b>
        {sub && <span className="block text-[12px] text-muted-foreground">{sub}</span>}
      </div>
      {children}
    </div>
  );
}

function Chip({ on, onClick, children }: { on?: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      aria-pressed={on}
      className={cn('inline-flex h-7 items-center gap-1.5 rounded-lg px-2.5 text-[12.5px]', on ? 'bg-primary text-primary-foreground' : 'bg-muted text-foreground hover:bg-accent')}
    >
      {children}
    </button>
  );
}

function Size({ value, range, onChange }: { value: number; range: { min: number; max: number }; onChange: (value: number) => void }) {
  return (
    <div className="flex items-center gap-2">
      <Button variant="ghost" size="icon-xs" className="bg-muted" onClick={() => onChange(value - 1)} disabled={value <= range.min} aria-label="작게">
        <Minus />
      </Button>
      <input type="range" min={range.min} max={range.max} value={value} onChange={(event) => onChange(Number(event.target.value))} className="w-[120px]" />
      <Button variant="ghost" size="icon-xs" className="bg-muted" onClick={() => onChange(value + 1)} disabled={value >= range.max} aria-label="크게">
        <Plus />
      </Button>
      <b className="w-6 text-right tabular-nums">{value}</b>
    </div>
  );
}

function ScreenPage({ onTheme }: { onTheme: () => void }) {
  const prefs = usePrefs();
  return (
    <>
      <Head title="화면" lead="바꾸면 바로 적용돼요." />
      <Row title="화면 글자 크기" sub={`사이드바 · 패널. 기본 ${UI_SIZE.base}`}>
        <Size value={prefs.ui} range={UI_SIZE} onChange={(ui) => setPrefs({ ui })} />
      </Row>
      <Row title="터미널 글자 크기" sub={`모든 pane. 기본 ${TERM_SIZE.base}`}>
        <Size value={prefs.term} range={TERM_SIZE} onChange={(term) => setPrefs({ term })} />
      </Row>
      <Row title="터미널 글꼴" sub="없는 글꼴이면 다음 글꼴로 보여요">
        <div className="flex gap-1.5">
          {TERMINAL_FONTS.map((font) => (
            <Chip key={font} on={prefs.font === font} onClick={() => setPrefs({ font })}>
              {font}
            </Chip>
          ))}
        </div>
      </Row>
      <Row title="테마" sub="흰 카드 · 검은 사이드바 · 다크 · 시스템">
        <Button variant="outline" size="sm" onClick={onTheme}>
          테마 고르기
        </Button>
      </Row>
    </>
  );
}

function AccountsPage({ agent, accounts, now, onAgent }: { agent: AgentKind; accounts: Accounts; now: number; onAgent: (agent: AgentKind) => void }) {
  const view = accounts.agents.find((item) => item.agent === agent);
  const note = view && switchNote(view);
  return (
    <>
      <Head title="에이전트 계정" lead="에이전트마다 계정을 여러 개 저장하고, 하단 바나 여기서 바로 바꿔요." />
      <div className="mb-2.5 flex flex-wrap gap-1.5">
        {AGENTS.map(({ kind, name }) => {
          const count = accounts.agents.find((item) => item.agent === kind)?.accounts.length ?? 0;
          return (
            <Chip key={kind} on={kind === agent} onClick={() => onAgent(kind)}>
              {name}
              {count > 0 && <span className="opacity-60">{count}</span>}
            </Chip>
          );
        })}
      </div>
      {note && <p className="mb-2.5 text-[12px] text-muted-foreground">{note}</p>}
      {view ? <AccountList view={view} accounts={accounts} now={now} manage /> : <p className="text-muted-foreground">계정을 읽는 중이에요</p>}
    </>
  );
}

function BarPage({ accounts }: { accounts: Accounts }) {
  const prefs = usePrefs();
  return (
    <>
      <Head title="하단 바" lead="바에 보일 에이전트와 한도를 골라요. 설치되지 않은 에이전트는 켜 둬도 보이지 않아요." />
      {AGENTS.map(({ kind, name }) => {
        const view = accounts.agents.find((item) => item.agent === kind);
        const active = view?.accounts.find((account) => account.active);
        return (
          <Row
            key={kind}
            title={
              <>
                <AgentMark kind={kind} off={!active} />
                {name}
              </>
            }
            sub={!view ? '' : !view.installed ? '설치 안 됨' : (active?.label ?? '로그인 필요')}
          >
            <Toggle on={prefs.bar[kind]} label={`${name} 보이기`} onChange={(on) => setPrefs({ bar: { ...prefs.bar, [kind]: on } })} />
          </Row>
        );
      })}
      <Row title="5시간 한도" sub="Claude · Codex · agy">
        <Toggle on={prefs.fiveHour} label="5시간 한도 보이기" onChange={(fiveHour) => setPrefs({ fiveHour })} />
      </Row>
      <Row title="주간 한도" sub="70%부터 주황, 90%부터 빨강">
        <Toggle on={prefs.week} label="주간 한도 보이기" onChange={(week) => setPrefs({ week })} />
      </Row>
    </>
  );
}

export function Toggle({ on, label, onChange }: { on: boolean; label: string; onChange: (on: boolean) => void }) {
  return (
    <Switch.Root
      checked={on}
      onCheckedChange={onChange}
      aria-label={label}
      className="relative h-5 w-9 shrink-0 rounded-full bg-muted-foreground/30 transition-colors data-[state=checked]:bg-primary"
    >
      <Switch.Thumb className="block size-4 translate-x-0.5 rounded-full bg-background shadow-sm transition-transform data-[state=checked]:translate-x-[18px]" />
    </Switch.Root>
  );
}

function KeysPage() {
  return (
    <>
      <Head title="단축키" lead="지금 앱에 있는 단축키예요. 어디서나 ⌘/ 로도 볼 수 있어요." />
      {SHORTCUT_GROUPS.flatMap(([group, rows]) => [
        <b key={group} className="mt-4 block text-[12px] font-semibold text-muted-foreground first:mt-0">
          {group}
        </b>,
        ...rows.map(([what, keys]) => (
          <Row key={`${group}:${what}`} title={what}>
            <kbd className="rounded-md bg-muted px-2 py-0.5 font-mono text-[12px]">{keys}</kbd>
          </Row>
        )),
      ])}
    </>
  );
}

function NotifyPage() {
  const prefs = usePrefs();
  const on = (change: Partial<typeof prefs>) => {
    askNotifyPermission();
    setPrefs(change);
  };
  return (
    <>
      <Head title="알림" lead="에이전트가 나를 부를 때 macOS 알림으로 알려요." />
      <div className="flex gap-6">
        <div className="min-w-0 flex-[1.3]">
          {NOTIFY_EVENTS.map(({ key, name, sub }) => (
            <Row key={key} title={name} sub={sub}>
              <Toggle on={prefs.notify[key]} label={name} onChange={(value) => on({ notify: { ...prefs.notify, [key]: value } })} />
            </Row>
          ))}
          <div className="my-2 border-t" />
          <Row title="Naru가 앞에 있으면 조용히" sub="이 앱을 보고 있을 땐 울리지 않아요">
            <Toggle on={prefs.quietInFront} label="Naru가 앞에 있으면 조용히" onChange={(quietInFront) => on({ quietInFront })} />
          </Row>
          <Row title="소리">
            <Toggle on={prefs.notifySound} label="소리" onChange={(notifySound) => on({ notifySound })} />
          </Row>
        </div>
        <div className="flex flex-1 flex-col gap-2 pt-3">
          <b className="text-[12px] font-semibold text-muted-foreground">이렇게 떠요</b>
          <div className="flex items-start gap-2.5 rounded-xl bg-muted p-3">
            <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-primary text-[13px] font-bold text-primary-foreground">N</span>
            <div className="min-w-0 flex-1">
              <b className="block truncate text-[12.5px]">solo-nestjs-api · 매칭 큐 지연 원인 찾기</b>
              <span className="block text-[12px] text-muted-foreground">concurrency를 8로 올릴까요?</span>
            </div>
            <span className="text-[11px] text-muted-foreground">지금</span>
          </div>
          <span className="text-[11.5px] text-muted-foreground">예시 알림이에요. 실제 알림에는 그 세션의 프로젝트·제목과 마지막 말이 들어가요.</span>
        </div>
      </div>
    </>
  );
}

/** The Meta system user token the Meta 광고 panel reads with; only its last four characters come back. */
function MetaAdsPage({ conn }: { conn: Connection }) {
  const [tail, setTail] = useState<string | null | undefined>(undefined);
  const [token, setToken] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const ask = () => conn.send({ t: 'meta_ads', account: null, level: 'campaign', parent: null, preset: 'today' });
    const off = conn.onMessage((message) => {
      if (message.t === 'meta_ads' && message.level === 'campaign') setTail(message.token_tail);
      if (message.t !== 'meta_ads_done' || message.id !== null) return;
      setBusy(false);
      setError(message.error);
      if (!message.error) {
        setToken('');
        ask();
      }
    });
    ask();
    return off;
  }, [conn]);

  const save = (value: string | null) => {
    setBusy(true);
    setError(null);
    conn.send({ t: 'meta_ads_token', token: value });
  };
  return (
    <>
      <Head title="Meta 광고" lead="비즈니스 관리자의 시스템 사용자 토큰을 넣어요. 읽기는 ads_read, 켜기·끄기와 예산 수정은 ads_management 권한이 필요해요." />
      <p className="mb-2 text-[12.5px]">
        {tail === undefined ? '확인하는 중이에요' : tail === null ? '저장된 토큰이 없어요' : <>저장된 토큰 <b>…{tail}</b></>}
      </p>
      <form
        className="flex gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          if (token.trim()) save(token);
        }}
      >
        <Input type="password" autoComplete="off" value={token} onChange={(event) => setToken(event.target.value)} placeholder="EAA…" disabled={busy} />
        <Button type="submit" disabled={busy || !token.trim()}>
          {busy ? '확인 중' : '저장'}
        </Button>
        {tail && (
          <Button type="button" variant="secondary" disabled={busy} onClick={() => save(null)}>
            지우기
          </Button>
        )}
      </form>
      <p className="mt-2 text-[12px] text-muted-foreground">저장할 때 Meta에 토큰이 맞는지 한 번 물어봐요. 토큰은 이 맥의 ~/.naru/meta-ads.json에만 있어요.</p>
      {error && <p className="mt-2 text-[12.5px] text-red-600 dark:text-red-400">{error}</p>}
    </>
  );
}

const PATH_FIELDS: { key: keyof NaruPaths; name: string; hint: string; placeholder: string }[] = [
  { key: 'brain_note', name: 'Brain 노트', hint: 'Brain 창이 고치는 노트 파일 하나', placeholder: '~/Obsidian/notes/inbox/Brain.md' },
  { key: 'library_notes', name: '서재 노트 폴더', hint: '책마다 노트를 이 폴더의 learning/서재 아래에 둬요', placeholder: '~/Obsidian/notes' },
  { key: 'study_root', name: '학습 노트 폴더', hint: '학습함이 날짜별 폴더로 학습 노트를 써요. 학습 지도도 여기서 읽어요', placeholder: '~/Obsidian/study' },
];

/** Folders outside Naru it reads and writes; a feature whose folder is unset says so instead of guessing. */
function PathsPage({ conn }: { conn: Connection }) {
  const [saved, setSaved] = useState<NaruPaths | null>(null);
  const [draft, setDraft] = useState<NaruPaths>({ brain_note: '', library_notes: '', study_root: '' });
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);

  useEffect(() => {
    const off = conn.onMessage((message) => {
      if (message.t !== 'paths') return;
      setError(message.error);
      if (message.paths) {
        setSaved(message.paths);
        setDraft({ brain_note: message.paths.brain_note ?? '', library_notes: message.paths.library_notes ?? '', study_root: message.paths.study_root ?? '' });
      }
    });
    conn.send({ t: 'paths' });
    return off;
  }, [conn]);

  const changed = saved !== null && PATH_FIELDS.some(({ key }) => (draft[key] ?? '') !== (saved[key] ?? ''));
  return (
    <>
      <Head title="경로" lead="Naru가 읽고 쓰는 바깥 폴더예요. 비워 두면 그 기능은 경로를 정해 달라고 알려요." />
      <form
        className="flex flex-col gap-3"
        onSubmit={(event) => {
          event.preventDefault();
          setDone(false);
          conn.send({ t: 'paths_set', paths: draft });
          setDone(true);
        }}
      >
        {PATH_FIELDS.map(({ key, name, hint, placeholder }) => (
          <label key={key} className="flex flex-col gap-1">
            <span className="text-[12.5px] font-semibold">{name}</span>
            <Input value={draft[key] ?? ''} placeholder={placeholder} onChange={(event) => (setDone(false), setDraft({ ...draft, [key]: event.target.value }))} spellCheck={false} />
            <span className="text-[12px] text-muted-foreground">{hint}</span>
          </label>
        ))}
        <div className="flex items-center gap-2">
          <Button type="submit" disabled={!changed}>
            저장
          </Button>
          {done && !error && !changed && <span className="text-[12px] text-muted-foreground">저장했어요</span>}
        </div>
      </form>
      {error && <p className="mt-2 text-[12.5px] text-red-600 dark:text-red-400">{error}</p>}
    </>
  );
}
