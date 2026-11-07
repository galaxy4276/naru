import { useEffect, useState } from 'react';
import type { Connection, PaneId } from '@/connection';
import { TerminalView } from '@/TerminalView';
import { Button } from '@/components/ui/button';

/** 마케팅 모드: organic-lab's screen in the middle, its claude session on the right. */
export function Marketing({ conn, visible }: { conn: Connection; visible: boolean }) {
  const [url, setUrl] = useState<string | null>(null);
  const [up, setUp] = useState<boolean | null>(null);
  const [pane, setPane] = useState<PaneId | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);

  useEffect(() => {
    const off = conn.onMessage((message) => {
      if (message.t !== 'marketing') return;
      setError(message.error);
      if (message.error) return;
      if (message.url) setUrl(message.url);
      if (message.pane) setPane(message.pane);
      setUp(message.up);
      if (message.up) setStarting(false);
    });
    conn.send({ t: 'marketing_open' });
    return off;
  }, [conn]);

  // ponytail: polls while the server boots; stops when it answers or after 60s.
  useEffect(() => {
    if (!starting) return;
    const tick = setInterval(() => conn.send({ t: 'marketing_check' }), 2000);
    const give = setTimeout(() => setStarting(false), 60_000);
    return () => (clearInterval(tick), clearTimeout(give));
  }, [conn, starting]);

  const start = () => {
    setStarting(true);
    conn.send({ t: 'marketing_start_server' });
  };

  return (
    <div className="flex min-w-0 flex-1 gap-2 py-2 pr-2">
      <section className="relative min-w-0 flex-1 overflow-hidden rounded-2xl bg-card">
        {error && <p className="absolute inset-x-3 top-3 z-10 rounded-xl bg-destructive/5 px-3 py-2 text-sm text-destructive">{error}</p>}
        {up && url ? (
          <iframe src={url} title="오가닉랩" className="size-full border-0" />
        ) : up === false ? (
          <div className="flex h-full flex-col items-center justify-center gap-3 text-center">
            <b>오가닉랩 서버가 꺼져 있어요</b>
            <p className="text-sm text-muted-foreground">{starting ? '서버를 켜는 중이에요' : '켜면 여기에 오가닉랩 화면이 떠요'}</p>
            <Button onClick={start} disabled={starting}>
              {starting && <i className="size-3 animate-spin rounded-full border-2 border-current border-t-transparent" />}
              서버 켜기
            </Button>
          </div>
        ) : (
          !error && <div className="flex h-full items-center justify-center text-sm text-muted-foreground">오가닉랩을 여는 중이에요</div>
        )}
      </section>
      <aside className="relative w-[400px] shrink-0 overflow-hidden rounded-2xl bg-card">
        {pane ? (
          <TerminalView conn={conn} pane={pane} focused={visible} />
        ) : (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">세션을 여는 중이에요</div>
        )}
      </aside>
    </div>
  );
}
