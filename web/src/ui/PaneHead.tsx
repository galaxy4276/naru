import { useEffect, useState } from 'react';
import { Check, Columns2, Pencil, Rows2, X } from 'lucide-react';
import type { ClientMsg, Connection, PaneId, Workspace } from '@/connection';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { cn } from '@/lib/utils';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { type Teams, basename, paneTitle, toneOf } from './model';
import { StatusChip } from './status';
import { FindBar } from './FindBar';
import { PlanSheet, planChip } from './PlanSheet';

interface Props {
  conn: Connection;
  ws: Workspace;
  teams: Teams;
  pane: PaneId;
  send: (message: ClientMsg) => void;
  onClose: () => void;
  /** ⌘F is open in this pane. */
  finding: boolean;
  onFindEnd: () => void;
  /** The pane the keyboard goes to; its head line turns into a dark band. */
  focused: boolean;
}

/** The thin line over a pane: title, what it is about, its state. Tools show on hover. */
export function PaneHead({ conn, ws, teams, pane, send, onClose, finding, onFindEnd, focused }: Props) {
  const [editing, setEditing] = useState(false);
  const [planOpen, setPlanOpen] = useState(false);
  const plan = teams[pane]?.plan ?? null;
  const [ideas, setIdeas] = useState<{ titles: string[]; error: string | null } | null>(null);
  const record = ws.panes[pane];
  const session = record?.session ?? null;
  const tone = toneOf(session, teams[pane]);
  const summary = record?.summary ?? session?.query ?? (record?.node ? `${record.node.host}:${record.node.cwd ?? '~'}` : record?.cwd);
  // The sheet asks haiku for title ideas each time it opens.
  useEffect(() => {
    if (!editing) return;
    setIdeas(null);
    const off = conn.onMessage((message) => {
      if (message.t === 'title_candidates' && message.pane === pane) setIdeas({ titles: message.titles, error: message.error });
    });
    conn.send({ t: 'title_candidates', pane });
    return off;
  }, [conn, editing, pane]);

  if (finding) return <FindBar pane={pane} onClose={onFindEnd} />;
  return (
    <div
      className={cn(
        'group/head flex h-10 shrink-0 items-center gap-2 px-3.5 transition-colors duration-300',
        focused && 'bg-primary text-primary-foreground [&_.text-muted-foreground]:text-primary-foreground/60',
      )}
    >
      {tone === 'done' && (
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              aria-label="대기로 돌리기"
              onClick={() => send({ t: 'mark_seen', pane })}
              className="group/check grid size-[18px] shrink-0 place-items-center rounded-full border-2 border-green-500 text-white hover:bg-green-500"
            >
              <Check className="size-3 opacity-0 group-hover/check:opacity-100" strokeWidth={3.5} />
            </button>
          </TooltipTrigger>
          <TooltipContent>대기로 돌리기</TooltipContent>
        </Tooltip>
      )}
      <button onClick={() => setEditing(true)} className="truncate text-[14px] font-bold hover:underline" title="제목 고치기">
        {paneTitle(ws, pane)}
      </button>
      <span className="min-w-0 flex-1 truncate text-muted-foreground">{summary ?? basename(record?.cwd)}</span>
      {plan && (
        <button
          onClick={() => setPlanOpen(true)}
          className={cn(
            'shrink-0 rounded-full px-2 py-0.5 text-[11.5px] font-semibold',
            plan.review ? 'bg-turn text-white' : 'bg-muted text-muted-foreground hover:text-foreground',
          )}
        >
          {planChip(plan)}
        </button>
      )}
      {plan && <PlanSheet pane={pane} plan={plan} open={planOpen} send={send} onClose={() => setPlanOpen(false)} />}
      {tone && <StatusChip tone={tone} />}
      <span className={cn('hidden items-center gap-0.5 text-muted-foreground group-hover/head:flex', focused && '[&_button:hover]:bg-primary-foreground/15 [&_button:hover]:text-primary-foreground')}>
        <Tool label="제목 고치기" onClick={() => setEditing(true)} icon={<Pencil />} />
        <Tool label="오른쪽으로 나누기" onClick={() => send({ t: 'split', pane, direction: 'right' })} icon={<Columns2 />} />
        <Tool label="아래로 나누기" onClick={() => send({ t: 'split', pane, direction: 'down' })} icon={<Rows2 />} />
        <Tool label="pane 닫기" onClick={onClose} icon={<X />} />
      </span>
      <TitleSheet
        open={editing}
        title={record?.title ?? ''}
        placeholder={paneTitle(ws, pane)}
        candidates={ideas?.titles}
        loading={editing && !ideas}
        error={ideas?.error ?? null}
        onClose={() => setEditing(false)}
        onSave={(title) => {
          send({ t: 'set_title', pane, title: title.trim() || null });
          setEditing(false);
        }}
      />
    </div>
  );
}

function Tool({ label, onClick, icon }: { label: string; onClick: () => void; icon: React.ReactNode }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button variant="ghost" size="icon-xs" onClick={onClick} aria-label={label}>
          {icon}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

interface SheetProps {
  open: boolean;
  title: string;
  placeholder: string;
  /** AI title suggestions; the sheet shows them once there are any. */
  candidates?: string[];
  loading?: boolean;
  error?: string | null;
  onClose: () => void;
  onSave: (title: string) => void;
}

export function TitleSheet({ open, title, placeholder, candidates = [], loading, error, onClose, onSave }: SheetProps) {
  const [draft, setDraft] = useState(title);
  return (
    <Dialog open={open} onOpenChange={(next) => (next ? setDraft(title) : onClose())}>
      <DialogContent className="top-24 translate-y-0 rounded-2xl border-0 sm:max-w-[520px]" showCloseButton={false}>
        <DialogHeader>
          <DialogTitle className="text-[15px]">pane 제목</DialogTitle>
          <DialogDescription>비워 두면 자동 제목을 씁니다. 직접 쓴 제목은 자동 제목이 덮지 않습니다.</DialogDescription>
        </DialogHeader>
        <Input
          autoFocus
          value={draft}
          placeholder={placeholder}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.nativeEvent.isComposing) onSave(draft);
          }}
          className="h-10 rounded-[10px] text-[15px] font-semibold"
        />
        {loading && <p className="text-[12px] text-muted-foreground">AI 제목 후보 만드는 중…</p>}
        {error && <p className="text-[12px] text-muted-foreground">{error}</p>}
        {candidates.length > 0 && (
          <div className="flex flex-wrap gap-1.5">
            {candidates.map((candidate) => (
              <Button key={candidate} variant="secondary" size="sm" onClick={() => setDraft(candidate)}>
                {candidate}
              </Button>
            ))}
          </div>
        )}
        <DialogFooter>
          <Button variant="secondary" onClick={onClose}>
            취소
          </Button>
          <Button onClick={() => onSave(draft)}>저장</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
