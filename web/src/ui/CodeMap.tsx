import { useEffect, useMemo, useRef, useState } from 'react';
import { RefreshCw, X } from 'lucide-react';
import type { CodeMap as CodeMapData, CodeMapFile, Connection, PaneId, Workspace } from '@/connection';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { fitted, grabFrame, loadFrame } from './frame';
import { paneTitle } from './model';

const FRAME_KEY = 'naru.codemap.frame';
const MIN = { w: 640, h: 420 };
// Agents keep editing while the map is open.
const POLL_MS = 5000;
const COL = 260;
const ROW = 74;
const CARD_W = 210;
/** Unchanged lines kept around a change before the rest folds. */
const CONTEXT = 3;

const base = (path: string) => path.split('/').pop() ?? path;
const dir = (path: string) => path.split('/').slice(0, -1).join('/');

/** Importers sit left of what they import; a cycle stops growing after one pass per file. */
function layout(map: CodeMapData) {
  const level = map.files.map(() => 0);
  for (let pass = 0; pass < map.files.length; pass++) {
    let moved = false;
    for (const [from, to] of map.edges) {
      if (level[to] < level[from] + 1) {
        level[to] = level[from] + 1;
        moved = true;
      }
    }
    if (!moved) break;
  }
  const rows: number[] = [];
  return map.files.map((_, i) => {
    const row = (rows[level[i]] = (rows[level[i]] ?? -1) + 1);
    return { x: 24 + level[i] * COL, y: 24 + row * ROW };
  });
}

/** One `<span>` per token; the text is escaped first. */
const KEYWORDS =
  /\b(import|from|export|default|function|const|let|var|return|if|else|for|while|match|pub|fn|struct|enum|use|impl|mut|async|await|type|interface|new|true|false|null|undefined|Some|None|Ok|Err|self|Self|mod|crate|class|extends|try|catch|throw)\b/g;
function highlight(line: string) {
  const escaped = line.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  const comment = escaped.search(/\/\/|^\s*\/\*|^\s*\*|^\s*#(?!\[)/);
  const code = comment >= 0 ? escaped.slice(0, comment) : escaped;
  const tail = comment >= 0 ? `<span class="text-muted-foreground/70 italic">${escaped.slice(comment)}</span>` : '';
  return (
    code
      .split(/('[^']*'|"[^"]*"|`[^`]*`)/)
      .map((part, i) => (i % 2 ? `<span class="text-muted-foreground">${part}</span>` : part.replace(KEYWORDS, '<b class="font-bold">$1</b>')))
      .join('') + tail
  );
}

type Row = { kind: 'line'; n: number; text: string; mark: boolean } | { kind: 'gone'; text: string } | { kind: 'fold'; from: number; count: number };

/** The file as rows: changed and removed lines with a few lines around them, the rest folded. */
function rowsOf(file: CodeMapFile, open: Set<number>): Row[] {
  const marks = new Set(file.marks);
  const near = new Set<number>();
  const anchors = [...file.marks, ...file.gone.flatMap((gone) => [gone.after, gone.after + 1])];
  for (const at of anchors) for (let n = at - CONTEXT; n <= at + CONTEXT; n++) near.add(n);
  const goneAt = (after: number) => file.gone.filter((gone) => gone.after === after).flatMap((gone) => gone.lines.map((text): Row => ({ kind: 'gone', text })));
  const rows: Row[] = [...goneAt(0)];
  for (let n = 1; n <= file.lines.length; n++) {
    if (!near.has(n) && !open.has(n)) {
      let end = n;
      while (end + 1 <= file.lines.length && !near.has(end + 1)) end++;
      // A short gap reads better than a fold.
      if (end - n >= 5) {
        rows.push({ kind: 'fold', from: n, count: end - n + 1 });
        n = end;
        continue;
      }
    }
    rows.push({ kind: 'line', n, text: file.lines[n - 1], mark: marks.has(n) });
    rows.push(...goneAt(n));
  }
  rows.push(...file.gone.filter((gone) => gone.after > file.lines.length).flatMap((gone) => gone.lines.map((text): Row => ({ kind: 'gone', text }))));
  return rows;
}

function Code({ file }: { file: CodeMapFile }) {
  const [open, setOpen] = useState(new Set<number>());
  const rows = useMemo(() => rowsOf(file, open), [file, open]);
  const whole = file.status === 'new';
  if (file.skipped) return <p className="px-4 py-3 text-muted-foreground">{file.skipped}</p>;
  return (
    <div className="font-mono text-[12px] leading-[1.62]">
      {rows.map((row, i) => {
        if (row.kind === 'fold')
          return (
            <button
              key={`f${row.from}`}
              onClick={() => setOpen(new Set([...open, ...Array.from({ length: row.count }, (_, k) => row.from + k)]))}
              className="my-1 mr-3.5 ml-11 block w-[calc(100%-3.75rem)] rounded-lg bg-muted px-2.5 py-0.5 text-left font-sans text-[11.5px] text-muted-foreground hover:text-foreground"
            >
              ··· {row.count}줄 접힘
            </button>
          );
        if (row.kind === 'gone')
          return (
            <div key={`g${i}`} className="flex bg-red-50 whitespace-pre text-red-700 shadow-[inset_3px_0_0_#ef4444] dark:bg-red-950/40 dark:text-red-300">
              <i className="w-11 shrink-0 pr-3 text-right not-italic">−</i>
              <span>{row.text || ' '}</span>
            </div>
          );
        return (
          <div
            key={row.n}
            className={cn(
              'flex pr-3.5 whitespace-pre',
              row.mark && !whole && 'bg-orange-50 shadow-[inset_3px_0_0_#f97316] dark:bg-orange-950/40',
              row.mark && whole && 'bg-orange-50/50 dark:bg-orange-950/20',
            )}
          >
            <i className={cn('w-11 shrink-0 pr-3 text-right not-italic', row.mark ? 'text-orange-600' : 'text-muted-foreground/50')}>{row.n}</i>
            <span dangerouslySetInnerHTML={{ __html: highlight(row.text) || ' ' }} />
          </div>
        );
      })}
    </div>
  );
}

function chip(file: CodeMapFile) {
  if (file.status === 'new') return <span className="rounded-md bg-primary px-1.5 text-[10.5px] font-semibold text-primary-foreground">새 파일</span>;
  if (file.status === 'deleted') return <span className="rounded-md bg-red-50 px-1.5 text-[10.5px] font-semibold text-red-600 dark:bg-red-950 dark:text-red-400">지움</span>;
  return (
    <span className="rounded-md bg-orange-50 px-1.5 text-[10.5px] font-semibold text-orange-700 dark:bg-orange-950 dark:text-orange-300">
      +{file.added}
      {file.removed > 0 && ` −${file.removed}`}
    </span>
  );
}

interface Win {
  path: string;
  x: number;
  y: number;
  z: number;
}

/** A file's code over the map; moved by its head, sized by its corner. */
function CodeWindow({ win, file, onMove, onFront, onClose }: { win: Win; file: CodeMapFile; onMove: (x: number, y: number) => void; onFront: () => void; onClose: () => void }) {
  const grab = (event: React.PointerEvent) => {
    if ((event.target as HTMLElement).closest('button')) return;
    event.preventDefault();
    onFront();
    const start = { x: event.clientX - win.x, y: event.clientY - win.y };
    const move = (e: PointerEvent) => onMove(Math.max(0, e.clientX - start.x), Math.max(0, e.clientY - start.y));
    const up = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  };
  return (
    <div
      onPointerDown={onFront}
      className="absolute flex h-[440px] w-[580px] min-w-[320px] resize flex-col overflow-hidden rounded-2xl bg-background shadow-[0_24px_60px_-18px_rgba(0,0,0,0.35),0_2px_8px_rgba(0,0,0,0.08)] animate-in fade-in zoom-in-95"
      style={{ left: win.x, top: win.y, zIndex: win.z }}
    >
      <div onPointerDown={grab} className="flex h-10 shrink-0 cursor-grab items-center gap-2 border-b border-muted pr-2 pl-3.5 select-none active:cursor-grabbing">
        <b className="text-[13.5px]">{base(file.path)}</b>
        {chip(file)}
        <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-muted-foreground">{file.path}</span>
        <Button variant="ghost" size="icon-xs" aria-label="닫기" onClick={onClose}>
          <X />
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto py-1.5 select-text">
        <Code file={file} />
      </div>
    </div>
  );
}

/** The files the focused pane's checkout changed, joined by imports; a card opens its code. */
export function CodeMap({ conn, ws, pane, onClose }: { conn: Connection; ws: Workspace; pane: PaneId | null; onClose: () => void }) {
  const [frame, setFrame] = useState(() => fitted(loadFrame(FRAME_KEY, () => ({ x: 140, y: 70, w: Math.min(1180, window.innerWidth - 200), h: Math.min(760, window.innerHeight - 120) })), MIN));
  const [result, setResult] = useState<{ pane: PaneId; map: CodeMapData | null; error: string | null } | null>(null);
  const [loading, setLoading] = useState(false);
  const [wins, setWins] = useState<Win[]>([]);
  const top = useRef(10);
  const board = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setWins([]);
    setResult(null);
    if (!pane) return;
    const off = conn.onMessage((message) => {
      if (message.t !== 'code_map' || message.pane !== pane) return;
      setResult(message);
      setLoading(false);
    });
    const ask = () => {
      setLoading(true);
      conn.send({ t: 'code_map', pane });
    };
    ask();
    const timer = setInterval(ask, POLL_MS);
    return () => {
      off();
      clearInterval(timer);
    };
  }, [conn, pane]);

  const map = result?.map ?? null;
  const places = useMemo(() => (map ? layout(map) : []), [map]);
  const byPath = new Map((map?.files ?? []).map((file) => [file.path, file]));
  const size = { w: Math.max(0, ...places.map((at) => at.x + CARD_W + 24)), h: Math.max(0, ...places.map((at) => at.y + ROW)) };
  const added = map?.files.reduce((sum, file) => sum + file.added, 0) ?? 0;
  const removed = map?.files.reduce((sum, file) => sum + file.removed, 0) ?? 0;

  // Windows float over the visible part of the map, next to the card when it fits.
  const openFile = (path: string, at: { x: number; y: number }) => {
    const z = ++top.current;
    const view = board.current;
    const [left, scrollTop, width, height] = view ? [view.scrollLeft, view.scrollTop, view.clientWidth, view.clientHeight] : [0, 0, 1200, 700];
    setWins((now) => {
      if (now.some((win) => win.path === path)) return now.map((win) => (win.path === path ? { ...win, z } : win));
      const step = (now.length % 4) * 24;
      const x = Math.max(8, Math.min(at.x - left + CARD_W + 16, width - 588)) + step;
      const y = Math.max(8, Math.min(at.y - scrollTop - 20, height - 448)) + step;
      return [...now, { path, x, y, z }];
    });
  };
  const update = (path: string, change: Partial<Win>) => setWins((now) => now.map((win) => (win.path === path ? { ...win, ...change } : win)));

  return (
    <div
      role="dialog"
      aria-label="코드 지도"
      tabIndex={-1}
      onKeyDown={(event) => event.key === 'Escape' && onClose()}
      className="fixed z-30 flex flex-col overflow-hidden rounded-2xl bg-background shadow-[0_12px_40px_rgba(9,9,11,0.18)] outline-none animate-in fade-in zoom-in-95"
      style={{ left: frame.x, top: frame.y, width: frame.w, height: frame.h }}
    >
      <header
        onPointerDown={(event) => grabFrame(event, frame, false, MIN, FRAME_KEY, setFrame)}
        className="flex h-[46px] shrink-0 cursor-grab items-center gap-2 pr-2.5 pl-[18px] select-none active:cursor-grabbing"
      >
        <b className="text-[14px] font-bold">코드 지도</b>
        {pane && paneTitle(ws, pane) !== base(map?.root ?? '') && <span className="max-w-[260px] truncate text-muted-foreground">{paneTitle(ws, pane)}</span>}
        {map && map.files.length > 0 && (
          <span className="text-[12px] text-muted-foreground tabular-nums">
            {base(map.root)} · 파일 {map.files.length}개 · <span className="text-orange-600">+{added}</span> <span className="text-red-500">−{removed}</span>
          </span>
        )}
        <div className="ml-auto flex gap-0.5">
          <Button variant="ghost" size="icon-xs" aria-label="새로 읽기" onClick={() => pane && (setLoading(true), conn.send({ t: 'code_map', pane }))}>
            <RefreshCw className={cn(loading && 'animate-spin')} />
          </Button>
          <Button variant="ghost" size="icon-xs" aria-label="닫기" onClick={onClose}>
            <X />
          </Button>
        </div>
      </header>
      <div className="relative mx-2.5 mb-2.5 min-h-0 flex-1 overflow-hidden rounded-xl bg-muted">
      <div ref={board} className="size-full overflow-auto">
        {!pane && <p className="p-6 text-muted-foreground">pane을 하나 고르면 그 폴더에서 바뀐 코드를 보여 줘요</p>}
        {pane && !result && <p className="p-6 text-muted-foreground">바뀐 코드를 읽는 중…</p>}
        {result?.error && <p className="p-6 text-muted-foreground">{result.error}</p>}
        {map && map.files.length === 0 && <p className="p-6 text-muted-foreground">커밋하지 않은 변경이 없어요</p>}
        {map && map.files.length > 0 && (
          <div className="relative" style={{ width: size.w, height: size.h + 24 }}>
            <svg className="pointer-events-none absolute inset-0 size-full overflow-visible">
              {map.edges.map(([from, to]) => {
                const a = { x: places[from].x + CARD_W, y: places[from].y + 28 };
                const b = { x: places[to].x, y: places[to].y + 28 };
                const mid = (a.x + b.x) / 2;
                const hot = wins.some((win) => win.path === map.files[from].path || win.path === map.files[to].path);
                return <path key={`${from}-${to}`} d={`M${a.x},${a.y} C${mid},${a.y} ${mid},${b.y} ${b.x},${b.y}`} fill="none" className={hot ? 'stroke-foreground' : 'stroke-border'} strokeWidth={hot ? 1.8 : 1.4} />;
              })}
            </svg>
            {map.files.map((file, i) => (
              <button
                key={file.path}
                onClick={() => openFile(file.path, places[i])}
                title={file.path}
                className={cn(
                  'absolute flex h-14 flex-col justify-center overflow-hidden rounded-xl bg-background px-3 text-left shadow-[0_1px_2px_rgba(0,0,0,0.05)] transition-[translate,box-shadow] hover:-translate-y-0.5 hover:shadow-[0_8px_20px_-8px_rgba(0,0,0,0.25)]',
                  wins.some((win) => win.path === file.path) && 'before:absolute before:top-2 before:bottom-2 before:left-0 before:w-[3px] before:rounded-r-full before:bg-primary',
                )}
                style={{ left: places[i].x, top: places[i].y, width: CARD_W }}
              >
                <span className="flex items-center gap-1.5">
                  <b className={cn('truncate text-[13px]', file.status === 'deleted' && 'line-through')}>{base(file.path)}</b>
                  {chip(file)}
                </span>
                <span className="truncate font-mono text-[11px] text-muted-foreground">{dir(file.path) || '/'}</span>
              </button>
            ))}
            {map.more > 0 && (
              <p className="absolute text-[12px] text-muted-foreground" style={{ left: 24, top: size.h }}>
                파일 {map.more}개는 더 있어요 ({map.files.length}개까지만 보여 줘요)
              </p>
            )}
          </div>
        )}
      </div>
            {wins.map((win) => {
              const file = byPath.get(win.path);
              if (!file) return null;
              return (
                <CodeWindow
                  key={win.path}
                  win={win}
                  file={file}
                  onMove={(x, y) => update(win.path, { x, y })}
                  onFront={() => update(win.path, { z: ++top.current })}
                  onClose={() => setWins((now) => now.filter((other) => other.path !== win.path))}
                />
              );
            })}
      </div>
      <div onPointerDown={(event) => grabFrame(event, frame, true, MIN, FRAME_KEY, setFrame)} className="absolute right-0 bottom-0 size-4 cursor-nwse-resize" aria-hidden />
    </div>
  );
}
