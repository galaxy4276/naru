import { useEffect, useMemo, useRef, useState } from 'react';
import { MessageSquareText, Search, SquareTerminal } from 'lucide-react';
import type { Connection, Conversation, PaneId, Workspace } from '@/connection';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog';
import { cn } from '@/lib/utils';
import { type Teams, agoLabel, basename, cardLine, panesOf, paneTitle, tabTitle } from './model';

const SEARCH_DELAY_MS = 120;
const MAX_SESSIONS = 6;

type Item =
  | { kind: 'session'; key: string; pane: PaneId; title: string; sub: string }
  | { kind: 'conversation'; key: string; conversation: Conversation; open: PaneId | null };

interface Props {
  conn: Connection;
  ws: Workspace;
  teams: Teams;
  now: number;
  onClose: () => void;
}

/** ⌘K: open session cards and past Claude conversations, by the words in them. */
export function Palette({ conn, ws, teams, now, onClose }: Props) {
  const [query, setQuery] = useState('');
  const [found, setFound] = useState<{ query: string; list: Conversation[] }>({ query: '', list: [] });
  const [index, setIndex] = useState(0);
  const list = useRef<HTMLDivElement>(null);

  useEffect(
    () =>
      conn.onMessage((message) => {
        if (message.t === 'conversations') setFound({ query: message.query, list: message.found });
      }),
    [conn],
  );
  useEffect(() => {
    if (!query.trim()) return;
    const timer = setTimeout(() => conn.send({ t: 'search_conversations', query }), SEARCH_DELAY_MS);
    return () => clearTimeout(timer);
  }, [conn, query]);

  const items = useMemo((): Item[] => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    const sessions: Item[] = ws.tabs
      .map((tab) => {
        const panes = panesOf(tab.layout);
        const hay = [tabTitle(ws, tab), ...panes.flatMap((pane) => [paneTitle(ws, pane), ws.panes[pane]?.cwd, ws.panes[pane]?.session?.query])]
          .filter(Boolean)
          .join('\n')
          .toLowerCase();
        return { tab, hay };
      })
      .filter(({ hay }) => words.every((word) => hay.includes(word)))
      .slice(0, MAX_SESSIONS)
      .map(({ tab }) => ({
        kind: 'session',
        key: `s${tab.id}`,
        pane: tab.focused,
        title: tabTitle(ws, tab),
        sub: cardLine(ws, teams, tab)?.text ?? '',
      }));
    const showing = (id: string) =>
      Object.entries(ws.panes).find(([, record]) => record.session?.session_id === id)?.[0] ?? null;
    const conversations: Item[] =
      words.length && found.query === query
        ? found.list.map((conversation) => ({
            kind: 'conversation',
            key: `c${conversation.session_id}`,
            conversation,
            open: showing(conversation.session_id),
          }))
        : [];
    return [...sessions, ...conversations];
  }, [ws, teams, query, found]);

  useEffect(() => setIndex(0), [query]);
  useEffect(() => {
    list.current?.querySelector(`[data-index="${index}"]`)?.scrollIntoView({ block: 'nearest' });
  }, [index]);

  const pick = (item: Item | undefined, here: boolean) => {
    if (!item) return;
    if (item.kind === 'session') conn.send({ t: 'focus', pane: item.pane });
    else if (item.open) conn.send({ t: 'focus', pane: item.open });
    else conn.send({ t: 'resume_conversation', session_id: item.conversation.session_id, cwd: item.conversation.cwd, here });
    onClose();
  };

  const firstConversation = items.findIndex((item) => item.kind === 'conversation');
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent showCloseButton={false} className="top-[12%] translate-y-0 gap-0 overflow-hidden rounded-2xl border-0 p-0 sm:max-w-[620px]">
        <DialogTitle className="sr-only">세션·대화 찾기</DialogTitle>
        <DialogDescription className="sr-only">열린 세션 카드와 지난 Claude 대화를 찾아요.</DialogDescription>
        <div className="flex h-12 items-center gap-2.5 px-4">
          <Search className="size-4 text-muted-foreground" />
          <input
            autoFocus
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.nativeEvent.isComposing) return;
              if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
                event.preventDefault();
                const step = event.key === 'ArrowDown' ? 1 : -1;
                setIndex((now) => (items.length ? (now + step + items.length) % items.length : 0));
              } else if (event.key === 'Enter') {
                event.preventDefault();
                pick(items[index], event.metaKey);
              }
            }}
            placeholder="세션이나 대화에서 한 말로 찾기"
            className="h-full flex-1 bg-transparent text-[15px] outline-none placeholder:text-muted-foreground"
          />
        </div>
        <div ref={list} className="max-h-[420px] overflow-y-auto px-2 pb-2">
          {items.length === 0 && (
            <p className="px-3 py-6 text-center text-muted-foreground">
              {query.trim() && found.query !== query ? '찾는 중…' : '맞는 세션이나 대화가 없어요'}
            </p>
          )}
          {items.map((item, at) => (
            <div key={item.key}>
              {(at === 0 && item.kind === 'session') || at === firstConversation ? (
                <p className="px-3 pt-2.5 pb-1 text-[11.5px] font-semibold text-muted-foreground">{item.kind === 'session' ? '세션' : '대화'}</p>
              ) : null}
              <button
                data-index={at}
                onMouseMove={() => setIndex(at)}
                onClick={(event) => pick(item, event.metaKey)}
                className={cn('flex w-full items-center gap-3 rounded-xl px-3 py-2 text-left', at === index && 'bg-muted')}
              >
                {item.kind === 'session' ? (
                  <>
                    <SquareTerminal className="size-4 shrink-0 text-muted-foreground" />
                    <span className="flex min-w-0 flex-1 flex-col">
                      <b className="truncate font-semibold">{item.title}</b>
                      {item.sub && <span className="truncate text-[12px] text-muted-foreground">{item.sub}</span>}
                    </span>
                  </>
                ) : (
                  <>
                    <MessageSquareText className="size-4 shrink-0 text-muted-foreground" />
                    <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                      <b className="truncate font-semibold">{item.conversation.first}</b>
                      <span className="flex min-w-0 items-center gap-1.5 text-[12px] text-muted-foreground">
                        <span className="shrink-0 rounded-full bg-muted px-1.5 font-semibold text-foreground">{basename(item.conversation.cwd)}</span>
                        <span className="truncate">마지막: {item.conversation.last}</span>
                      </span>
                    </span>
                    <span className="shrink-0 text-[12px] text-muted-foreground">{item.open ? '열려 있음' : agoLabel(item.conversation.modified, now)}</span>
                  </>
                )}
              </button>
            </div>
          ))}
        </div>
        <div className="flex gap-3 bg-muted/60 px-4 py-2 text-[11.5px] text-muted-foreground">
          <span>Enter 열기</span>
          <span>⌘Enter 지금 pane에서 이어 가기</span>
          <span className="ml-auto">Esc 닫기</span>
        </div>
      </DialogContent>
    </Dialog>
  );
}
