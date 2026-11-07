import { useEffect, useState } from 'react';
import { Globe, Power } from 'lucide-react';
import type { ClientMsg, Connection, DevServer, Workspace } from '@/connection';
import { Button } from '@/components/ui/button';
import { SidePanel } from './SidePanel';
import { paneTitle, panesOf } from './model';

const POLL_MS = 3000;

/** `/Library/…/Python -m http.server` reads as `Python -m http.server`. */
const shortCommand = (command: string) => command.replace(/^\S*\//, '');

function since(seconds: number): string {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (hours) return `${hours}시간 ${String(minutes).padStart(2, '0')}분`;
  return minutes ? `${minutes}분` : '방금';
}

/** Ports that programs started in Naru panes listen on. */
export function ServerPanel({ conn, ws, send, onClose }: { conn: Connection; ws: Workspace; send: (message: ClientMsg) => void; onClose: () => void }) {
  const [servers, setServers] = useState<DevServer[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [stopping, setStopping] = useState<number[]>([]);

  useEffect(() => {
    const off = conn.onMessage((message) => {
      if (message.t !== 'servers') return;
      setServers(message.servers);
      setError(message.error);
      setStopping((pids) => pids.filter((pid) => message.servers.some((server) => server.pid === pid)));
    });
    const poll = () => conn.send({ t: 'servers' });
    poll();
    const timer = setInterval(poll, POLL_MS);
    return () => {
      off();
      clearInterval(timer);
    };
  }, [conn]);

  return (
    <SidePanel title="개발 서버" sub={servers ? `${servers.length}개` : undefined} onClose={onClose}>
      {error && <p className="px-4 text-muted-foreground">{error}</p>}
      {servers?.length === 0 && <p className="px-4 text-muted-foreground">pane에서 띄운 서버가 없어요</p>}
      <ul className="m-0 flex list-none flex-col px-2 pb-3">
        {servers?.map((server) => {
          const record = ws.panes[server.pane];
          const group = ws.tabs.find((tab) => panesOf(tab.layout).includes(server.pane))?.group;
          const project = ws.groups.find((g) => g.id === group)?.name;
          return (
            <li key={`${server.pid}:${server.port}`} className="flex flex-col gap-1 border-t border-muted px-2 py-2.5 first:border-t-0">
              <div className="flex items-center gap-2">
                <b className="font-mono text-[14px]">:{server.port}</b>
                <code className="min-w-0 flex-1 truncate rounded-md bg-muted px-1.5 py-0.5 text-[11px]" title={server.command}>
                  {shortCommand(server.command)}
                </code>
                <span className="shrink-0 text-[11px] text-muted-foreground">{since(server.age)}</span>
              </div>
              <button onClick={() => send({ t: 'focus', pane: server.pane })} className="truncate text-left text-[11.5px] text-muted-foreground hover:underline">
                {[project, record ? paneTitle(ws, server.pane) : null].filter(Boolean).join(' · ')}
              </button>
              <div className="flex gap-1.5">
                <Button size="xs" onClick={() => send({ t: 'open_link', url: `http://localhost:${server.port}` })}>
                  <Globe /> 열기
                </Button>
                <Button
                  size="xs"
                  variant="secondary"
                  disabled={stopping.includes(server.pid)}
                  onClick={() => {
                    setStopping((pids) => [...pids, server.pid]);
                    send({ t: 'stop_server', pid: server.pid });
                  }}
                  className="text-red-600 dark:text-red-400"
                >
                  <Power /> {stopping.includes(server.pid) ? '끄는 중' : '끄기'}
                </Button>
              </div>
            </li>
          );
        })}
      </ul>
    </SidePanel>
  );
}
