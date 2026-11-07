import { useEffect, useState } from 'react';
import { Check, Clock, GitPullRequest, Minus, RefreshCw, X } from 'lucide-react';
import type { ClientMsg, Connection, Pr, RepoPrs, Workspace } from '@/connection';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { SidePanel } from './SidePanel';
import { paneTitle } from './model';

// gh goes to GitHub per repository; once a minute is plenty for check states.
const POLL_MS = 60_000;

const CHECKS: Record<Pr['checks'], { label: string; icon: React.ReactNode; className?: string }> = {
  pass: { label: '통과', icon: <Check /> },
  running: { label: '도는 중', icon: <Clock />, className: 'text-muted-foreground' },
  fail: { label: '실패', icon: <X />, className: 'bg-red-50 text-red-600 dark:bg-red-950 dark:text-red-400' },
  none: { label: '체크 없음', icon: <Minus />, className: 'text-muted-foreground' },
};

/** Open pull requests of every project's repository, with their checks and the sessions on them. */
export function PrPanel({ conn, ws, send, onClose }: { conn: Connection; ws: Workspace; send: (message: ClientMsg) => void; onClose: () => void }) {
  const [repos, setRepos] = useState<RepoPrs[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [onlyFailed, setOnlyFailed] = useState(false);

  useEffect(() => {
    const off = conn.onMessage((message) => {
      if (message.t !== 'prs') return;
      setRepos(message.repos);
      setLoading(false);
    });
    const poll = () => {
      setLoading(true);
      conn.send({ t: 'prs' });
    };
    poll();
    const timer = setInterval(poll, POLL_MS);
    return () => {
      off();
      clearInterval(timer);
    };
  }, [conn]);

  // PRs a session works on come first, from any repository.
  const all = (repos ?? []).flatMap((repo) => repo.prs.map((pr) => ({ repo, pr }))).sort((a, b) => Number(!a.pr.panes.length) - Number(!b.pr.panes.length));
  const failed = all.filter(({ pr }) => pr.checks === 'fail').length;
  const shown = onlyFailed ? all.filter(({ pr }) => pr.checks === 'fail') : all;
  const errors = (repos ?? []).filter((repo) => repo.error);
  const segment = (on: boolean, label: string, onClick: () => void) => (
    <button onClick={onClick} className={cn('h-6 rounded-md px-2 text-[11.5px] font-semibold', on ? 'bg-background shadow-sm' : 'text-muted-foreground')}>
      {label}
    </button>
  );
  return (
    <SidePanel
      title="PR"
      actions={
        <>
          <span className="flex rounded-lg bg-muted p-0.5">
            {segment(!onlyFailed, `열림 ${all.length}`, () => setOnlyFailed(false))}
            {segment(onlyFailed, `실패 ${failed}`, () => setOnlyFailed(true))}
          </span>
          <Button variant="ghost" size="icon-xs" aria-label="새로 읽기" onClick={() => (setLoading(true), conn.send({ t: 'prs' }))}>
            <RefreshCw className={cn(loading && 'animate-spin')} />
          </Button>
        </>
      }
      onClose={onClose}
    >
      {repos === null && <p className="px-4 text-muted-foreground">GitHub에서 읽는 중…</p>}
      {repos !== null && shown.length === 0 && (
        <p className="px-4 text-muted-foreground">{onlyFailed ? '체크가 실패한 PR이 없어요' : '열린 PR이 없어요'}</p>
      )}
      <ul className="m-0 flex list-none flex-col px-2 pb-3">
        {shown.map(({ repo, pr }) => {
          const check = CHECKS[pr.checks];
          const agent = pr.panes.map((pane) => ws.panes[pane]?.session?.agent).find(Boolean);
          return (
            <li key={`${repo.repo}#${pr.number}`} className="flex flex-col gap-1 border-t border-muted px-2 py-2.5 first:border-t-0">
              <div className="flex items-center gap-1.5">
                <b className="tabular-nums">#{pr.number}</b>
                <span className={cn('flex h-5 items-center gap-1 rounded-md bg-muted px-1.5 text-[11px] font-semibold [&_svg]:size-3', check.className)}>
                  {check.icon}
                  {check.label}
                </span>
                {pr.draft && <span className="text-[11px] text-muted-foreground">초안</span>}
                <span className="flex-1" />
                {agent && <span className="rounded-md bg-muted px-1.5 text-[11px] font-semibold">{agent}</span>}
              </div>
              <button onClick={() => send({ t: 'open_link', url: pr.url })} className="text-left text-[13px] leading-snug hover:underline" title="GitHub에서 열기">
                {pr.title}
              </button>
              <span className="truncate font-mono text-[11px] text-muted-foreground">
                {repo.name} · {pr.branch}
              </span>
              {pr.panes.length > 0 && (
                <div className="flex flex-wrap gap-1">
                  {pr.panes.map((pane) => (
                    <button key={pane} onClick={() => send({ t: 'focus', pane })} className="flex h-6 items-center gap-1 rounded-md bg-muted px-2 text-[11.5px] hover:bg-accent">
                      <GitPullRequest className="size-3" />
                      <span className="max-w-[220px] truncate">{paneTitle(ws, pane)}</span>
                    </button>
                  ))}
                </div>
              )}
            </li>
          );
        })}
      </ul>
      {errors.length > 0 && (
        <details className="px-4 pb-3 text-[11.5px] text-muted-foreground">
          <summary className="cursor-pointer">읽지 못한 저장소 {errors.length}개</summary>
          {errors.map((repo) => (
            <p key={repo.repo} className="mt-1 truncate" title={repo.error ?? ''}>
              <b>{repo.name || 'gh'}</b> {repo.error}
            </p>
          ))}
        </details>
      )}
    </SidePanel>
  );
}
