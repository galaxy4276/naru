import { useEffect, useState } from 'react';
import { Image as ImageIcon, Waypoints } from 'lucide-react';
import type { Connection, Mark, Proposal, Stage, Workspace } from '@/connection';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { cn } from '@/lib/utils';
import { timeLabel } from './model';
import { SidePanel } from './SidePanel';

export const stageLabel: Record<Stage, string> ={ new: '새로', studying: '공부 중', understood: '이해함' };
const whyLabel: Record<Proposal['why'], string> = { question: '물어봤어요', confused: '헷갈려 했어요', corrected: '잘못 알고 있었어요', unknown: '모른다고 했어요' };

/** A mark to show opened, and whether to ask for its hard part (⌘⇧L just made it). */
export interface LearnFocus {
  id: number;
  retitle: boolean;
}

interface Props {
  conn: Connection;
  ws: Workspace;
  marks: Mark[];
  /** What Memory reads found the user did not know. */
  proposals: Proposal[];
  memoryOn: boolean;
  error: string | null;
  focus: LearnFocus | null;
  onOpenAi: () => void;
  onOpenMap: () => void;
  onClose: () => void;
}

/** 학습함: what the user found hard while working, to study in a session of its own. */
export function LearnPanel({ conn, ws, marks, proposals, memoryOn, error, focus, onOpenAi, onOpenMap, onClose }: Props) {
  const [filter, setFilter] = useState<Stage | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const [editing, setEditing] = useState<LearnFocus | null>(null);
  const [failure, setFailure] = useState<{ id: number; message: string } | null>(null);

  useEffect(() => conn.send({ t: 'read_learn' }), [conn]);
  useEffect(() => {
    if (!focus) return;
    setFilter(null);
    setSelected(focus.id);
    setEditing(focus);
  }, [focus]);
  useEffect(
    () =>
      conn.onMessage((message) => {
        if (message.t === 'learn_failure' && message.id !== null) setFailure({ id: message.id, message: message.message });
      }),
    [conn],
  );

  const count = (stage: Stage) => marks.filter((mark) => mark.stage === stage).length;
  const shown = marks.filter((mark) => !filter || mark.stage === filter).reverse();
  const filters: [Stage | null, string][] = [
    [null, '전체'],
    ['new', `새로 ${count('new')}`],
    ['studying', `공부 중 ${count('studying')}`],
    ['understood', `이해함 ${count('understood')}`],
  ];

  return (
    <SidePanel
      title="학습함"
      sub={`${marks.length}개`}
      onClose={onClose}
      actions={
        <Button variant="ghost" size="xs" onClick={onOpenMap}>
          <Waypoints />
          지도
        </Button>
      }
    >
      <div className="mx-3 mb-2 grid grid-cols-4 gap-1 rounded-[10px] bg-muted p-1 text-[12px]">
        {filters.map(([stage, label]) => (
          <button
            key={label}
            onClick={() => setFilter(stage)}
            className={cn('h-7 rounded-lg text-muted-foreground', filter === stage && 'bg-background font-semibold text-foreground')}
          >
            {label}
          </button>
        ))}
      </div>
      {error && <p className="px-4 pb-2 text-destructive">{error}</p>}
      <Proposals conn={conn} proposals={proposals} memoryOn={memoryOn} onOpenAi={onOpenAi} />
      {marks.length === 0 ? (
        <div className="px-4 py-6 text-center">
          <b className="font-semibold">아직 표시한 곳이 없어요</b>
          <p className="mt-1 text-muted-foreground">
            대화에서 어려운 부분을 고르고 ⌘⇧L을 누르세요. 에이전트에게 "학습에 남겨 줘"라고 말해도 돼요.
          </p>
        </div>
      ) : shown.length === 0 ? (
        <p className="px-4 text-muted-foreground">이 상태의 표시가 없어요.</p>
      ) : null}
      <ul className="flex flex-col gap-0.5 px-2 pb-3">
        {shown.map((mark) => {
          const open = selected === mark.id;
          const meta = [mark.from_title, mark.project, timeLabel(mark.at)].filter(Boolean).join(' · ');
          return (
            <li key={mark.id}>
              <button
                onClick={() => setSelected(open ? null : mark.id)}
                className={cn('flex w-full flex-col gap-0.5 rounded-xl px-2.5 py-2 text-left hover:bg-muted', open && 'bg-muted')}
              >
                <span className="flex items-center gap-2">
                  <b className="min-w-0 flex-1 truncate font-semibold">{mark.title}</b>
                  <span className={cn('shrink-0 text-[11.5px]', mark.stage === 'new' ? 'font-semibold text-turn' : 'text-muted-foreground')}>
                    {stageLabel[mark.stage]}
                  </span>
                </span>
                <span className="truncate text-[11.5px] text-muted-foreground">{meta}</span>
              </button>
              {open && (
                <Detail
                  conn={conn}
                  ws={ws}
                  mark={mark}
                  editing={editing?.id === mark.id ? editing : null}
                  failure={failure?.id === mark.id ? failure.message : null}
                  onEdit={(retitle) => setEditing({ id: mark.id, retitle })}
                  onEdited={() => setEditing(null)}
                  onAct={() => setFailure(null)}
                />
              )}
            </li>
          );
        })}
      </ul>
    </SidePanel>
  );
}

/** 담을까요: concepts a Memory read found in the user's own words, newest first. */
function Proposals({ conn, proposals, memoryOn, onOpenAi }: { conn: Connection; proposals: Proposal[]; memoryOn: boolean; onOpenAi: () => void }) {
  if (!memoryOn)
    return (
      <div className="mx-3 mb-2 flex flex-col items-start gap-1.5 rounded-xl bg-muted px-3 py-2.5 text-[12px] text-muted-foreground">
        Memory가 꺼져 있어 대화에서 담을 거리를 찾지 않아요
        <Button size="xs" variant="secondary" onClick={onOpenAi}>
          Naru AI에서 켜기
        </Button>
      </div>
    );
  if (proposals.length === 0) return null;
  return (
    <section className="mx-3 mb-2 flex flex-col gap-1">
      <h3 className="px-1 text-[12px] font-semibold">담을까요? {proposals.length}</h3>
      {[...proposals].reverse().map((proposal) => (
        <div key={proposal.id} className="flex flex-col gap-1.5 rounded-xl border border-dashed border-foreground/30 px-3 py-2">
          <b className="font-semibold">{proposal.title}</b>
          <q className="line-clamp-3 text-[12.5px] [overflow-wrap:anywhere]">{proposal.quote}</q>
          <span className="truncate text-[11.5px] text-muted-foreground">
            {[proposal.project, whyLabel[proposal.why], timeLabel(proposal.at)].filter(Boolean).join(' · ')}
          </span>
          <div className="flex gap-1.5">
            <ProposalActions conn={conn} id={proposal.id} />
          </div>
        </div>
      ))}
    </section>
  );
}

/** 담기 and 넘기기 for proposal `id`, wherever it is shown. */
export function ProposalActions({ conn, id }: { conn: Connection; id: number }) {
  return (
    <>
      <Button size="xs" onClick={() => conn.send({ t: 'learn_take', proposal: id })}>
        담기
      </Button>
      <Button size="xs" variant="ghost" onClick={() => conn.send({ t: 'learn_skip', proposal: id })}>
        넘기기
      </Button>
    </>
  );
}

interface DetailProps {
  conn: Connection;
  ws: Workspace;
  mark: Mark;
  editing: LearnFocus | null;
  failure: string | null;
  onEdit: (retitle: boolean) => void;
  onEdited: () => void;
  onAct: () => void;
}

function Detail({ conn, ws, mark, editing, failure, onEdit, onEdited, onAct }: DetailProps) {
  const lines = mark.excerpt.split('\n');
  const excerpt = lines.slice(0, 12).join('\n') + (lines.length > 12 ? '\n…' : '');
  const original = mark.source
    ? Object.entries(ws.panes).find(([, record]) => record.session?.session_id === mark.source?.session_id)?.[0]
    : undefined;
  const act = (message: Parameters<Connection['send']>[0]) => {
    onAct();
    conn.send(message);
  };
  return (
    <div className="flex flex-col gap-2 px-2.5 pt-1.5 pb-3">
      <pre className="m-0 rounded-lg border-l-[3px] border-primary bg-muted px-3 py-2 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap [overflow-wrap:anywhere]">
        {excerpt}
      </pre>
      {editing ? (
        <HardInput
          initial={mark.hard}
          onDone={(hard) => {
            act({ t: 'learn_hard', id: mark.id, hard, retitle: editing.retitle });
            onEdited();
          }}
        />
      ) : mark.hard && mark.hard === mark.title ? null : (
        <button onClick={() => onEdit(false)} className={cn('text-left hover:underline', !mark.hard && 'text-muted-foreground')}>
          {mark.hard ? `어려운 부분 · ${mark.hard}` : '어려운 부분 적기'}
        </button>
      )}
      {mark.origin && <p className="text-[11.5px] text-muted-foreground">{mark.origin}</p>}
      {mark.materials.length > 0 && (
        <div className="flex flex-wrap items-center gap-1">
          <span className="text-[11.5px] text-muted-foreground">자료 {mark.materials.length}</span>
          {mark.materials.map((name) => (
            <button
              key={name}
              onClick={() => act({ t: 'learn_open_material', id: mark.id, name })}
              className="flex h-6 items-center gap-1 rounded-md bg-muted px-2 text-[11.5px] hover:bg-accent"
              title={name}
            >
              <ImageIcon className="size-3" />
              <span className="max-w-40 truncate">{name}</span>
            </button>
          ))}
        </div>
      )}
      <div className="flex flex-wrap gap-1.5">
        <Button size="sm" onClick={() => act({ t: 'learn_study', id: mark.id })}>
          학습 세션 열기
        </Button>
        {mark.stage === 'understood' ? (
          <>
            {mark.note_exists ? (
              <Button size="sm" variant="secondary" onClick={() => act({ t: 'learn_open_note', id: mark.id })}>
                노트 열기
              </Button>
            ) : (
              <Button size="sm" variant="secondary" onClick={() => act({ t: 'learn_note', id: mark.id })}>
                노트로 남기기
              </Button>
            )}
            <Button size="sm" variant="secondary" onClick={() => act({ t: 'learn_stage', id: mark.id, stage: 'studying' })}>
              다시 공부
            </Button>
          </>
        ) : (
          <>
            <Button size="sm" variant="secondary" disabled={!original} onClick={() => original && act({ t: 'focus', pane: original })}>
              원래 대화로
            </Button>
            <Button size="sm" variant="secondary" onClick={() => act({ t: 'learn_stage', id: mark.id, stage: 'understood' })}>
              이해함
            </Button>
          </>
        )}
      </div>
      {failure && <p className="text-[11.5px] text-destructive">{failure}</p>}
    </div>
  );
}

/** One line of what was hard. Enter, Esc and leaving the field all keep what was typed. */
function HardInput({ initial, onDone }: { initial: string; onDone: (hard: string) => void }) {
  const [draft, setDraft] = useState(initial);
  const [done, setDone] = useState(false);
  const finish = () => {
    if (done) return;
    setDone(true);
    onDone(draft);
  };
  return (
    <label className="flex flex-col gap-1">
      <span className="text-[11.5px] text-muted-foreground">어려운 부분 한 줄 · Enter로 저장</span>
      <Input
        autoFocus
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={finish}
        onKeyDown={(event) => {
          if ((event.key === 'Enter' || event.key === 'Escape') && !event.nativeEvent.isComposing) finish();
        }}
        className="h-8 rounded-lg"
      />
    </label>
  );
}
