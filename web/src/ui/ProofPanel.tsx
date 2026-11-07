import { useEffect, useRef, useState } from 'react';
import type { Connection, Id, PaneId, Workspace } from '@/connection';
import { cn } from '@/lib/utils';
import { basename } from './model';
import { type Decision, type Review, parseDecisions, parseReview, reverted } from './proof';
import { SidePanel } from './SidePanel';

const POLL_MS = 3000;

/** Reads every review an agent sent with `naru proof`, so the queue can count its problems. */
export function useReviews(conn: Connection | null, ws: Workspace | null): Record<Id, Review | null> {
  const [reviews, setReviews] = useState<Record<Id, Review | null>>({});
  const asked = useRef(new Set<Id>());
  useEffect(
    () =>
      conn?.onMessage((message) => {
        if (message.t === 'view_body' && asked.current.has(message.view)) {
          const review = message.kind === 'markdown' || message.kind === 'text' ? parseReview(message.text ?? '') : null;
          setReviews((all) => ({ ...all, [message.view]: review }));
        }
      }),
    [conn],
  );
  useEffect(() => {
    if (!conn?.isOpen) return;
    for (const view of ws?.views ?? []) {
      if (view.proof && !asked.current.has(view.id)) {
        asked.current.add(view.id);
        conn.send({ t: 'read_view', view: view.id });
      }
    }
  }, [conn, ws]);
  return reviews;
}

interface Props {
  conn: Connection;
  ws: Workspace;
  pane: PaneId | null;
  title: string;
  reviews: Record<Id, Review | null>;
  onClose: () => void;
}

/** What the focused session proved: its last review and the decision log in its folder. */
export function ProofPanel({ conn, ws, pane, title, reviews, onClose }: Props) {
  const [tab, setTab] = useState<'review' | 'decisions'>('review');
  const sent = ws.views.filter((view) => view.proof && view.from === pane);
  const latest = sent.at(-1);
  const review = latest ? reviews[latest.id] : undefined;

  // Looking at the panel is looking at the review: it leaves the queue.
  useEffect(() => {
    for (const view of sent) if (!view.seen) conn.send({ t: 'view_seen', view: view.id });
  }, [conn, sent]);

  return (
    <SidePanel title="증명" sub={title} onClose={onClose}>
      <div className="mx-3 mb-2 grid grid-cols-2 gap-1 rounded-[10px] bg-muted p-1 text-[12.5px]">
        {(['review', 'decisions'] as const).map((key) => (
          <button
            key={key}
            onClick={() => setTab(key)}
            className={cn('h-7 rounded-lg text-muted-foreground', tab === key && 'bg-background font-semibold text-foreground')}
          >
            {key === 'review' ? '검토' : '결정'}
          </button>
        ))}
      </div>
      {tab === 'review' ? (
        !latest ? (
          <p className="px-4 text-muted-foreground">이 세션이 naru proof로 보낸 검토가 없어요.</p>
        ) : review === undefined ? null : review === null ? (
          <p className="px-4 text-muted-foreground">{basename(latest.path)}에 ## Findings가 없어요.</p>
        ) : (
          <ReviewView review={review} name={basename(latest.path) ?? ''} />
        )
      ) : (
        <DecisionsView conn={conn} pane={pane} />
      )}
    </SidePanel>
  );
}

function ReviewView({ review, name }: { review: Review; name: string }) {
  return (
    <div className="flex flex-col gap-2 px-3 pb-4">
      <p className="m-0 px-1 text-muted-foreground">
        {name} · 증명된 문제 <b className="text-foreground">{review.findings.length}개</b>
      </p>
      {review.findings.map((finding, index) => (
        <section key={index} className="rounded-xl bg-muted p-3">
          <b className="block">{finding.title}</b>
          {finding.foundBy && <span className="text-[12px] text-muted-foreground">{finding.foundBy}</span>}
          {finding.note && <p className="my-1.5 text-[12.5px]">{finding.note}</p>}
          {finding.command && <pre className="my-1 rounded-lg bg-background p-2 font-mono text-[11.5px] whitespace-pre-wrap">{finding.command}</pre>}
          {finding.output && <pre className="my-1 rounded-lg bg-background p-2 font-mono text-[11.5px] whitespace-pre-wrap text-muted-foreground">{finding.output}</pre>}
        </section>
      ))}
      {review.dropped.length > 0 && (
        <details className="px-1 text-[12.5px] text-muted-foreground">
          <summary>버린 것 {review.droppedCount}개</summary>
          <ul className="mt-1 pl-4">
            {review.dropped.map((line) => <li key={line}>{line}</li>)}
          </ul>
        </details>
      )}
    </div>
  );
}

function DecisionsView({ conn, pane }: { conn: Connection; pane: PaneId | null }) {
  const [log, setLog] = useState<{ path: string | null; rows: Decision[]; broken: number }>({ path: null, rows: [], broken: 0 });
  const mtime = useRef<number | null>(null);
  useEffect(() => {
    mtime.current = null;
    setLog({ path: null, rows: [], broken: 0 });
    if (!pane) return;
    const off = conn.onMessage((message) => {
      if (message.t !== 'decisions' || message.pane !== pane) return;
      mtime.current = message.mtime;
      if (message.text !== null) setLog({ path: message.path, ...parseDecisions(message.text) });
      else if (!message.path) setLog({ path: null, rows: [], broken: 0 });
    });
    const poll = () => conn.send({ t: 'read_decisions', pane, mtime: mtime.current });
    poll();
    const timer = setInterval(poll, POLL_MS);
    return () => {
      off();
      clearInterval(timer);
    };
  }, [conn, pane]);
  if (!log.path) return <p className="px-4 text-muted-foreground">이 폴더에 decisions.tsv나 .audit/*.tsv가 없어요.</p>;
  return (
    <div className="flex flex-col gap-1.5 px-3 pb-4">
      <p className="m-0 truncate px-1 text-[12px] text-muted-foreground" title={log.path}>
        {basename(log.path)}
        {log.broken > 0 && ` · 읽지 못한 줄 ${log.broken}개`}
      </p>
      {[...log.rows].reverse().map((row, index) => (
        <section key={index} className={cn('rounded-xl bg-muted px-3 py-2', reverted(row) && 'bg-orange-50')}>
          <div className="flex gap-2 text-[11.5px] text-muted-foreground">
            <span>{row.at}</span>
            <span>{row.phase}</span>
          </div>
          <b className="block">{row.decision}</b>
          <p className="my-0.5 text-[12.5px]">{row.why}</p>
          <p className="my-0 text-[12px] text-muted-foreground">{row.evidence}</p>
          <p className={cn('my-0 text-[12px]', reverted(row) ? 'text-turn' : 'text-muted-foreground')}>{row.result}</p>
        </section>
      ))}
    </div>
  );
}
