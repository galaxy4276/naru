import type { Group, Layout, PaneId, PaneTeam, Session, Tab, Workspace } from '@/connection';

/** How a pane's state reads at a glance. `null` is a plain shell with no agent. */
export type Tone = 'work' | 'turn' | 'done' | 'wait' | 'err';

export type Teams = Record<PaneId, PaneTeam>;

export function toneOf(session: Session | null, team?: PaneTeam): Tone | null {
  if (team?.alert || team?.review) return 'turn';
  if (!session) return null;
  switch (session.status.kind) {
    case 'in_progress':
      return 'work';
    case 'blocked':
      return 'turn';
    case 'failed':
      return 'err';
    case 'cancelled':
      return 'wait';
    case 'success':
      return session.seen ? 'wait' : 'done';
  }
}

export const toneLabel: Record<Tone, string> = {
  work: '작업 중',
  turn: '내 차례',
  done: '완료',
  wait: '대기',
  err: '오류',
};

export function panesOf(layout: Layout): PaneId[] {
  return layout.kind === 'pane' ? [layout.pane] : layout.children.flatMap(panesOf);
}

export function basename(path: string | null | undefined): string | null {
  return path?.split('/').filter(Boolean).pop() ?? null;
}

/** Same order as the daemon's `team::pane_title`, so letters and screens name panes alike. */
export function paneTitle(ws: Workspace, pane: PaneId): string {
  const record = ws.panes[pane];
  const tab = ws.tabs.find((t) => panesOf(t.layout).includes(pane));
  return (
    record?.title ??
    record?.auto_title ??
    tab?.title ??
    record?.program_title ??
    basename(record?.cwd) ??
    'shell'
  );
}

export function tabTitle(ws: Workspace, tab: Tab): string {
  return tab.title ?? paneTitle(ws, tab.focused);
}

/** What an asking agent waits for, e.g. `권한 · Bash: pnpm test`, `질문 · …`, `막힘 · …`. */
export function askLine(session: Session): string | null {
  const join = (verb: string, what: string | null | undefined) => (what ? `${verb} · ${what}` : verb);
  const status = session.status;
  switch (status.kind) {
    case 'failed':
      return join('막힘', status.message ?? status.error_type);
    case 'blocked':
      if (session.tool_name) {
        const what = session.tool_input_preview ? `${session.tool_name}: ${session.tool_input_preview}` : session.tool_name;
        return join('권한', what);
      }
      return join('질문', status.message || session.summary);
    case 'in_progress':
    case 'success':
    case 'cancelled':
      return null;
  }
}

export function waitLabel(since: number | null | undefined, now: number): string {
  if (!since) return '';
  const minutes = Math.floor((now - since) / 60000);
  if (minutes < 1) return '방금';
  if (minutes < 60) return `${minutes}분`;
  return minutes < 24 * 60 ? `${Math.floor(minutes / 60)}시간` : `${Math.floor(minutes / (24 * 60))}일`;
}

export function agoLabel(since: number | null | undefined, now: number): string {
  const wait = waitLabel(since, now);
  return wait && wait !== '방금' ? `${wait} 전` : wait;
}

export interface QueueEntry {
  pane: PaneId;
  title: string;
  detail: string;
  trailing: string;
  /** An ask the user has looked at; it stays, dimmed, until answered. */
  dim: boolean;
  tone: Tone;
  /** The unseen document a 볼 것 row opens. */
  view?: number;
}

export function projectName(ws: Workspace, pane: PaneId): string {
  const tab = ws.tabs.find((t) => panesOf(t.layout).includes(pane));
  const group = ws.groups.find((g) => g.id === tab?.group);
  return group?.name ?? 'Terminal';
}

/** Questions, permissions, errors, alerts and unseen documents, across every project. */
/** Problems proven in each unseen review, by view id, once the screen has read it. */
export type Proofs = Record<number, number>;

/** Proven problems in the pane's newest unseen review, or `null` when it has none to show. */
function unseenProof(ws: Workspace, pane: PaneId, proofs: Proofs): number | null {
  const review = ws.views.filter((v) => v.from === pane && v.proof && !v.seen).at(-1);
  return review && review.id in proofs ? proofs[review.id] : null;
}

export function attention(ws: Workspace, teams: Teams, now: number, proofs: Proofs = {}): QueueEntry[] {
  const entries: QueueEntry[] = [];
  for (const tab of ws.tabs) {
    for (const pane of panesOf(tab.layout)) {
      const session = ws.panes[pane]?.session ?? null;
      const team = teams[pane];
      const views = ws.views.filter((v) => v.from === pane && !v.seen && !v.proof);
      const found = unseenProof(ws, pane, proofs);
      const more = (views.length ? ` · 볼 것 ${views.length}` : '') + (found ? ` · 검토 ${found}` : '');
      const ask = session && askLine(session);
      if (session && ask) {
        entries.push({
          pane,
          title: paneTitle(ws, pane),
          detail: ask + more,
          trailing: session.read ? '읽음' : waitLabel(session.since, now),
          dim: session.read,
          tone: session.status.kind === 'failed' ? 'err' : 'turn',
        });
      } else if (team?.alert || team?.review) {
        entries.push({ pane, title: paneTitle(ws, pane), detail: (team.alert ?? team.review) + more, trailing: '', dim: false, tone: 'turn' });
      } else if (views.length) {
        const newest = views[views.length - 1];
        const others = views.length > 1 ? ` · 외 ${views.length - 1}` : '';
        entries.push({
          pane,
          title: paneTitle(ws, pane),
          detail: `볼 것 · ${basename(newest.path) ?? ''}${others}`,
          trailing: '',
          dim: false,
          tone: 'turn',
          view: newest.id,
        });
      } else if (found) {
        entries.push({ pane, title: paneTitle(ws, pane), detail: `검토 · 증명된 문제 ${found}개`, trailing: '', dim: false, tone: 'turn' });
      }
    }
  }
  return entries;
}

/** Finished turns the user has not looked at yet. */
export function unread(ws: Workspace, now: number, proofs: Proofs = {}): QueueEntry[] {
  const entries: QueueEntry[] = [];
  for (const tab of ws.tabs) {
    for (const pane of panesOf(tab.layout)) {
      const session = ws.panes[pane]?.session;
      const asks = session && askLine(session);
      const clean = !asks && unseenProof(ws, pane, proofs) === 0;
      const done = session && session.status.kind === 'success' && !session.read && !session.seen;
      if (!done && !clean) continue;
      entries.push({
        pane,
        title: paneTitle(ws, pane),
        detail: done ? `${projectName(ws, pane)} · 끝남${clean ? ' · 검토 문제 없음' : ''}` : '검토 · 증명된 문제 없음',
        trailing: done ? agoLabel(session.since, now) : '',
        dim: false,
        tone: 'done',
      });
    }
  }
  return entries;
}

export interface Card {
  tab: Tab;
  depth: number;
}

/** The tab another tab's agent was spawned from with `--parent here`. */
function parentTab(ws: Workspace, tab: Tab, owner: Map<PaneId, Tab>): Tab | null {
  for (const pane of panesOf(tab.layout)) {
    const parent = ws.panes[pane]?.parent;
    const found = parent ? owner.get(parent) : undefined;
    if (found && found.id !== tab.id) return found;
  }
  return null;
}

/**
 * Session cards by project. A child card sits right under its parent, even when its folder
 * belongs to another project: related work stays together.
 */
export function projects(ws: Workspace): { group: Group | null; cards: Card[] }[] {
  const owner = new Map<PaneId, Tab>();
  for (const tab of ws.tabs) for (const pane of panesOf(tab.layout)) owner.set(pane, tab);
  const children = new Map<number, Tab[]>();
  const roots: Tab[] = [];
  for (const tab of ws.tabs) {
    const parent = parentTab(ws, tab, owner);
    if (parent) children.set(parent.id, [...(children.get(parent.id) ?? []), tab]);
    else roots.push(tab);
  }
  const walk = (tab: Tab, depth: number, seen: Set<number>): Card[] => {
    if (seen.has(tab.id)) return [];
    seen.add(tab.id);
    return [{ tab, depth }, ...(children.get(tab.id) ?? []).flatMap((child) => walk(child, depth + 1, seen))];
  };
  const seen = new Set<number>();
  const lists: { group: Group | null; cards: Card[] }[] = [];
  const loose = roots.filter((tab) => !ws.groups.some((g) => g.id === tab.group));
  if (loose.length) lists.push({ group: null, cards: loose.flatMap((tab) => walk(tab, 0, seen)) });
  for (const group of ws.groups) {
    lists.push({ group, cards: roots.filter((tab) => tab.group === group.id).flatMap((tab) => walk(tab, 0, seen)) });
  }
  return lists;
}

export interface CardLine {
  text: string;
  tone: 'turn' | 'err' | 'muted';
}

/** A card's second line: what needs the user first, then what the session is about. */
export function cardLine(ws: Workspace, teams: Teams, tab: Tab): CardLine | null {
  const panes = panesOf(tab.layout);
  const kids = Object.entries(ws.panes).filter(([, record]) => record.parent && panes.includes(record.parent));
  for (const [kid] of kids) {
    const team = teams[kid];
    if (team?.question) return { text: `질문 · ${paneTitle(ws, kid)}: ${team.question}`, tone: 'turn' };
    if (team?.stalled) return { text: `멈춤 · ${paneTitle(ws, kid)} · ${team.stalled}`, tone: 'turn' };
  }
  const focused = ws.panes[tab.focused];
  const held = panes.reduce((sum, pane) => sum + (teams[pane]?.held ?? 0), 0);
  if (held) return { text: `편지 ${held}통 · 일이 끝나면 넣어요`, tone: 'muted' };
  const session = focused?.session;
  const ask = session && askLine(session);
  if (ask) return { text: ask, tone: session.status.kind === 'failed' ? 'err' : 'turn' };
  const review = teams[tab.focused]?.review;
  if (review) return { text: review, tone: 'turn' };
  if (session?.subagents) return { text: `서브에이전트 ${session.subagents}개 작업 중`, tone: 'muted' };
  if (teams[tab.focused]?.asleep) return { text: '잠듦 · naru agent wake로 깨워요', tone: 'muted' };
  const about = focused?.summary ?? session?.query;
  if (about) return { text: about, tone: 'muted' };
  const path = focused?.node ? `${focused.node.host}:${focused.node.cwd ?? '~'}` : focused?.cwd;
  return path ? { text: path.split('/').slice(-2).join('/'), tone: 'muted' } : null;
}

export interface Family {
  /** Live children of the closing panes, outside them. */
  children: PaneId[];
  /** One line per child that still works or waits for an answer; empty when all are done. */
  busy: string[];
}

/** Supervised children a close would orphan. Done children simply close along. */
export function family(ws: Workspace, teams: Teams, closing: PaneId[]): Family {
  const children = Object.entries(ws.panes)
    .filter(([pane, record]) => record.parent && closing.includes(record.parent) && !closing.includes(pane))
    .map(([pane]) => pane)
    .sort((a, b) => (teams[a]?.id ?? '').localeCompare(teams[b]?.id ?? '', undefined, { numeric: true }));
  const busy = children.flatMap((child) => {
    const status = ws.panes[child]?.session?.status.kind;
    const word = teams[child]?.question || status === 'blocked' ? '답 대기' : status === 'in_progress' ? '작업 중' : null;
    if (!word) return [];
    const id = teams[child]?.id;
    return [`${id ? `${id} ` : ''}${paneTitle(ws, child)} · ${word}`];
  });
  return { children, busy };
}

/** Unix seconds as `HH:MM` today, else `M/D HH:MM`. */
export function timeLabel(seconds: number): string {
  const at = new Date(seconds * 1000);
  const time = at.toLocaleTimeString('ko-KR', { hour: '2-digit', minute: '2-digit', hour12: false });
  return at.toDateString() === new Date().toDateString() ? time : `${at.getMonth() + 1}/${at.getDate()} ${time}`;
}

/** Which rail project a tab belongs to: its group's id, or `loose` for tabs with no group. */
export function projectKeyOf(ws: Workspace, tab: Tab | undefined): string {
  return tab && ws.groups.some((g) => g.id === tab.group) ? String(tab.group) : 'loose';
}

export function paneProjectKey(ws: Workspace, pane: PaneId): string {
  return projectKeyOf(ws, ws.tabs.find((t) => panesOf(t.layout).includes(pane)));
}

export function groupKey(group: Group | null): string {
  return group ? String(group.id) : 'loose';
}

/** The rail's projects: every group, plus Terminal when tabs without a group exist. */
export function railProjects(ws: Workspace): { key: string; name: string }[] {
  const list = ws.groups.map((g) => ({ key: String(g.id), name: g.name }));
  if (ws.tabs.some((t) => projectKeyOf(ws, t) === 'loose')) list.push({ key: 'loose', name: 'Terminal' });
  return list;
}

const URGENCY: Record<Tone, number> = { turn: 0, err: 0, work: 1, wait: 2, done: 3 };

/** The most urgent tone among a tab's panes; a plain shell reads as waiting. */
export function tabTone(ws: Workspace, teams: Teams, tab: Tab): Tone {
  const tones = panesOf(tab.layout).flatMap((pane) => toneOf(ws.panes[pane]?.session ?? null, teams[pane]) ?? []);
  return tones.sort((a, b) => URGENCY[a] - URGENCY[b])[0] ?? 'wait';
}

export interface Task {
  root: Tab;
  kids: Tab[];
  project: Group | null;
  tone: Tone;
}

/** A root session plus its children, in the state of the most urgent of them. */
export function tasks(ws: Workspace, teams: Teams): Task[] {
  const out: Task[] = [];
  for (const { group, cards } of projects(ws)) {
    for (const { tab, depth } of cards) {
      if (depth === 0) out.push({ root: tab, kids: [], project: group, tone: tabTone(ws, teams, tab) });
      else {
        const task = out[out.length - 1];
        task.kids.push(tab);
        const tone = tabTone(ws, teams, tab);
        if (URGENCY[tone] < URGENCY[task.tone]) task.tone = tone;
      }
    }
  }
  return out;
}

export const TASK_SECTIONS: { title: string; tones: Tone[] }[] = [
  { title: '내 차례', tones: ['turn', 'err'] },
  { title: '작업 중', tones: ['work'] },
  { title: '대기', tones: ['wait'] },
  { title: '끝남', tones: ['done'] },
];

/** Agent events the 알림 settings can turn on. */
export type NotifyEvent = 'ask' | 'done' | 'err' | 'child';

/** What a pane is calling the user about now, if anything. */
function eventOf(ws: Workspace, teams: Teams, pane: PaneId): { event: NotifyEvent; body: string } | null {
  const alert = teams[pane]?.alert;
  if (alert) return { event: 'child', body: alert };
  const review = teams[pane]?.review;
  if (review) return { event: 'ask', body: review };
  const session = ws.panes[pane]?.session;
  if (!session) return null;
  const status = session.status;
  switch (status.kind) {
    case 'blocked':
      return { event: 'ask', body: status.message ?? session.query ?? '답을 기다려요' };
    case 'failed':
      return { event: 'err', body: status.message ?? status.error_type ?? '오류가 났어요' };
    case 'success':
      return session.seen ? null : { event: 'done', body: ws.panes[pane]?.summary ?? session.response ?? '턴이 끝났어요' };
    case 'in_progress':
    case 'cancelled':
      return null;
  }
}

export interface Alert {
  pane: PaneId;
  event: NotifyEvent;
  title: string;
  body: string;
}

/**
 * Panes that started calling since `before`, which is updated in place. A pane is reported once
 * per call: it has to quiet down or change what it calls about before it is reported again.
 */
export function newAlerts(before: Map<PaneId, NotifyEvent>, ws: Workspace, teams: Teams): Alert[] {
  const alerts: Alert[] = [];
  const now = new Set<PaneId>();
  for (const tab of ws.tabs) {
    const project = ws.groups.find((group) => group.id === tab.group)?.name;
    for (const pane of panesOf(tab.layout)) {
      now.add(pane);
      const found = eventOf(ws, teams, pane);
      if (!found) {
        before.delete(pane);
        continue;
      }
      if (before.get(pane) === found.event) continue;
      before.set(pane, found.event);
      const title = [project, paneTitle(ws, pane)].filter(Boolean).join(' · ');
      alerts.push({ pane, event: found.event, title, body: found.body.replace(/\s+/g, ' ').slice(0, 180) });
    }
  }
  for (const pane of [...before.keys()]) if (!now.has(pane)) before.delete(pane);
  return alerts;
}
