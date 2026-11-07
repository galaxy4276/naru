import { useEffect, useMemo, useRef, useState } from 'react';
import { Minus, Plus, Scan, X } from 'lucide-react';
import type { Connection, Mark, Proposal, ServerMsg } from '@/connection';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { fitted, grabFrame, loadFrame } from './frame';
import { ProposalActions, stageLabel } from './LearnPanel';
import { type Body, seed, settle } from './overviewGraph';

type Data = Extract<ServerMsg, { t: 'learn_map' }>;

const FRAME_KEY = 'naru.learnmap.frame';
const MIN = { w: 560, h: 380 };
const ZOOM = { min: 0.4, max: 3 };
// The map stays light in every theme: black and gray dots only read on a light ground.
const PAPER = '#ffffff';
const INK = '#18181b';
const MUTED = '#71717a';
const GROUND = '#f4f4f5';
const FONT = 'Pretendard, -apple-system, sans-serif';

type Kind = 'note' | 'mark' | 'proposal';

interface Dot {
  key: string;
  kind: Kind;
  title: string;
  /** Index into `groups`; the last one holds what no grouping placed yet. */
  group: number;
  x: number;
  y: number;
  note?: string;
  mark?: Mark;
  proposal?: Proposal;
}

interface Camera {
  zoom: number;
  center: [number, number];
}

const RADIUS: Record<Kind, number> = { note: 5, mark: 9, proposal: 8 };
const ROOM: Record<Kind, number> = { note: 20, mark: 30, proposal: 28 };
const clip = (text: string, max: number) => (text.length > max ? `${text.slice(0, max - 1)}…` : text);

/** Lays the notes, marks and proposals out by group; the same data always lands the same way. */
function layout(data: Data, marks: Mark[], proposals: Proposal[]): { dots: Dot[]; groups: string[] } {
  const groups = data.map?.groups ?? [];
  const where = new Map<string, number>();
  groups.forEach((group, i) => {
    group.notes.forEach((path) => where.set(`n:${path}`, i));
    group.marks.forEach((id) => where.set(`m:${id}`, i));
    group.proposals.forEach((id) => where.set(`p:${id}`, i));
  });
  const loose = groups.length;
  const dots: Dot[] = [
    ...data.notes.map((note) => ({ key: `n:${note.path}`, kind: 'note' as const, title: note.title, note: note.path })),
    ...marks.map((mark) => ({ key: `m:${mark.id}`, kind: 'mark' as const, title: mark.title, mark })),
    ...proposals.map((proposal) => ({ key: `p:${proposal.id}`, kind: 'proposal' as const, title: proposal.title, proposal })),
  ].map((dot) => ({ ...dot, group: where.get(dot.key) ?? loose, x: 0, y: 0 }));
  const used = new Set(dots.map((dot) => dot.group));
  const order = [...used].sort((a, b) => a - b);
  const bodies: Body[] = dots.map((dot) => ({ seed: seed(dot.key), project: order.indexOf(dot.group), parent: null, radius: ROOM[dot.kind], old: null }));
  settle(order.length, bodies).forEach(([x, y], i) => Object.assign(dots[i], { x, y }));
  return { dots, groups: [...groups.map((group) => group.name), '아직 묶지 않음'] };
}

/** The circle around a group's dots. */
function hull(dots: Dot[]): { x: number; y: number; r: number } {
  const x = dots.reduce((sum, dot) => sum + dot.x, 0) / dots.length;
  const y = dots.reduce((sum, dot) => sum + dot.y, 0) / dots.length;
  const r = Math.max(...dots.map((dot) => Math.hypot(dot.x - x, dot.y - y) + RADIUS[dot.kind])) + 26;
  return { x, y, r };
}

/**
 * 학습 지도: the study notes and 학습함 marks as dots in their fields of knowledge, with links where
 * one builds on another. A note opens in the document lane; a mark or proposal shows its card.
 */
export function LearnMap({ conn, marks, proposals, onClose }: { conn: Connection; marks: Mark[]; proposals: Proposal[]; onClose: () => void }) {
  const [frame, setFrame] = useState(() =>
    fitted(loadFrame(FRAME_KEY, () => ({ x: (window.innerWidth - 860) / 2, y: (window.innerHeight - 610) / 2, w: 860, h: 610 })), MIN),
  );
  const [data, setData] = useState<Data | null>(null);
  const [camera, setCamera] = useState<Camera | null>(null);
  const [picked, setPicked] = useState<string | null>(null);
  const [hover, setHover] = useState<string | null>(null);
  const [size, setSize] = useState({ w: 1, h: 1 });
  const box = useRef<HTMLDivElement>(null);
  const drag = useRef<{ x: number; y: number; center: [number, number] } | null>(null);

  useEffect(() => {
    const off = conn.onMessage((message) => {
      if (message.t === 'learn_map') setData(message);
    });
    conn.send({ t: 'read_learn_map' });
    return off;
  }, [conn]);

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => setSize({ w: entry.contentRect.width || 1, h: entry.contentRect.height || 1 }));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const { dots, groups } = useMemo(() => (data ? layout(data, marks, proposals) : { dots: [], groups: [] }), [data, marks, proposals]);
  const byKey = useMemo(() => new Map(dots.map((dot) => [dot.key, dot])), [dots]);
  const hulls = useMemo(
    () =>
      groups.flatMap((name, i) => {
        const members = dots.filter((dot) => dot.group === i);
        return members.length ? [{ name, loose: i === groups.length - 1, ...hull(members) }] : [];
      }),
    [dots, groups],
  );
  const links = useMemo(
    () =>
      (data?.map?.links ?? []).flatMap((link) => {
        const a = byKey.get(link.a);
        const b = byKey.get(link.b);
        return a && b ? [{ a, b, why: link.why }] : [];
      }),
    [data, byKey],
  );
  const neighbors = useMemo(() => {
    const found = new Map<string, Set<string>>();
    for (const { a, b } of links) {
      found.set(a.key, (found.get(a.key) ?? new Set()).add(b.key));
      found.set(b.key, (found.get(b.key) ?? new Set()).add(a.key));
    }
    return found;
  }, [links]);

  const fit = useMemo((): Camera => {
    if (hulls.length === 0) return { zoom: 1, center: [0, 0] };
    const left = Math.min(...hulls.map((h) => h.x - h.r));
    const right = Math.max(...hulls.map((h) => h.x + h.r));
    const top = Math.min(...hulls.map((h) => h.y - h.r - 18));
    const bottom = Math.max(...hulls.map((h) => h.y + h.r));
    const zoom = Math.min(size.w / (right - left + 40), size.h / (bottom - top + 40), 1.4);
    return { zoom, center: [(left + right) / 2, (top + bottom) / 2] };
  }, [hulls, size]);
  const view = camera ?? fit;
  const viewBox = [view.center[0] - size.w / 2 / view.zoom, view.center[1] - size.h / 2 / view.zoom, size.w / view.zoom, size.h / view.zoom].join(' ');
  const zoomBy = (factor: number) => setCamera({ ...view, zoom: Math.min(Math.max(view.zoom * factor, ZOOM.min), ZOOM.max) });

  // React's wheel listener is passive, so the page would scroll instead of the map.
  const wheel = useRef({ view, zoomBy });
  wheel.current = { view, zoomBy };
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      const { view, zoomBy } = wheel.current;
      if (event.ctrlKey || event.metaKey) return zoomBy(Math.exp(-event.deltaY * 0.01));
      setCamera({ zoom: view.zoom, center: [view.center[0] + event.deltaX / view.zoom, view.center[1] + event.deltaY / view.zoom] });
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => el.removeEventListener('wheel', onWheel);
  }, []);

  const pick = (dot: Dot) => {
    if (dot.kind === 'note' && dot.note) {
      setPicked(null);
      return conn.send({ t: 'learn_open_study_note', path: dot.note });
    }
    setPicked(dot.key);
  };
  const near = (dot: Dot) => hover !== null && (hover === dot.key || !!neighbors.get(hover)?.has(dot.key));
  const selected = picked ? byKey.get(picked) : undefined;
  const status = !data
    ? null
    : data.offline
      ? { text: '학습 노트 폴더를 읽지 못해요. 설정 → 경로와 디스크 연결을 확인해 주세요', error: false }
      : data.map?.error
        ? { text: `묶지 못했어요 · ${data.map.error}`, error: true }
        : !data.map
          ? { text: '아직 묶지 않았어요', error: false }
          : null;
  const builtOn = data?.map?.built_on.match(/^\d{4}-(\d{2})-(\d{2})$/);

  return (
    <section
      tabIndex={-1}
      aria-label="학습 지도"
      onKeyDown={(event) => event.key === 'Escape' && !event.nativeEvent.isComposing && onClose()}
      className="fixed z-30 flex flex-col overflow-hidden rounded-2xl bg-background shadow-[0_12px_40px_rgba(9,9,11,0.18)] outline-none animate-in fade-in zoom-in-95"
      style={{ left: frame.x, top: frame.y, width: frame.w, height: frame.h }}
    >
      <header
        onPointerDown={(event) => grabFrame(event, frame, false, MIN, FRAME_KEY, setFrame)}
        className="flex h-[46px] shrink-0 cursor-grab items-center gap-2 pr-2.5 pl-[18px] select-none active:cursor-grabbing"
      >
        <b className="text-[14px] font-bold">학습 지도</b>
        {data && (
          <span className="min-w-0 truncate text-[12px] text-muted-foreground">
            {[
              `학습 노트 ${data.notes.length}개`,
              `학습함 ${marks.length}개`,
              proposals.length ? `담을까요 ${proposals.length}개` : '',
              builtOn ? `${Number(builtOn[1])}월 ${Number(builtOn[2])}일 묶음` : '',
            ]
              .filter(Boolean)
              .join(' · ')}
          </span>
        )}
        {status && <span className={cn('min-w-0 truncate text-[12px] font-semibold', status.error ? 'text-destructive' : 'text-foreground')}>{status.text}</span>}
        <div className="ml-auto flex gap-0.5">
          <Button variant="ghost" size="icon-xs" aria-label="축소" onClick={() => zoomBy(0.8)}>
            <Minus />
          </Button>
          <Button variant="ghost" size="icon-xs" aria-label="확대" onClick={() => zoomBy(1.25)}>
            <Plus />
          </Button>
          <Button variant="ghost" size="icon-xs" aria-label="화면 맞춤" onClick={() => setCamera(null)}>
            <Scan />
          </Button>
          <Button variant="ghost" size="icon-xs" aria-label="닫기" onClick={onClose}>
            <X />
          </Button>
        </div>
      </header>
      <div
        ref={box}
        className="relative mx-2.5 mb-2.5 min-h-0 flex-1 cursor-grab touch-none overflow-hidden rounded-xl select-none active:cursor-grabbing"
        style={{ background: PAPER }}
        onPointerDown={(event) => {
          if ((event.target as Element).closest('[data-dot]')) return;
          drag.current = { x: event.clientX, y: event.clientY, center: view.center };
          (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
        }}
        onPointerMove={(event) => {
          const start = drag.current;
          if (!start) return;
          setCamera({ zoom: view.zoom, center: [start.center[0] - (event.clientX - start.x) / view.zoom, start.center[1] - (event.clientY - start.y) / view.zoom] });
        }}
        onPointerUp={(event) => {
          const start = drag.current;
          drag.current = null;
          if (start && Math.hypot(event.clientX - start.x, event.clientY - start.y) < 4) setPicked(null);
        }}
      >
        {data && dots.length === 0 && <p className="absolute inset-x-0 top-1/2 text-center text-zinc-500">아직 학습 노트와 표시가 없어요</p>}
        <svg viewBox={viewBox} className="absolute inset-0 size-full" style={{ fontFamily: FONT }}>
          <style>{'@keyframes naru-drift{0%,100%{transform:translate(0,0)}50%{transform:translate(0,-3px)}}.naru-drift{animation:naru-drift 5.5s ease-in-out infinite}'}</style>
          {hulls.map((h) => (
            <g key={h.name}>
              <circle cx={h.x} cy={h.y} r={h.r} fill={h.loose ? 'none' : GROUND} stroke={h.loose ? '#d4d4d8' : 'none'} strokeDasharray={h.loose ? '4 4' : undefined} />
              <text x={h.x} y={h.y - h.r - 6} textAnchor="middle" fontSize={12} fontWeight={600} fill={h.loose ? MUTED : '#52525b'}>
                {h.loose ? `${h.name} · 다음 묶을 때 자리를 찾아요` : h.name}
              </text>
            </g>
          ))}
          {links.map(({ a, b, why }) => {
            const lit = hover === a.key || hover === b.key;
            return (
              <g key={`${a.key}-${b.key}`} opacity={hover && !lit ? 0.25 : 0.7}>
                <line x1={a.x} y1={a.y} x2={b.x} y2={b.y} stroke={INK} strokeWidth={1.2} strokeDasharray="4 3" />
                {why && (lit || picked === a.key || picked === b.key) && (
                  <text x={(a.x + b.x) / 2} y={(a.y + b.y) / 2 - 4} textAnchor="middle" fontSize={10.5} fill="#52525b">
                    {why}
                  </text>
                )}
              </g>
            );
          })}
          {dots.map((dot, i) => {
            const label = dot.kind !== 'note' || near(dot) || picked === dot.key;
            const r = RADIUS[dot.kind];
            return (
              <g
                key={dot.key}
                data-dot
                className="naru-drift cursor-pointer"
                style={{ animationDelay: `${-i * 0.37}s` }}
                onPointerEnter={() => setHover(dot.key)}
                onPointerLeave={() => setHover((at) => (at === dot.key ? null : at))}
                onClick={() => pick(dot)}
              >
                <title>{dot.title}</title>
                {dot.kind === 'mark' && <circle cx={dot.x} cy={dot.y} r={r + 5} fill="none" stroke={INK} strokeWidth={1.5} />}
                <circle
                  cx={dot.x}
                  cy={dot.y}
                  r={r}
                  fill={dot.kind === 'note' ? MUTED : dot.kind === 'mark' ? INK : '#ffffff'}
                  stroke={dot.kind === 'proposal' ? INK : picked === dot.key ? INK : 'none'}
                  strokeWidth={dot.kind === 'proposal' ? 2 : 1.5}
                  strokeDasharray={dot.kind === 'proposal' ? '3 2' : undefined}
                />
                {label && (
                  <text x={dot.x} y={dot.y + r + (dot.kind === 'mark' ? 18 : 14)} textAnchor="middle" fontSize={11.5} fontWeight={dot.kind === 'mark' ? 600 : 400} fill={dot.kind === 'note' ? '#3f3f46' : INK}>
                    {clip(dot.title, 22)}
                  </text>
                )}
              </g>
            );
          })}
        </svg>
        {selected && selected.kind !== 'note' && <ItemCard conn={conn} dot={selected} links={links.filter((link) => link.a === selected || link.b === selected)} />}
      </div>
      <div onPointerDown={(event) => grabFrame(event, frame, true, MIN, FRAME_KEY, setFrame)} className="absolute right-0 bottom-0 size-4 cursor-nwse-resize" aria-hidden />
    </section>
  );
}

function ItemCard({ conn, dot, links }: { conn: Connection; dot: Dot; links: { a: Dot; b: Dot; why: string }[] }) {
  const others = links.map((link) => ({ other: link.a === dot ? link.b : link.a, why: link.why }));
  const linked = others.find(({ other }) => other.kind === 'note')?.other.note;
  const { mark, proposal } = dot;
  const words = proposal ? `“${proposal.quote}”` : mark?.hard !== mark?.title ? mark?.hard : '';
  return (
    <div className="absolute right-3 bottom-3 flex w-[300px] flex-col gap-1.5 rounded-xl bg-background p-3.5 shadow-[0_6px_24px_rgba(9,9,11,0.14)]" onPointerDown={(event) => event.stopPropagation()}>
      <span className="text-[11.5px] text-muted-foreground">{mark ? `학습함 · ${stageLabel[mark.stage]}` : '담을까요?'}</span>
      <b className="font-semibold">{dot.title}</b>
      {words && <p className="line-clamp-3 text-[12.5px] [overflow-wrap:anywhere]">{words}</p>}
      {others.length > 0 && <p className="text-[11.5px] text-muted-foreground">{others.map(({ other, why }) => `${other.title} · ${why}`).join(' / ')}</p>}
      <div className="mt-0.5 flex flex-wrap gap-1.5">
        {mark && (
          <>
            <Button size="xs" onClick={() => conn.send({ t: 'learn_study', id: mark.id })}>
              학습 세션 열기
            </Button>
            {linked && (
              <Button size="xs" variant="secondary" onClick={() => conn.send({ t: 'learn_open_study_note', path: linked })}>
                이어진 노트 열기
              </Button>
            )}
          </>
        )}
        {proposal && <ProposalActions conn={conn} id={proposal.id} />}
      </div>
    </div>
  );
}
