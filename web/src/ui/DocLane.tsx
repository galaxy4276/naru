import { useEffect, useRef, useState } from 'react';
import Markdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Copy, Maximize, Minus, Plus, X } from 'lucide-react';
import type { Connection, Id, ServerMsg, View } from '@/connection';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { useEdgeWidth } from './frame';
import { basename } from './model';

type Body = Extract<ServerMsg, { t: 'view_body' }>;

const WIDTH_KEY = 'naru.lane.width';
const MIN_WIDTH = 260;
const MAX_WIDTH = 900;

interface Props {
  conn: Connection;
  views: View[];
  active: Id | null;
  /** The user picked a tab: show it and count it as read. */
  onPick: (view: Id) => void;
}

/** Files agents hand over with `naru open`, as tabs beside the rail. Focus stays in the pane. */
export function DocLane({ conn, views, active, onPick }: Props) {
  const [width, grab] = useEdgeWidth(WIDTH_KEY, 380, MIN_WIDTH, MAX_WIDTH, 'left');
  const [bodies, setBodies] = useState<Record<Id, Body>>({});
  const current = views.find((view) => view.id === active) ?? views[views.length - 1];

  useEffect(
    () =>
      conn.onMessage((message) => {
        if (message.t === 'view_body') setBodies((all) => ({ ...all, [message.view]: message }));
      }),
    [conn],
  );
  useEffect(() => {
    if (current && !bodies[current.id]) conn.send({ t: 'read_view', view: current.id });
  }, [conn, current, bodies]);

  if (!current) return null;
  const body = bodies[current.id];
  return (
    <aside className="relative flex min-w-[200px] flex-col overflow-hidden rounded-2xl bg-background" style={{ width }}>
      <div onPointerDown={grab} className="absolute inset-y-0 left-0 z-10 w-1.5 cursor-col-resize" />
      <div className="flex shrink-0 items-center gap-1 overflow-x-auto p-1.5">
        {views.map((view) => (
          <div
            key={view.id}
            className={cn(
              'group/tab flex h-7 shrink-0 items-center gap-1.5 rounded-lg pr-1 pl-2.5 text-[12.5px] text-muted-foreground',
              view.id === current.id && 'bg-muted font-semibold text-foreground',
            )}
          >
            <button className="flex items-center gap-1.5" onClick={() => onPick(view.id)} title={view.path}>
              {!view.seen && <i className="size-1.5 rounded-full bg-turn" />}
              {basename(view.path)}
            </button>
            <button
              aria-label="탭 닫기"
              className="invisible rounded p-0.5 hover:bg-background group-hover/tab:visible"
              onClick={() => conn.send({ t: 'close_view', view: view.id })}
            >
              <X className="size-3" />
            </button>
          </div>
        ))}
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        {!body ? null : body.kind === 'error' ? (
          <p className="p-4 text-muted-foreground">{body.text}</p>
        ) : body.kind === 'image' ? (
          <ImageView body={body} path={current.path} />
        ) : body.kind === 'html' ? (
          // A learning session's picture. Never add allow-same-origin: the page would reach this
          // window and its daemon token.
          <iframe title={basename(current.path) ?? ''} sandbox="allow-scripts" srcDoc={body.text} className="size-full border-0 bg-white" />
        ) : body.kind === 'markdown' ? (
          <article className="prose prose-sm prose-zinc max-w-none px-5 py-3 prose-headings:font-semibold prose-pre:rounded-xl prose-pre:bg-muted prose-pre:text-foreground">
            <Markdown
              remarkPlugins={[remarkGfm]}
              components={{
                // Web links open in the browser; anything else would leave the app window or run something.
                a: ({ href, children }) =>
                  href && /^https?:\/\//.test(href) ? (
                    <a
                      href={href}
                      title={href}
                      onClick={(event) => {
                        event.preventDefault();
                        conn.send({ t: 'open_link', url: href });
                      }}
                    >
                      {children}
                    </a>
                  ) : (
                    <span title={href} className="underline">
                      {children}
                    </span>
                  ),
              }}
            >
              {body.text ?? ''}
            </Markdown>
          </article>
        ) : (
          <pre className="m-0 px-4 py-3 font-mono text-[12px] leading-relaxed whitespace-pre-wrap">{body.text}</pre>
        )}
      </div>
    </aside>
  );
}

function ImageView({ body, path }: { body: Body; path: string }) {
  const [zoom, setZoom] = useState<number | null>(null);
  const image = useRef<HTMLImageElement>(null);
  const src = `data:${body.mime};base64,${body.data}`;
  const step = (factor: number) => {
    const now = zoom ?? (image.current ? image.current.width / image.current.naturalWidth : 1);
    setZoom(Math.min(6, Math.max(0.25, Math.round(now * factor * 100) / 100)));
  };
  const copy = async () => {
    const element = image.current;
    if (!element) return;
    const canvas = document.createElement('canvas');
    canvas.width = element.naturalWidth;
    canvas.height = element.naturalHeight;
    canvas.getContext('2d')!.drawImage(element, 0, 0);
    const png = await new Promise<Blob>((resolve, reject) =>
      canvas.toBlob((blob) => (blob ? resolve(blob) : reject(new Error('cannot encode PNG'))), 'image/png'),
    );
    await navigator.clipboard.write([
      new ClipboardItem({ 'image/png': png, 'text/plain': new Blob([path], { type: 'text/plain' }) }),
    ]);
  };
  return (
    <div className="flex h-full flex-col">
      <div className="flex shrink-0 items-center gap-1 px-2 pb-1.5 text-[12px] text-muted-foreground">
        <Button variant="ghost" size="icon-xs" onClick={() => step(1 / 1.25)} aria-label="축소">
          <Minus />
        </Button>
        <span className="w-10 text-center tabular-nums">{zoom ? `${Math.round(zoom * 100)}%` : '맞춤'}</span>
        <Button variant="ghost" size="icon-xs" onClick={() => step(1.25)} aria-label="확대">
          <Plus />
        </Button>
        <Button variant="ghost" size="icon-xs" onClick={() => setZoom(null)} aria-label="맞춤">
          <Maximize />
        </Button>
        <Button variant="ghost" size="xs" className="ml-auto" onClick={() => void copy()}>
          <Copy />
          복사
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-2 pb-2">
        <img
          ref={image}
          src={src}
          alt={basename(path) ?? ''}
          className={cn('rounded-lg', zoom === null && 'max-w-full')}
          style={zoom === null ? undefined : { width: `${(image.current?.naturalWidth ?? 0) * zoom}px`, maxWidth: 'none' }}
        />
      </div>
    </div>
  );
}
