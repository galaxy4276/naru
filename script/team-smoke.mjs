import fs from 'node:fs';
const [url, phase, home] = process.argv.slice(2);
const decode = (data) => Buffer.from(data, 'base64').toString();
const fail = (message) => {
  throw new Error(`${phase}: ${message}`);
};

const ws = new WebSocket(url);
const output = new Map();
let state = null;
let letters = null;
const waiters = new Set();
ws.onmessage = (event) => {
  const m = JSON.parse(event.data);
  if (m.t === 'state') state = m;
  if (m.t === 'letters') letters = m.letters;
  if (m.t === 'replay') output.set(m.pane, decode(m.data));
  if (m.t === 'output') output.set(m.pane, (output.get(m.pane) ?? '') + decode(m.data));
  if (m.t === 'error') console.error('daemon error:', m.message);
  for (const waiter of waiters) waiter();
};
await new Promise((resolve, reject) => {
  ws.onopen = resolve;
  ws.onerror = () => reject(new Error(`cannot connect to ${url}`));
});
const send = (message) => ws.send(JSON.stringify(message));

/** Waits until `test()` returns something truthy. */
function until(test, label, ms = 15000) {
  return new Promise((resolve, reject) => {
    const check = () => {
      let value;
      try {
        value = state && test();
      } catch {
        value = undefined;
      }
      if (value) {
        waiters.delete(check);
        clearTimeout(timer);
        resolve(value);
      }
    };
    const timer = setTimeout(() => {
      waiters.delete(check);
      for (const [pane, text] of output) console.error(`--- ${pane.slice(0, 8)}\n${JSON.stringify(text.slice(-700))}`);
      reject(new Error(`${phase}: timed out waiting for ${label}`));
    }, ms);
    waiters.add(check);
    check();
  });
}

const panes = () => state.workspace.panes;
const session = (pane) => panes()[pane]?.session;
const team = (pane) => state.team[pane] ?? {};
const attached = new Set();
const attach = (pane) => {
  if (!attached.has(pane)) send({ t: 'attach', pane }), attached.add(pane);
};
const out = (pane) => output.get(pane) ?? '';
const letter = (id) => letters?.find((l) => `l${l.id}` === id);
const saved = () => JSON.parse(fs.readFileSync(`${home}/.naru/team.json`, 'utf8'));
let step = 0;

/** Has the agent in `pane` run `command` with its shell tool and returns what it printed. */
async function run(pane, command) {
  const marker = `step-${++step}-done`;
  const before = out(pane).length;
  send({ t: 'input', pane, data: `!${command}; echo ${marker}\r` });
  await until(() => out(pane).slice(before).includes(`\n${marker}`) || out(pane).slice(before).includes(`\r${marker}`), command);
  return out(pane).slice(before);
}

await until(() => true, 'state');
const tabs = () => state.workspace.tabs;
const panesOf = (tab) => (tab.layout.kind === 'pane' ? [tab.layout.pane] : tab.layout.children.flatMap((c) => panesOf({ layout: c })));

switch (phase) {
  case 'letters': {
    const parent = tabs()[0].focused;
    attach(parent);
    await until(() => out(parent).includes('%'), 'prompt');
    send({ t: 'input', pane: parent, data: `${home}/bin/claude\r` });
    await until(() => session(parent)?.status.kind === 'success' && session(parent).session_id, 'parent agent');

    const spawned = await run(parent, `naru agent spawn --parent here --title kid --cwd ${home} -- ${home}/bin/claude`);
    const childId = spawned.match(/Started child (a\d+)/)?.[1] ?? fail(`spawn said: ${spawned}`);
    const child = await until(() => Object.keys(panes()).find((p) => panes()[p].parent === parent), 'child pane');
    if (tabs().length !== 2 || panesOf(tabs()[1])[0] !== child) fail('the child card is not right after its parent');
    if (state.workspace.active_tab !== tabs()[0].id) fail('spawn moved the focus');
    if (tabs()[1].title !== 'kid') fail('the child card has no title');
    attach(child);
    await until(() => session(child)?.status.kind === 'success' && session(child).session_id, 'child agent');
    if (team(child).id !== childId) fail(`team view id ${team(child).id}`);

    const ps = await run(parent, 'naru ps');
    if (!ps.includes(`└─ ${childId}  Claude  done  "kid"`)) fail(`ps: ${ps}`);

    const told = await run(parent, `naru tell ${childId} hello-from-parent`);
    const hello = told.match(new RegExp(`Letter (l\\d+) to ${childId} delivered\\.`))?.[1] ?? fail(`tell: ${told}`);
    // The letter's last line gets its Enter 150 ms after the paste.
    await until(() => out(child).includes(`got: (reply with: naru reply ${hello}`), 'letter in the child');
    if (!out(child).includes('got: hello-from-parent')) fail('letter body missing');
    if (!out(child).includes(`[Naru letter ${hello} from`)) fail('letter header missing');
    send({ t: 'read_letters' });
    await until(() => letter(hello)?.state === 'taken', `${hello} taken`);

    const asked = await run(child, 'naru tell parent --kind block which-db');
    const question = asked.match(/Letter (l\d+) to/)?.[1] ?? fail(`question: ${asked}`);
    await until(() => team(child).question === 'which-db', 'question recorded');
    await until(() => out(parent).includes('got: which-db'), 'question reaches the idle parent');
    await until(() => out(parent).includes(`got: (reply with: naru reply ${question}`), 'question fully typed');

    const wrong = await run(child, `naru reply ${question} not-mine 2>&1`);
    if (!wrong.includes('was not written to you')) fail(`replied to its own letter: ${wrong}`);
    const answered = await run(parent, `naru reply ${question} use-postgres`);
    const answer = answered.match(/Reply (l\d+) to/)?.[1] ?? fail(`reply: ${answered}`);
    await until(() => !team(child).question, 'question cleared by the reply');
    await until(() => out(child).includes(`· request · reply to ${question}]`), 'reply header');
    await until(() => letter(question)?.answered_by === Number(answer.slice(1)), 'reply linked');

    send({ t: 'letter_send', to: child, text: 'from-the-user' });
    await until(() => out(child).includes('got: from-the-user'), 'the user\'s letter');
    if (!out(child).includes('from the user · request]')) fail('user letter header');

    const shown = await run(parent, `naru agent show ${childId}`);
    for (const line of ['status: done', 'agent: Claude', 'title: kid', 'parent: a']) {
      if (!shown.includes(line)) fail(`show lacks "${line}": ${shown}`);
    }
    const read = await run(parent, `naru agent read ${childId} --lines 3`);
    if (!read.includes('got: from-the-user')) fail(`read: ${read}`);

    await run(parent, `naru agent send ${childId} typed-by-send --enter`);
    await until(() => out(child).includes('got: typed-by-send'), 'send typed in');
    const refused = await run(parent, `naru agent send ${childId} --key f1 2>&1`);
    if (!refused.includes('unknown key f1')) fail(`bad key accepted: ${refused}`);

    const beside = await run(parent, 'naru pane open --beside -- echo beside-$((6*7))');
    const besideId = beside.match(/Opened (a\d+) beside you/)?.[1] ?? fail(`pane open: ${beside}`);
    const besidePane = await until(() => panesOf(tabs()[0]).find((p) => p !== parent), 'beside pane');
    if (tabs()[0].focused !== parent) fail('pane open moved the focus');
    attach(besidePane);
    await until(() => out(besidePane).includes('beside-42'), `${besideId} ran its command`);

    const reported = await run(child, `naru reply ${hello} done-it`);
    const report = reported.match(/Reply (l\d+) to/)?.[1] ?? fail(`report: ${reported}`);
    await until(() => out(parent).includes('got: done-it'), 'report reaches the parent');
    if (!out(parent).includes(`· report · reply to ${hello}]`)) fail('an answer to a request is not a report');
    await until(() => out(parent).includes(`got: (reply with: naru reply ${report}`), 'report fully typed');

    await run(parent, `naru agent sleep ${childId}`);
    await until(() => session(child) === null && team(child).asleep, 'child asleep');
    const asleep = JSON.parse(fs.readFileSync(`${home}/.naru/asleep.json`, 'utf8'));
    if (Object.keys(asleep).length !== 1) fail(`asleep.json: ${JSON.stringify(asleep)}`);
    await run(parent, `naru tell ${childId} --kind progress kept-for-later`);
    await new Promise((resolve) => setTimeout(resolve, 2500));
    if (!team(child).asleep || team(child).held !== 1) fail(`progress woke the sleeper: ${JSON.stringify(team(child))}`);
    const woke = await run(parent, `naru agent wake ${childId}`);
    if (!woke.includes(`--resume`)) fail(`wake: ${woke}`);
    await until(() => session(child)?.session_id && !team(child).asleep, 'child awake');
    if (team(child).id !== childId) fail('the woken agent lost its id');
    await until(() => out(child).includes('got: kept-for-later'), 'held letter after waking');

    await run(parent, `naru agent sleep ${childId}`);
    await until(() => session(child) === null && team(child).asleep, 'child asleep again');
    await run(parent, `naru tell ${childId} wake-for-this`);
    await until(() => out(child).includes('got: wake-for-this'), 'a request wakes the sleeper', 20000);
    if (team(child).id !== childId) fail('the agent woken for a letter lost its id');
    await until(() => saved().letters.length >= 6, 'letters saved');
    fs.writeFileSync(`${home}/child-id`, `${childId} ${hello}`);

    const marked = await run(parent, 'naru learn mark --title 개념 --hard 어려운-점 --excerpt 발췌');
    if (!marked.includes('Kept in 학습함 as #1.')) fail(`learn: ${marked}`);
    const store = JSON.parse(fs.readFileSync(`${home}/.naru/learn.json`, 'utf8'));
    if (store.marks[0].source?.session_id !== session(parent).session_id) fail('mark has no source');

    fs.writeFileSync(`${home}/doc.md`, '# doc\n');
    fs.writeFileSync(`${home}/review.md`, '# review\n\n## Findings\n\nno verified findings\n');
    await run(parent, `naru open ${home}/doc.md && naru proof ${home}/review.md`);
    await until(() => state.workspace.views.length === 2, 'views');
    const [doc, review] = state.workspace.views;
    if (doc.proof || !review.proof || doc.from !== parent || doc.seen) fail(`views: ${JSON.stringify(state.workspace.views)}`);

    // Plans: a plan put up for review waits on the user, holds its steps, and a step's child
    // reports into it.
    const planned = await run(parent, 'naru plan set --title 내보내기 --ask api:API-만들기 ui:화면-만들기');
    if (!planned.includes('waits in the user')) fail(`plan set: ${planned}`);
    await until(() => team(parent).review === '계획 검토 · 2단계' && team(parent).plan?.review, 'plan review on the parent');
    const early = await run(parent, `naru agent spawn --parent here --plan-node api --cwd ${home} -- ${home}/bin/claude 2>&1`);
    if (!early.includes("waits for the user's review")) fail(`a step went out before review: ${early}`);
    send({ t: 'plan_approve', pane: parent });
    await until(() => !team(parent).review && team(parent).plan && !team(parent).plan.review, 'review lifted');
    await until(() => out(parent).includes('got: 계획을 승인했어요'.replace(/[^a-z0-9-]/g, '')) || out(parent).includes('계획을 승인했어요'), 'the agent is told of the approval');
    const stepSpawn = await run(parent, `naru agent spawn --parent here --plan-node api --title api --cwd ${home} -- ${home}/bin/claude`);
    const stepId = stepSpawn.match(/Started child (a\d+)/)?.[1] ?? fail(`step spawn: ${stepSpawn}`);
    const stepChild = await until(() => team(parent).plan?.steps[0].child, 'step child recorded');
    if (team(parent).plan.steps[0].child_id !== stepId) fail('the step names another child');
    attach(stepChild);
    await until(() => session(stepChild)?.status.kind === 'success', 'step child agent');
    await until(() => team(parent).plan.steps[0].state === 'working', 'step working');
    await run(stepChild, 'naru tell parent --kind report api-done');
    await until(() => team(parent).plan.steps[0].state === 'reported', 'step reported');
    const shownPlan = await run(parent, 'naru plan show');
    if (!shownPlan.includes(`api  [reported · ${stepId}]`) || !shownPlan.includes('ui  [waiting]')) fail(`plan show: ${shownPlan}`);
    // Waiting: the report a parent waits on comes out of naru agent wait, not into its input.
    await run(parent, `(naru agent wait ${stepId} > ${home}/wait-out 2>&1 &)`);
    await new Promise((resolve) => setTimeout(resolve, 2000));
    await run(stepChild, 'naru tell parent --kind report waited-result');
    await until(() => fs.existsSync(`${home}/wait-out`) && fs.readFileSync(`${home}/wait-out`, 'utf8').includes('waited-result'), 'wait prints the report');
    const waited = fs.readFileSync(`${home}/wait-out`, 'utf8');
    if (!waited.startsWith(`${stepId} reported (l`)) fail(`wait: ${waited}`);
    await new Promise((resolve) => setTimeout(resolve, 2500));
    if (out(parent).includes('got: waited-result')) fail('a waited report was also typed into the parent');
    const stranger = await run(stepChild, `naru agent wait ${childId} 2>&1`);
    if (!stranger.includes("is not your child")) fail(`waited on a stranger: ${stranger}`);
    const twice = await run(parent, `naru agent spawn --plan-node ui --cwd ${home} -- true 2>&1`);
    if (!twice.includes('needs --parent here')) fail(`plan-node without a parent: ${twice}`);

    // Memory: only a session a request brought a card's body to can hold it; a held card leaves
    // the hooks and shows its hold.
    const cwd = panes()[parent].cwd;
    await run(parent, `git init -q ${cwd}`);
    const key = (await import('node:crypto')).createHash('sha256').update(cwd).digest('hex').slice(0, 16);
    fs.mkdirSync(`${home}/.naru/memory`, { recursive: true });
    const card = { id: 1, kind: 'rule', title: '금요일 배포 금지', body: '금요일엔 배포하지 않는다', aliases: ['배포'], sources: [], at: 100, archived_at: null, promoted: [] };
    fs.writeFileSync(`${home}/.naru/memory/${key}.json`, JSON.stringify({ root: cwd, next_card: 2, next_change: 1, cards: [card], conflicts: [], log: [], captured: {} }));
    const hook = `printf '%s' '{"cwd":"${cwd}","session_id":"${session(parent).session_id}","prompt":"배포 해도 돼?"}' | naru memory --hook`;
    const given = await run(parent, hook);
    if (!given.includes('m1 [rule]') || !given.includes('naru memory stale')) fail(`memory hook: ${given}`);
    const notMine = await run(stepChild, 'naru memory stale m1 nope 2>&1');
    if (!notMine.includes('was not given')) fail(`a session without the card held it: ${notMine}`);
    const held = await run(parent, 'naru memory stale m1 금요일도-배포한다고-함');
    if (!held.includes('m1 is held')) fail(`stale: ${held}`);
    const after = await run(parent, hook);
    if (after.includes('m1 [rule]') || !after.includes('{}')) fail(`a held card was still given: ${after}`);
    const showHeld = await run(parent, 'naru memory show m1');
    if (!showHeld.includes('held since an agent reported it out of date: 금요일도-배포한다고-함')) fail(`show: ${showHeld}`);
    fs.writeFileSync(`${home}/plan-step`, stepId);

    const attach_ = await run(parent, 'naru node attach 2>&1');
    if (!attach_.includes('not a naru node pane')) fail(`node attach: ${attach_}`);

    send({ t: 'input', pane: child, data: 'interrupt\r' });
    await until(() => session(child)?.status.kind === 'in_progress', 'child working');
    await until(() => session(child)?.status.kind === 'cancelled', 'interrupted turn ended', 15000);
    break;
  }
  case 'restored': {
    if (!Object.keys(panes()).some((p) => panes()[p].parent)) fail('the parent link was lost');
    if (state.workspace.views.length !== 2) fail('views were lost');
    const [childId, hello] = fs.readFileSync(`${home}/child-id`, 'utf8').split(' ');
    await until(() => Object.keys(panes()).some((p) => panes()[p].parent && team(p).id === childId), `the child kept ${childId}`);
    send({ t: 'read_letters' });
    await until(() => letter(hello)?.answered_by, `${hello} and its reply kept`);
    const owner = Object.keys(state.team).find((p) => state.team[p].plan);
    if (!owner) fail('the plan was lost');
    const stepId = fs.readFileSync(`${home}/plan-step`, 'utf8');
    const kept = state.team[owner].plan.steps[0];
    if (kept.state !== 'reported' || kept.child_id !== stepId) fail(`plan step after restart: ${JSON.stringify(kept)}`);
    break;
  }
  default:
    fail('unknown phase');
}
ws.close();
console.log(`${phase} ok`);
