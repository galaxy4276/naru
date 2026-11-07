import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { Minus, Plus, Scan, X } from 'lucide-react';
import type { Connection, Letter, PaneId, Workspace } from '@/connection';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { agentColor, drawAgentMark } from './agentIcons';
import { fitted, grabFrame, loadFrame } from './frame';
import { LetterThread } from './LetterThread';
import { type End, ME as USER, type Pair, pairsOf, threadOf } from './letters';
import { getStored, setStored } from './stored';
import type { Teams } from './model';
import { type PairHit, drawHint, drawLetters } from './overviewLetters';
import { CHILD_ROOM, FILTERS, type Filter, type Graph, type Mover, type Node, ROOT_ROOM, graphOf, matches, seed, settle, step, visible } from './overviewGraph';
import { Ring } from './status';

const FRAME_KEY = 'naru.overview.frame';
const PLACES_KEY = 'naru.overview.places';
const MIN = { w: 560, h: 380 };
const ZOOM = { min: 0.3, max: 2.5 };
/** Names show from this zoom on, and always for the hovered session and its neighbours. */
const LABEL_ZOOM = 1.25;
const ME = 22;
const ROOT_R = 14;
const CHILD_R = 10;
// The map stays dark in every theme: its glow only reads on a dark ground.
const MAP = '#141416';
const INK = '#e4e4e7';
const MUTED = '#71717a';
/** Session titles. */
const LABEL = '#ffffff';
const TURN = '#f97316';
const ERR = '#ef4444';
const WORK = '#facc15';
const DONE = '#4ade80';
const HUES = ['96,165,250', '167,139,250', '74,222,128', '251,191,36', '244,114,182', '45,212,191'];
const FONT = 'Pretendard, -apple-system, sans-serif';
const PALETTE = { ink: INK, map: MAP, err: ERR, font: FONT };
const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');

/** World positions by pane, kept while the app runs and across restarts. */
const places = new Map<PaneId, [number, number]>();
let placesLoaded = false;

// Read on first open rather than at load, so the daemon's copy has arrived.
function loadPlaces() {
  if (placesLoaded) return;
  placesLoaded = true;
  try {
    for (const [pane, at] of Object.entries(JSON.parse(getStored(PLACES_KEY) ?? '{}') as Record<string, [number, number]>)) places.set(pane, at);
  } catch {
    // Nothing kept yet, or a broken copy: the map seeds fresh places.
  }
}

function savePlaces(open: Set<PaneId>) {
  // Places of closed panes go, so the kept map does not grow forever.
  for (const pane of places.keys()) if (!open.has(pane)) places.delete(pane);
  setStored(PLACES_KEY, JSON.stringify(Object.fromEntries(places)));
}

const clip = (text: string, max: number) => (text.length > max ? `${text.slice(0, max - 1)}…` : text);

/** A session and everything started under it, folded ones included. */
function teamOf(graph: Graph, pane: PaneId): Node[] {
  return graph.all.filter((node) => {
    for (let at: Node | undefined = node, hops = 0; at && hops <= graph.all.length; hops++) {
      if (at.pane === pane) return true;
      at = at.parent ? graph.all.find((other) => other.pane === at!.parent) : undefined;
    }
    return false;
  });
}

type Camera = { zoom: number; center: [number, number] };
type Grab = { pane: PaneId | null; x: number; y: number; startX: number; startY: number; moved: boolean; homes: [PaneId, [number, number]][] };
type Hit =
  | { kind: 'node' | 'fold' | 'unfold'; pane: PaneId; x: number; y: number; r: number }
  | { kind: 'project'; name: string; x: number; y: number; w: number; h: number }
  | PairHit;

interface Scene {
  graph: Graph;
  shown: Node[];
  movers: Map<PaneId, Mover>;
  view: Camera;
  w: number;
  h: number;
  filter: Filter;
  hover: PaneId | null;
  foldedProjects: Set<string>;
  t: number;
  pairs: Pair[];
  /** The pair whose letters are open, and the one under the pointer. */
  picked: string | null;
  hoverPair: string | null;
  nameOf: (end: End) => string;
}

/** Paints the map and returns what can be clicked, topmost last. */
function draw(ctx: CanvasRenderingContext2D, s: Scene): Hit[] {
  const { view, w, h, t } = s;
  // Orbs and names keep a readable size while positions follow the zoom.
  const k = Math.min(Math.max(view.zoom, 0.75), 1.4);
  const at = ([x, y]: [number, number]): [number, number] => [(x - view.center[0]) * view.zoom + w / 2, (y - view.center[1]) * view.zoom + h / 2];
  const me = at([0, 0]);
  const pos = new Map(s.shown.map((node) => [node.pane, at([s.movers.get(node.pane)!.x, s.movers.get(node.pane)!.y])]));
  const radius = (node: Node) => (node.parent ? CHILD_R : ROOT_R) * k;
  const focus = s.hover ? s.shown.find((node) => node.pane === s.hover) : undefined;
  const lit = focus
    ? new Set([focus.pane, ...(focus.parent ? [focus.parent] : []), ...s.shown.filter((node) => node.parent === focus.pane).map((node) => node.pane)])
    : null;
  const filtered = (node: Node) => (matches(s.filter, node.tone) ? 1 : 0.25);
  const faded = (node: Node) => filtered(node) * (lit && !lit.has(node.pane) ? 0.15 : 1);
  const hits: Hit[] = [];
  const circle = (x: number, y: number, r: number) => {
    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
  };

  ctx.fillStyle = MAP;
  ctx.fillRect(0, 0, w, h);

  s.graph.projects.forEach((project, index) => {
    const members = s.shown.filter((node) => node.project === index);
    if (!members.length) return;
    const points = members.map((node) => pos.get(node.pane)!);
    const cx = points.reduce((sum, [x]) => sum + x, 0) / points.length;
    const cy = points.reduce((sum, [, y]) => sum + y, 0) / points.length;
    const r = Math.max(...members.map((node, i) => Math.hypot(points[i][0] - cx, points[i][1] - cy) + radius(node))) + 44 * k;
    const hue = HUES[index % HUES.length];
    const glow = ctx.createRadialGradient(cx, cy, 0, cx, cy, r);
    glow.addColorStop(0, `rgba(${hue},.13)`);
    glow.addColorStop(0.7, `rgba(${hue},.13)`);
    glow.addColorStop(1, `rgba(${hue},0)`);
    ctx.fillStyle = glow;
    circle(cx, cy, r);
    ctx.fill();
    const label = s.foldedProjects.has(project.name) ? `${project.name} · 접힘` : project.name;
    ctx.font = `700 12.5px ${FONT}`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'bottom';
    ctx.globalAlpha = lit ? 0.4 : 1;
    ctx.fillStyle = MUTED;
    // A big glow reaches past the map's top edge; its name then sticks to the edge.
    const top = Math.min(Math.max(cy - r + 28 * k, 24), cy);
    ctx.fillText(label, cx, top);
    ctx.globalAlpha = 1;
    const width = ctx.measureText(label).width + 10;
    hits.push({ kind: 'project', name: project.name, x: cx - width / 2, y: top - 17, w: width, h: 19 });
  });

  ctx.lineCap = 'round';
  for (const node of s.shown) {
    const [x, y] = pos.get(node.pane)!;
    const parent = node.parent ? pos.get(node.parent) : undefined;
    const asks = node.tone === 'turn' || node.tone === 'err';
    const touches = !lit || node.pane === focus?.pane || node.parent === focus?.pane;
    if (parent || !asks) {
      const [x0, y0] = parent ?? me;
      ctx.globalAlpha = filtered(node) * (touches ? 1 : 0.1);
      ctx.strokeStyle = 'rgba(255,255,255,.16)';
      ctx.lineWidth = lit && touches ? 1.8 : 1.2;
      ctx.shadowColor = 'rgba(255,255,255,.35)';
      ctx.shadowBlur = 6;
      ctx.beginPath();
      ctx.moveTo(x0, y0);
      ctx.lineTo(x, y);
      ctx.stroke();
      ctx.shadowBlur = 0;
    }
    if (!asks) continue;
    // Whatever waits on the user gets a moving arrow from "나".
    const dx = x - me[0];
    const dy = y - me[1];
    const d = Math.hypot(dx, dy) || 1;
    const [ux, uy] = [dx / d, dy / d];
    const end = radius(node) + 7 * k;
    const [x1, y1] = [x - ux * end, y - uy * end];
    const color = node.tone === 'turn' ? TURN : ERR;
    ctx.globalAlpha = faded(node);
    ctx.strokeStyle = color;
    ctx.fillStyle = color;
    ctx.lineWidth = 1.7;
    ctx.setLineDash([5, 3]);
    ctx.lineDashOffset = -t / 40;
    ctx.beginPath();
    ctx.moveTo(me[0] + ux * (ME * k + 4), me[1] + uy * (ME * k + 4));
    ctx.lineTo(x1, y1);
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.beginPath();
    ctx.moveTo(x1, y1);
    ctx.lineTo(x1 - ux * 8 - uy * 4.5, y1 - uy * 8 + ux * 4.5);
    ctx.lineTo(x1 - ux * 8 + uy * 4.5, y1 - uy * 8 - ux * 4.5);
    ctx.closePath();
    ctx.fill();
  }
  ctx.globalAlpha = 1;

  const endRadius = (end: End) => {
    const node = end === USER ? undefined : s.shown.find((each) => each.pane === end);
    return node ? radius(node) : ME * k;
  };
  const pairLit = (pair: Pair) => pair.key === s.picked || pair.key === s.hoverPair || (!!focus && (pair.a === focus.pane || pair.b === focus.pane));
  const letterScene = {
    pairs: s.pairs,
    at: (end: End) => (end === USER ? me : pos.get(end)),
    radius: endRadius,
    lit: pairLit,
    now: Date.now() / 1000,
    t,
    still: reducedMotion.matches,
    k,
  };
  hits.push(...drawLetters(ctx, letterScene, PALETTE));

  ctx.fillStyle = INK;
  circle(me[0], me[1], ME * k);
  ctx.fill();
  ctx.fillStyle = MAP;
  ctx.font = `700 ${14 * k}px ${FONT}`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText('나', me[0], me[1] + 1);

  for (const node of s.shown) {
    const [x, y] = pos.get(node.pane)!;
    const r = radius(node);
    const base = faded(node) * (node.tone === 'wait' ? 0.55 : 1);
    const brand = agentColor(node.agent);
    const fill = brand ?? (node.agent ? INK : '#3f3f46');
    ctx.globalAlpha = base;
    ctx.shadowColor = fill;
    ctx.shadowBlur = 18 * k;
    ctx.fillStyle = fill;
    circle(x, y, r);
    ctx.fill();
    ctx.shadowBlur = 0;
    drawAgentMark(ctx, node.agent, x, y, r * 1.05, brand || !node.agent ? '#fff' : MAP);

    const ring = r + 4 * k;
    ctx.lineWidth = 2;
    if (node.tone === 'work') {
      // Indeterminate progress: the arc runs round its faint track while it grows and shrinks.
      ctx.lineWidth = 2.5 * k;
      ctx.strokeStyle = WORK;
      ctx.globalAlpha = base * 0.25;
      circle(x, y, ring);
      ctx.stroke();
      ctx.globalAlpha = base;
      const from = t / 260;
      ctx.beginPath();
      ctx.arc(x, y, ring, from, from + 0.7 + 1.8 * (1 + Math.sin(t / 420)));
      ctx.stroke();
    } else if (node.tone === 'done') {
      ctx.lineWidth = 2.5 * k;
      ctx.strokeStyle = DONE;
      ctx.shadowColor = DONE;
      ctx.shadowBlur = 10 * k;
      circle(x, y, ring);
      ctx.stroke();
      ctx.shadowBlur = 0;
    } else if (node.tone === 'turn' || node.tone === 'err') {
      const pulse = (t / 1100) % 1;
      ctx.strokeStyle = node.tone === 'turn' ? TURN : ERR;
      circle(x, y, ring);
      ctx.stroke();
      ctx.globalAlpha = base * (1 - pulse);
      circle(x, y, ring + pulse * 10 * k);
      ctx.stroke();
      ctx.globalAlpha = base;
    }
    hits.push({ kind: 'node', pane: node.pane, x, y, r: r + 5 });

    const badge = (text: string, bx: number, by: number) => {
      ctx.font = `700 ${10 * k}px ${FONT}`;
      const bw = Math.max(ctx.measureText(text).width + 8 * k, 14 * k);
      ctx.fillStyle = INK;
      ctx.beginPath();
      ctx.roundRect(bx - bw / 2, by - 7 * k, bw, 14 * k, 7 * k);
      ctx.fill();
      ctx.fillStyle = MAP;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText(text, bx, by + 0.5);
      return Math.max(bw / 2, 7 * k) + 2;
    };
    if (node.hidden > 0) {
      const [bx, by] = [x + r * 0.95, y + r * 0.75];
      hits.push({ kind: 'unfold', pane: node.pane, x: bx, y: by, r: badge(`+${node.hidden}`, bx, by) });
    } else if (node.children && node.pane === s.hover) {
      const [bx, by] = [x - r * 0.9, y - r * 0.9];
      hits.push({ kind: 'fold', pane: node.pane, x: bx, y: by, r: badge('−', bx, by) });
    }
    ctx.globalAlpha = 1;
  }

  for (const node of s.shown) {
    const near = lit?.has(node.pane) ?? false;
    // Zoomed out, every session still says what it is, in small white type.
    const full = near || view.zoom >= LABEL_ZOOM;
    const [x, y] = pos.get(node.pane)!;
    const below = y + radius(node) + (full ? 7 : 4) * k;
    ctx.globalAlpha = near ? 1 : faded(node);
    ctx.textAlign = 'center';
    ctx.textBaseline = 'top';
    const size = full ? (node.parent ? 10.5 : 11.5) * k : 9;
    ctx.font = `${node.pane === focus?.pane ? 700 : 600} ${size}px ${FONT}`;
    ctx.fillStyle = LABEL;
    ctx.fillText(clip(node.title, full ? 22 : 14), x, below);
    if (full && node.ask) {
      ctx.font = `600 ${10.5 * k}px ${FONT}`;
      ctx.fillStyle = node.tone === 'err' ? ERR : TURN;
      ctx.fillText(clip(node.ask, 30), x, below + 15 * k);
    }
  }
  ctx.globalAlpha = 1;
  if (focus) {
    const lines = s.pairs.flatMap((pair) => {
      if (pair.a !== focus.pane && pair.b !== focus.pane) return [];
      const [a, b] = [s.nameOf(pair.a), s.nameOf(pair.b)];
      return [...(pair.ab ? [`${a} → ${b} ${pair.ab}통`] : []), ...(pair.ba ? [`${b} → ${a} ${pair.ba}통`] : [])];
    });
    const [x, y] = pos.get(focus.pane)!;
    const shown = lines.length > 4 ? [...lines.slice(0, 3), `그 밖에 ${lines.length - 3}줄`] : lines;
    drawHint(ctx, [x, y - radius(focus) - 10 * k], shown, k, PALETTE);
  }
  return hits;
}

/**
 * 한눈에 보기: every agent session around "나" on a dark map that keeps moving, one tinted glow
 * per project, children tied to their parent. Orbs drag with their team and spring back into
 * place, the canvas pans and zooms, and the map keeps its places.
 */
export function Overview({ conn, ws, teams, onClose, onPicked }: { conn: Connection; ws: Workspace; teams: Teams; onClose: () => void; onPicked?: () => void }) {
  const [frame, setFrame] = useState(() =>
    fitted(loadFrame(FRAME_KEY, () => ({ x: (window.innerWidth - 1120) / 2, y: (window.innerHeight - 720) / 2, w: 1120, h: 720 })), MIN),
  );
  const [filter, setFilter] = useState<Filter>('all');
  const [camera, setCamera] = useState<Camera | null>(null);
  const [folded, setFolded] = useState<Set<PaneId>>(new Set());
  const [foldedProjects, setFoldedProjects] = useState<Set<string>>(new Set());
  const [size, setSize] = useState({ w: 0, h: 0 });
  const map = useRef<HTMLDivElement>(null);
  const paint = useRef<HTMLCanvasElement>(null);
  const root = useRef<HTMLElement>(null);
  const grab = useRef<Grab | null>(null);
  const hover = useRef<PaneId | null>(null);
  const hoverPair = useRef<string | null>(null);
  const [letters, setLetters] = useState<Letter[]>([]);
  const [picked, setPicked] = useState<string | null>(null);
  const hits = useRef<Hit[]>([]);
  const movers = useRef(new Map<PaneId, Mover>());
  const heat = useRef(0.6);

  loadPlaces();
  const graph = useMemo(() => graphOf(ws, teams), [ws, teams]);
  // New sessions settle around the ones already placed, which hold still.
  useMemo(() => {
    if (graph.all.every((node) => places.has(node.pane))) return;
    const index = new Map(graph.all.map((node, i) => [node.pane, i]));
    const bodies = graph.all.map((node) => ({
      seed: seed(node.pane),
      project: node.project,
      parent: node.parent ? index.get(node.parent)! : null,
      radius: node.parent ? CHILD_ROOM : ROOT_ROOM,
      old: places.get(node.pane) ?? null,
    }));
    settle(graph.projects.length, bodies).forEach((at, i) => places.set(graph.all[i].pane, at));
  }, [graph]);
  const shown = visible(graph, folded, foldedProjects);

  useLayoutEffect(() => {
    const el = map.current;
    if (!el) return;
    const observer = new ResizeObserver(() => setSize({ w: el.clientWidth, h: el.clientHeight }));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  useEffect(() => root.current?.focus(), []);
  useEffect(() => {
    const off = conn.onMessage((message) => {
      if (message.t === 'letters') setLetters(message.letters);
    });
    conn.send({ t: 'read_letters' });
    return off;
  }, [conn]);
  useEffect(() => {
    const refit = () => setFrame((now) => fitted(now, MIN));
    window.addEventListener('resize', refit);
    return () => window.removeEventListener('resize', refit);
  }, []);

  // Fit every shown session and "나" until the user zooms or pans.
  const view: Camera = (() => {
    if (camera) return camera;
    let [x0, y0, x1, y1] = [-ME * 2, -ME * 2, ME * 2, ME * 2];
    for (const node of shown) {
      const [x, y] = places.get(node.pane)!;
      x0 = Math.min(x0, x - 80);
      x1 = Math.max(x1, x + 80);
      y0 = Math.min(y0, y - 70);
      y1 = Math.max(y1, y + 80);
    }
    const zoom = size.w ? Math.min(Math.max(Math.min(size.w / (x1 - x0), size.h / (y1 - y0)), 0.45), 1) : 1;
    return { zoom, center: [(x0 + x1) / 2, (y0 + y1) / 2] };
  })();

  const pairs = pairsOf(letters, Date.now() / 1000);
  const pair = pairs.find((each) => each.key === picked);
  const nameOf = (end: End) => (end === USER ? '나' : (teams[end]?.id ?? ''));
  const titleOf = (end: End) => (end === USER ? '' : (graph.all.find((node) => node.pane === end)?.title ?? ''));

  // The frame loop reads the latest render through this.
  const live = useRef({ graph, shown, view, filter, size, foldedProjects, pairs, picked, nameOf });
  live.current = { graph, shown, view, filter, size, foldedProjects, pairs, picked, nameOf };

  useEffect(() => {
    let frameId = 0;
    let alpha = 0;
    const loop = (t: number) => {
      frameId = requestAnimationFrame(loop);
      const canvas = paint.current;
      const ctx = canvas?.getContext('2d');
      const { graph, shown, view, filter, size, foldedProjects, pairs, picked, nameOf } = live.current;
      if (!canvas || !ctx || !size.w) return;
      const dpr = window.devicePixelRatio || 1;
      if (canvas.width !== Math.round(size.w * dpr) || canvas.height !== Math.round(size.h * dpr)) {
        canvas.width = Math.round(size.w * dpr);
        canvas.height = Math.round(size.h * dpr);
      }
      const list: Mover[] = [];
      const index = new Map<PaneId, number>();
      for (const node of shown) {
        const home = places.get(node.pane)!;
        let mover = movers.current.get(node.pane);
        if (!mover) {
          mover = { x: home[0], y: home[1], vx: 0, vy: 0, r: 0, home, held: false };
          movers.current.set(node.pane, mover);
        }
        mover.home = home;
        mover.r = node.parent ? CHILD_R : ROOT_R;
        index.set(node.pane, list.push(mover) - 1);
      }
      for (const pane of movers.current.keys()) if (!index.has(pane)) movers.current.delete(pane);
      const ties = shown.flatMap((node): [number, number][] =>
        node.parent && index.has(node.parent) ? [[index.get(node.parent)!, index.get(node.pane)!]] : [],
      );
      alpha = Math.max(alpha * 0.98, heat.current, grab.current?.moved && grab.current.pane ? 0.3 : 0);
      heat.current = 0;
      step(list, ties, alpha);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      hits.current = draw(ctx, {
        graph,
        shown,
        movers: movers.current,
        view,
        w: size.w,
        h: size.h,
        filter,
        hover: hover.current,
        foldedProjects,
        t,
        pairs,
        picked,
        hoverPair: hoverPair.current,
        nameOf,
      });
    };
    frameId = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(frameId);
  }, []);

  const hitAt = (event: React.PointerEvent) => {
    const rect = paint.current!.getBoundingClientRect();
    const [px, py] = [event.clientX - rect.left, event.clientY - rect.top];
    return [...hits.current]
      .reverse()
      .find((hit) =>
        hit.kind === 'project'
          ? px >= hit.x && px <= hit.x + hit.w && py >= hit.y && py <= hit.y + hit.h
          : Math.hypot(px - hit.x, py - hit.y) <= hit.r || (hit.kind === 'pair' && hit.points.some(([x, y]) => Math.hypot(px - x, py - y) <= 7)),
      );
  };
  const toggle = <T,>(set: Set<T>, key: T) => {
    const next = new Set(set);
    if (!next.delete(key)) next.add(key);
    return next;
  };
  const focus = (pane: PaneId) => {
    conn.send({ t: 'focus', pane });
    onPicked?.();
    onClose();
  };

  const onPointerDown = (event: React.PointerEvent) => {
    const hit = hitAt(event);
    if (hit?.kind === 'pair') return setPicked(hit.key);
    if (hit?.kind === 'project') return setFoldedProjects((now) => toggle(now, hit.name));
    if (hit?.kind === 'fold') return setFolded((now) => toggle(now, hit.pane));
    if (hit?.kind === 'unfold') {
      const node = graph.all.find((other) => other.pane === hit.pane);
      setFolded((now) => {
        const next = new Set(now);
        next.delete(hit.pane);
        return next;
      });
      setFoldedProjects((now) => {
        const next = new Set(now);
        if (node) next.delete(graph.projects[node.project].name);
        return next;
      });
      return;
    }
    const pane = hit?.kind === 'node' ? hit.pane : null;
    const homes = pane ? teamOf(graph, pane).map((node): [PaneId, [number, number]] => [node.pane, places.get(node.pane)!]) : [];
    grab.current = { pane, x: event.clientX, y: event.clientY, startX: event.clientX, startY: event.clientY, moved: false, homes };
    paint.current?.setPointerCapture(event.pointerId);
  };
  const onPointerMove = (event: React.PointerEvent) => {
    const held = grab.current;
    if (!held) {
      const hit = hitAt(event);
      hover.current = hit?.kind === 'node' || hit?.kind === 'fold' || hit?.kind === 'unfold' ? hit.pane : null;
      hoverPair.current = hit?.kind === 'pair' ? hit.key : null;
      if (paint.current) paint.current.style.cursor = hit ? 'pointer' : 'grab';
      return;
    }
    if (!held.moved && Math.hypot(event.clientX - held.startX, event.clientY - held.startY) < 3) return;
    held.moved = true;
    if (held.pane) {
      const [dx, dy] = [(event.clientX - held.startX) / view.zoom, (event.clientY - held.startY) / view.zoom];
      for (const [pane, [x, y]] of held.homes) places.set(pane, [x + dx, y + dy]);
      const mover = movers.current.get(held.pane);
      const home = places.get(held.pane);
      if (mover && home) Object.assign(mover, { x: home[0], y: home[1], held: true });
      hover.current = held.pane;
      return;
    }
    const [dx, dy] = [event.clientX - held.x, event.clientY - held.y];
    held.x = event.clientX;
    held.y = event.clientY;
    setCamera({ zoom: view.zoom, center: [view.center[0] - dx / view.zoom, view.center[1] - dy / view.zoom] });
  };
  const onPointerUp = () => {
    const held = grab.current;
    grab.current = null;
    if (!held?.pane) return;
    const mover = movers.current.get(held.pane);
    if (mover) mover.held = false;
    if (!held.moved) return focus(held.pane);
    heat.current = 0.4;
    savePlaces(new Set(graph.all.map((node) => node.pane)));
  };
  const zoomBy = (factor: number) => setCamera({ ...view, zoom: Math.min(Math.max(view.zoom * factor, ZOOM.min), ZOOM.max) });

  // React's wheel listener is passive, so the page would scroll instead of the map.
  const wheel = useRef({ view, zoomBy });
  wheel.current = { view, zoomBy };
  useEffect(() => {
    const el = map.current;
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

  const waiting = graph.all.filter((node) => node.tone === 'turn' || node.tone === 'err');

  return (
    <section
      ref={root}
      tabIndex={-1}
      aria-label="한눈에 보기"
      onKeyDown={(event) => {
        if (event.key !== 'Escape' || event.nativeEvent.isComposing) return;
        if (pair) setPicked(null);
        else onClose();
      }}
      className="fixed z-30 flex flex-col overflow-hidden rounded-2xl bg-background shadow-[0_12px_40px_rgba(9,9,11,0.18)] outline-none animate-in fade-in zoom-in-95"
      style={{ left: frame.x, top: frame.y, width: frame.w, height: frame.h }}
    >
      <header
        onPointerDown={(event) => grabFrame(event, frame, false, MIN, FRAME_KEY, setFrame)}
        className="flex h-[46px] shrink-0 cursor-grab items-center gap-2 pr-2.5 pl-[18px] select-none active:cursor-grabbing"
      >
        <b className="mr-2 text-[14px] font-bold">한눈에 보기</b>
        {FILTERS.map((item) => (
          <button
            key={item.key}
            onClick={() => setFilter(item.key)}
            className={cn('h-6 rounded-lg px-2.5 text-[12px]', filter === item.key ? 'bg-primary text-primary-foreground' : 'bg-muted text-muted-foreground')}
          >
            {item.label} {graph.all.filter((node) => matches(item.key, node.tone)).length}
          </button>
        ))}
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
      <div className="flex min-h-0 flex-1">
        <div ref={map} className="relative mb-2.5 ml-2.5 flex-1 overflow-hidden rounded-xl select-none" style={{ background: MAP }}>
          <canvas
            ref={paint}
            aria-label="세션 지도"
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerUp}
            onPointerLeave={() => {
              if (!grab.current) [hover.current, hoverPair.current] = [null, null];
            }}
            className="absolute inset-0 size-full cursor-grab touch-none"
          />
          {graph.all.length === 0 && <p className="pointer-events-none absolute inset-x-0 top-1/2 text-center text-zinc-500">아직 에이전트 세션이 없어요</p>}
        </div>
        {pair ? (
          <LetterThread
            key={pair.key}
            conn={conn}
            pair={pair}
            letters={threadOf(letters, pair.a, pair.b)}
            nameOf={nameOf}
            titleOf={titleOf}
            onClose={() => setPicked(null)}
          />
        ) : (
          <aside className="w-[260px] shrink-0 overflow-y-auto px-3.5 pb-3.5">
            <h4 className="mt-0.5 mb-1.5 text-[11.5px] font-semibold text-muted-foreground">프로젝트</h4>
            {graph.projects.map((project) => (
              <div key={project.name} className="flex items-center gap-2 rounded-[10px] px-2 py-1.5">
                <b className="min-w-0 flex-1 truncate font-semibold">{project.name}</b>
                {(['turn', 'work', 'done'] as const).map((tone) =>
                  project[tone] ? (
                    <span key={tone} className="inline-flex items-center gap-1 text-[11.5px] text-foreground/80">
                      <Ring tone={tone} className={tone === 'work' ? 'text-yellow-500' : tone === 'done' ? 'text-green-500' : undefined} />
                      {project[tone]}
                    </span>
                  ) : null,
                )}
              </div>
            ))}
            <h4 className="mt-3 mb-1.5 text-[11.5px] font-semibold text-muted-foreground">나를 기다리는 것 {waiting.length}</h4>
            {waiting.length === 0 && <p className="px-2 text-[12px] text-muted-foreground">기다리는 것이 없어요</p>}
            {waiting.map((node) => (
              <button key={node.pane} onClick={() => focus(node.pane)} className="flex w-full gap-2 rounded-[10px] px-2 py-1.5 text-left hover:bg-muted">
                <Ring tone={node.tone} className="mt-[5px]" />
                <span className="min-w-0">
                  <b className="block truncate font-semibold">{node.title}</b>
                  {node.ask && <span className={cn('block truncate text-[12px]', node.tone === 'err' ? 'text-destructive' : 'text-turn')}>{node.ask}</span>}
                </span>
              </button>
            ))}
          </aside>
        )}
      </div>
      <div onPointerDown={(event) => grabFrame(event, frame, true, MIN, FRAME_KEY, setFrame)} className="absolute right-0 bottom-0 size-4 cursor-nwse-resize" aria-hidden />
    </section>
  );
}
