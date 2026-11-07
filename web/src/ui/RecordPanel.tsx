import { useEffect, useRef, useState } from 'react';
import type { Connection, PaneId, Turn } from '@/connection';
import { cn } from '@/lib/utils';
import { SidePanel } from './SidePanel';

const POLL_MS = 3000;

interface Props {
  conn: Connection;
  pane: PaneId | null;
  title: string;
  onClose: () => void;
}

/** The focused session's requests, newest first; a row opens the whole request and the reply. */
export function RecordPanel({ conn, pane, title, onClose }: Props) {
  const [turns, setTurns] = useState<Turn[] | null>(null);
  const [reason, setReason] = useState<string | null>(null);
  const [cut, setCut] = useState(false);
  const [open, setOpen] = useState<number | null>(null);
  const mtime = useRef<number | null>(null);

  useEffect(() => {
    setTurns(null);
    setReason(null);
    setOpen(null);
    mtime.current = null;
    if (!pane) return;
    const off = conn.onMessage((message) => {
      if (message.t !== 'record' || message.pane !== pane) return;
      mtime.current = message.mtime;
      setReason(message.reason);
      if (message.turns) {
        setTurns(message.turns);
        setCut(message.cut);
      } else if (message.reason) {
        setTurns(null);
      }
    });
    const poll = () => conn.send({ t: 'read_record', pane, mtime: mtime.current });
    poll();
    const timer = setInterval(poll, POLL_MS);
    return () => {
      off();
      clearInterval(timer);
    };
  }, [conn, pane]);

  const newest = turns ? [...turns].reverse() : [];
  return (
    <SidePanel title="기록" sub={title} onClose={onClose}>
      {reason && <p className="px-4 text-muted-foreground">{reason}</p>}
      <ol className="m-0 flex list-none flex-col gap-1 px-2 pb-3">
        {newest.map((turn, index) => {
          const number = newest.length - index;
          const expanded = open === number;
          return (
            <li key={number}>
              <button
                onClick={() => setOpen(expanded ? null : number)}
                className={cn('w-full rounded-xl px-3 py-2 text-left hover:bg-muted', expanded && 'bg-muted')}
              >
                <div className="mb-0.5 flex items-center gap-2 text-[11.5px] text-muted-foreground">
                  <span className="font-mono">#{number}</span>
                  {turn.at && <span>{new Date(turn.at).toLocaleString('ko-KR', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })}</span>}
                  {turn.answers && <span className="rounded bg-background px-1.5">질문에 답함</span>}
                </div>
                <div className={cn('whitespace-pre-wrap break-words', !expanded && 'line-clamp-2')}>{turn.request}</div>
              </button>
              {expanded && turn.reply && (
                <div className="mx-3 mt-1 mb-2 border-l-2 pl-3 text-[12.5px] whitespace-pre-wrap break-words text-muted-foreground">
                  {turn.reply}
                </div>
              )}
            </li>
          );
        })}
      </ol>
      {cut && <p className="px-4 pb-4 text-[12px] text-muted-foreground">기록 끝 4MiB만 읽었어요. 더 오래된 요청은 빠져 있어요.</p>}
    </SidePanel>
  );
}
