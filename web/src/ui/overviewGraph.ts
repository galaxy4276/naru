// 한눈에 보기: which sessions the map shows and where. Placement ports `settle` from the warp fork's
// overview_graph.rs; `step` keeps the map moving around those places while it is open.
import type { PaneId, Workspace } from '@/connection';
import { type Teams, type Tone, askLine, panesOf, paneTitle, toneOf } from './model.ts';

export const ROOT_ROOM = 66;
export const CHILD_ROOM = 44;
const TICKS = 120;

export type Filter = 'all' | 'turn' | 'work' | 'done';
export const FILTERS: { key: Filter; label: string }[] = [
  { key: 'all', label: '전체' },
  { key: 'turn', label: '내 차례' },
  { key: 'work', label: '일하는 중' },
  { key: 'done', label: '안 읽음' },
];

export function matches(filter: Filter, tone: Tone): boolean {
  switch (filter) {
    case 'all':
      return true;
    case 'turn':
      return tone === 'turn' || tone === 'err';
    case 'work':
      return tone === 'work';
    case 'done':
      return tone === 'done';
  }
}

export interface Node {
  pane: PaneId;
  project: number;
  parent: PaneId | null;
  title: string;
  agent: string | null;
  tone: Tone;
  /** What it waits on the user for. */
  ask: string | null;
  /** Sessions folded into this one. */
  hidden: number;
  children: boolean;
}

export interface Graph {
  projects: { name: string; turn: number; work: number; done: number }[];
  /** Every session, folded ones included. */
  all: Node[];
}

/** Agent sessions and the panes of agent teams, by the tab group (project) they sit in. */
export function graphOf(ws: Workspace, teams: Teams): Graph {
  const projects: Graph['projects'] = [];
  const projectOf = new Map<number | null, number>();
  const parents = new Set(Object.values(ws.panes).map((record) => record.parent));
  const all: Node[] = [];
  for (const tab of ws.tabs) {
    for (const pane of panesOf(tab.layout)) {
      const record = ws.panes[pane];
      if (!record || (!record.session && !record.parent && !parents.has(pane))) continue;
      const group = ws.groups.find((g) => g.id === tab.group) ?? null;
      let project = projectOf.get(group?.id ?? null);
      if (project === undefined) {
        project = projects.push({ name: group?.name ?? 'Terminal', turn: 0, work: 0, done: 0 }) - 1;
        projectOf.set(group?.id ?? null, project);
      }
      const team = teams[pane];
      const tone = toneOf(record.session, team) ?? 'wait';
      all.push({
        pane,
        project,
        parent: record.parent,
        title: paneTitle(ws, pane),
        agent: record.session?.agent ?? null,
        tone,
        ask: team?.alert ?? (record.session && askLine(record.session)),
        hidden: 0,
        children: false,
      });
    }
  }
  const shown = new Set(all.map((node) => node.pane));
  for (const node of all) {
    if (node.parent && !shown.has(node.parent)) node.parent = null;
    if (node.parent) all.find((other) => other.pane === node.parent)!.children = true;
    const project = projects[node.project];
    if (node.tone === 'turn' || node.tone === 'err') project.turn++;
    else if (node.tone === 'work') project.work++;
    else if (node.tone === 'done') project.done++;
  }
  return { projects, all };
}

/**
 * The sessions to draw: a folded team shows as its root, a folded project as its first root.
 * A hidden session's ask rises to the one it folds into.
 */
export function visible(graph: Graph, folded: Set<PaneId>, foldedProjects: Set<string>): Node[] {
  const byPane = new Map(graph.all.map((node) => [node.pane, node]));
  const shownAs = (node: Node): Node => {
    let shown = node;
    let at = node;
    for (let step = 0; step < graph.all.length && at.parent; step++) {
      at = byPane.get(at.parent)!;
      if (folded.has(at.pane)) shown = at;
    }
    if (foldedProjects.has(graph.projects[shown.project].name)) {
      shown = graph.all.find((other) => other.project === shown.project && !other.parent) ?? shown;
    }
    return shown;
  };
  const out = new Map(graph.all.map((node) => [node.pane, { ...node }]));
  for (const node of graph.all) {
    const shown = shownAs(node);
    if (shown.pane === node.pane) continue;
    const into = out.get(shown.pane)!;
    into.hidden++;
    const asks = node.tone === 'turn' || node.tone === 'err';
    if (asks && into.tone !== 'turn' && into.tone !== 'err') {
      into.tone = node.tone;
      into.ask = node.ask && `${node.title}: ${node.ask}`;
    }
    out.delete(node.pane);
  }
  return [...out.values()];
}

/** FNV-1a, so a pane gets the same first spot every time. */
export function seed(text: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash;
}

export interface Body {
  seed: number;
  project: number;
  parent: number | null;
  radius: number;
  /** Where it already was; it stays there. */
  old: [number, number] | null;
}

/** Places `bodies` around a fixed "나" at the origin and returns each one's centre. */
export function settle(projects: number, bodies: Body[]): [number, number][] {
  const count = bodies.length;
  const TAU = Math.PI * 2;
  const reach = Math.max(72 * Math.sqrt(count + 1), 380);
  const anchors: [number, number][] = Array.from({ length: projects }, (_, i) => {
    if (projects === 1) return [0, -reach * 0.6];
    const angle = -Math.PI / 2 + (i / projects) * TAU;
    return [Math.cos(angle) * reach, Math.sin(angle) * reach * 0.7];
  });
  const order: number[] = [];
  const queued = new Array<boolean>(count).fill(false);
  const visit = (i: number) => {
    if (queued[i]) return;
    queued[i] = true;
    const parent = bodies[i].parent;
    if (parent != null) visit(parent);
    order.push(i);
  };
  [...bodies.keys()].sort((a, b) => bodies[a].seed - bodies[b].seed).forEach(visit);

  const pos: [number, number][] = bodies.map(() => [0, 0]);
  const vel: [number, number][] = bodies.map(() => [0, 0]);
  for (const i of order) {
    const body = bodies[i];
    if (body.old) {
      pos[i] = [...body.old];
      continue;
    }
    const near = body.parent != null ? pos[body.parent] : anchors[body.project];
    const angle = (body.seed / 0xffffffff) * TAU;
    const spread = body.parent != null ? body.radius * 2 : 48;
    pos[i] = [near[0] + Math.cos(angle) * spread, near[1] + Math.sin(angle) * spread];
  }
  // [source, target, length, strength]; a null source is "나".
  const ties = bodies.map((body, i): [number | null, number, number, number] =>
    body.parent != null ? [body.parent, i, bodies[body.parent].radius + body.radius, 0.9] : [null, i, reach * 0.63, 0.04],
  );
  const decay = 1 - Math.pow(0.001, 1 / TICKS);
  let alpha = 1;
  for (let tick = 0; tick < TICKS; tick++) {
    alpha -= alpha * decay;
    for (const [source, target, length, strength] of ties) {
      const [sx, sy] = source == null ? [0, 0] : pos[source];
      const [svx, svy] = source == null ? [0, 0] : vel[source];
      const dx = pos[target][0] + vel[target][0] - sx - svx;
      const dy = pos[target][1] + vel[target][1] - sy - svy;
      const distance = Math.max(Math.hypot(dx, dy), 1e-3);
      const pull = ((distance - length) / distance) * alpha * strength;
      if (source == null) {
        vel[target][0] -= dx * pull;
        vel[target][1] -= dy * pull;
      } else {
        vel[target][0] -= dx * pull * 0.5;
        vel[target][1] -= dy * pull * 0.5;
        vel[source][0] += dx * pull * 0.5;
        vel[source][1] += dy * pull * 0.5;
      }
    }
    for (let i = 0; i < count; i++) {
      const push = (x: number, y: number, charge: number) => {
        const dx = x - pos[i][0];
        const dy = y - pos[i][1];
        const distance = Math.max(dx * dx + dy * dy, 1);
        vel[i][0] += (dx * charge * alpha) / distance;
        vel[i][1] += (dy * charge * alpha) / distance;
      };
      push(0, 0, -600);
      for (let j = 0; j < count; j++) if (j !== i) push(pos[j][0], pos[j][1], bodies[j].parent == null ? -320 : -140);
      const anchor = anchors[bodies[i].project];
      const pull = bodies[i].parent == null ? 0.28 : 0.02;
      vel[i][0] += (anchor[0] - pos[i][0]) * pull * alpha;
      vel[i][1] += (anchor[1] - pos[i][1]) * pull * alpha;
    }
    for (let round = 0; round < 3; round++) {
      for (let i = 0; i < count; i++) {
        for (let j = i + 1; j < count; j++) {
          const room = bodies[i].radius + bodies[j].radius;
          const dx = pos[j][0] + vel[j][0] - pos[i][0] - vel[i][0];
          const dy = pos[j][1] + vel[j][1] - pos[i][1] - vel[i][1];
          const distance = Math.hypot(dx, dy);
          if (distance >= room || distance === 0) continue;
          const overlap = (room - distance) / distance;
          const ri = bodies[i].radius ** 2;
          const rj = bodies[j].radius ** 2;
          const share = rj / (ri + rj);
          vel[i][0] -= dx * overlap * share;
          vel[i][1] -= dy * overlap * share;
          vel[j][0] += dx * overlap * (1 - share);
          vel[j][1] += dy * overlap * (1 - share);
        }
      }
    }
    for (let i = 0; i < count; i++) {
      if (bodies[i].old) {
        vel[i] = [0, 0];
        continue;
      }
      vel[i][0] *= 0.6;
      vel[i][1] *= 0.6;
      pos[i][0] += vel[i][0];
      pos[i][1] += vel[i][1];
    }
  }
  return pos;
}

/** A session on the live map, in world units. It rests at `home`; a `held` one follows the pointer. */
export interface Mover {
  x: number;
  y: number;
  vx: number;
  vy: number;
  r: number;
  home: [number, number];
  held: boolean;
}

/** Room kept around each session for its name. */
const LABEL_ROOM = 34;

/**
 * One tick of the live map: every session springs toward its home, pushes off "나" at the origin
 * and the others, and tugs on its team through `ties` (parent, child). `alpha` heats the springs
 * and fades to zero; sessions keep their room at any heat.
 */
export function step(movers: Mover[], ties: [number, number][], alpha: number): void {
  for (const [parent, child] of ties) {
    const a = movers[parent];
    const b = movers[child];
    const dx = b.x - a.x;
    const dy = b.y - a.y;
    const d = Math.hypot(dx, dy) || 1;
    const pull = ((d - (a.r + b.r + 40)) / d) * 0.3 * alpha;
    a.vx += dx * pull * 0.5;
    a.vy += dy * pull * 0.5;
    b.vx -= dx * pull * 0.5;
    b.vy -= dy * pull * 0.5;
  }
  const push = (m: Mover, x: number, y: number, charge: number, room: number) => {
    const dx = m.x - x;
    const dy = m.y - y;
    const d2 = Math.max(dx * dx + dy * dy, 80);
    const d = Math.sqrt(d2);
    m.vx += (dx * charge * alpha) / d2;
    m.vy += (dy * charge * alpha) / d2;
    if (d < room) {
      m.vx += (dx / d) * (room - d) * 0.25;
      m.vy += (dy / d) * (room - d) * 0.25;
    }
  };
  for (const m of movers) {
    m.vx += (m.home[0] - m.x) * 0.08 * alpha;
    m.vy += (m.home[1] - m.y) * 0.08 * alpha;
    push(m, 0, 0, 420, m.r + 30);
    for (const other of movers) if (other !== m) push(m, other.x, other.y, 150, m.r + other.r + LABEL_ROOM);
  }
  for (const m of movers) {
    if (m.held) {
      m.vx = m.vy = 0;
      continue;
    }
    m.vx *= 0.55;
    m.vy *= 0.55;
    m.x += m.vx;
    m.y += m.vy;
  }
}
