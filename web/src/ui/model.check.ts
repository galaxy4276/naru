// Runs with `pnpm check:model` (node strips the types). Fails loudly if a sidebar rule breaks.
import type { PaneRecord, Session, Workspace } from '@/connection';
import { attention, cardLine, family, newAlerts, projects, unread } from './model.ts';
import { parseDecisions, parseReview, reverted } from './proof.ts';
import { type Mover, graphOf, settle, step, visible } from './overviewGraph.ts';

function ok(condition: unknown, message: string): void {
  if (!condition) throw new Error(message);
}

const session = (status: Session['status'], extra: Partial<Session> = {}): Session => ({
  agent: 'claude', status, session_id: 's', launcher: 'claude', launch_command: null, cwd: null, project: null,
  query: null, response: null, transcript_path: null, summary: null, tool_name: null, tool_input_preview: null,
  plugin_version: null, seen: false, read: false, since: 0, remote: false, ...extra,
});
const pane = (extra: Partial<PaneRecord>): PaneRecord => ({
  cwd: '/x/naru', title: null, auto_title: null, summary: null, session: null, running: null, program_title: null, parent: null, node: null, ...extra,
});
const tab = (id: number, panes: string[], group: number | null) => ({
  id, group, title: null, color: null, pinned: false, background: null, focused: panes[0],
  layout: panes.length === 1
    ? { kind: 'pane' as const, pane: panes[0] }
    : { kind: 'split' as const, row: true, children: panes.map((p) => ({ kind: 'pane' as const, pane: p })), flex: panes.map(() => 1) },
});
const ws: Workspace = {
  groups: [{ id: 1, name: 'naru', path: null, color: null, collapsed: false, pinned: false }],
  tabs: [tab(10, ['p', 'sh'], 1), tab(11, ['kid'], null), tab(12, ['done'], 1)],
  active_tab: 10,
  panes: {
    p: pane({ title: 'parent', session: session({ kind: 'blocked', message: null }, { tool_name: 'Bash', tool_input_preview: 'ls', read: true }) }),
    sh: pane({}),
    kid: pane({ title: 'kid', parent: 'sh', session: session({ kind: 'in_progress' }) }),
    done: pane({ title: 'done', session: session({ kind: 'success' }) }),
  },
  views: [{ id: 1, path: '/x/a.md', proof: false, from: 'kid', seen: false }],
};
const teams = { kid: { id: 'a1', question: 'which db?', stalled: null, alert: null, held: 0, asleep: false, not_taken: false, plan: null, review: null } };

const asks = attention(ws, teams, 0);
ok(asks.length === 2, `two asks, got ${asks.length}`);
ok(asks[0].detail === '권한 · Bash: ls' && asks[0].dim && asks[0].trailing === '읽음', 'a read permission stays, dimmed');
ok(asks[1].detail === '볼 것 · a.md' && asks[1].view === 1, 'an unseen document waits as 볼 것');
ok(unread(ws, 0).map((e) => e.pane).join() === 'done', 'only the unread finished turn is 읽을 것');

const cards = projects(ws);
ok(cards[0].group?.name === 'naru', 'the child is not a loose root');
ok(cards[0].cards.map((c) => `${c.tab.id}:${c.depth}`).join() === '10:0,11:1,12:0', 'child card right under its parent, even across projects');
ok(cardLine(ws, teams, ws.tabs[0])?.text === '질문 · kid: which db?', "a child's question shows on the parent card");

const closing = family(ws, teams, ['p', 'sh']);
ok(closing.children.join() === 'kid' && closing.busy[0] === 'a1 kid · 답 대기', 'closing the parent asks about its asking child');
ok(family(ws, teams, ['done']).children.length === 0, 'no family, no question');
const withProof: Workspace = { ...ws, views: [...ws.views, { id: 2, path: '/x/r.md', proof: true, from: 'done', seen: false }] };
ok(unread(withProof, 0, { 2: 0 })[0].detail === 'naru · 끝남 · 검토 문제 없음', 'a clean review rides on the done row');
ok(attention(withProof, teams, 0, { 2: 3 }).some((e) => e.detail === '검토 · 증명된 문제 3개'), 'proven problems wait on the user');
const heard = new Map();
const first = newAlerts(heard, ws, teams);
ok(first.map((a) => `${a.pane}:${a.event}`).join() === 'p:ask,done:done' && first[0].title === 'naru · parent', `calls since nothing: ${first.map((a) => a.pane)}`);
ok(newAlerts(heard, ws, teams).length === 0, 'a call is reported once');
const childStuck = { ...teams, sh: { ...teams.kid, alert: 'kid is stuck' } };
const later: Workspace = { ...ws, panes: { ...ws.panes, p: pane({ title: 'parent', session: session({ kind: 'in_progress' }) }) } };
const second = newAlerts(heard, later, childStuck);
ok(second.length === 1 && second[0].event === 'child' && second[0].body === 'kid is stuck', 'a stuck child is news; a working parent is not');
ok(newAlerts(heard, ws, childStuck).map((a) => a.pane).join() === 'p', 'asking again after working is reported again');
const reviewing = { ...teams, done: { ...teams.kid, question: null, review: '계획 검토 · 3단계' } };
ok(attention(ws, reviewing, 0).some((e) => e.pane === 'done' && e.detail === '계획 검토 · 3단계' && e.tone === 'turn'), 'a plan waiting for review waits on the user');
ok(newAlerts(new Map(), ws, reviewing).some((a) => a.pane === 'done' && a.event === 'ask'), 'a plan review calls like a question');
console.log('model ok');

const review = parseReview(
  ['# r', '## Findings', '### Letter sent twice', 'Found by: adversary-2', 'Breaks retry.', '```', 'naru tell a2', '```', '```', 'sent twice', '```', '## Dropped', '- adversary-1: 3 dropped', '- adversary-2: none'].join('\n'),
);
ok(review?.findings.length === 1, 'one finding');
ok(review?.findings[0].foundBy === 'adversary-2' && review.findings[0].command === 'naru tell a2' && review.findings[0].output === 'sent twice', 'finding parts');
ok(review?.droppedCount === 3, `dropped count ${review?.droppedCount}`);
ok(parseReview('# just notes') === null, 'no findings section, no review');
const log = parseDecisions('ts\tphase\tdecision\twhy\tevidence\tresult\n1\tplan\tA\tB\tC\t되돌림\nbad line\n');
ok(log.rows.length === 1 && reverted(log.rows[0]) && log.broken === 1, 'decision rows');
console.log('proof ok');

// 한눈에 보기: sh started kid, so kid folds into sh and its question rises there.
const graph = graphOf(ws, teams);
ok(graph.all.map((n) => n.pane).sort().join() === 'done,kid,p,sh', 'agent panes and team panes are on the map');
ok(graph.all.find((n) => n.pane === 'kid')?.parent === 'sh', 'kid hangs off sh');
const folded = visible(graph, new Set(['sh']), new Set());
const sh = folded.find((n) => n.pane === 'sh');
ok(!folded.some((n) => n.pane === 'kid') && sh?.hidden === 1, 'folding a team hides its children');
const bodies = [
  { seed: 1, project: 0, parent: null, radius: 66, old: [500, 500] as [number, number] },
  { seed: 2, project: 0, parent: 0, radius: 44, old: null },
  { seed: 3, project: 1, parent: null, radius: 66, old: null },
];
const placed = settle(2, bodies);
ok(placed[0][0] === 500 && placed[0][1] === 500, 'a placed session holds still');
ok(Math.hypot(placed[1][0] - 500, placed[1][1] - 500) < 250, 'a child settles near its parent');
ok(JSON.stringify(settle(2, bodies)) === JSON.stringify(placed), 'placement is the same every time');
const mover = (x: number, home: [number, number], held = false): Mover => ({ x, y: home[1], vx: 0, vy: 0, r: 14, home, held });
const pair = [mover(300, [205, 0]), mover(205, [205, 0], true)];
for (let i = 0; i < 300; i++) step(pair, [], 0.5);
ok(pair[1].x === 205, 'a held session stays under the pointer');
ok(Math.hypot(pair[0].x - pair[1].x, pair[0].y - pair[1].y) >= 14 + 14 + 30, 'sessions keep room for their names');
const alone = [mover(520, [260, -180])];
for (let i = 0; i < 300; i++) step(alone, [], 0.3);
ok(Math.hypot(alone[0].x - 260, alone[0].y + 180) < 40, 'a let-go session springs back home');
console.log('overview ok');
