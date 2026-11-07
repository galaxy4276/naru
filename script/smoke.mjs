const [url, phase, home] = process.argv.slice(2);
const decode = (data) => Buffer.from(data, 'base64').toString();
const sessionId = '11111111-1111-4111-8111-111111111111';

function open() {
  const ws = new WebSocket(url);
  const queue = [];
  const waiters = [];
  ws.onmessage = (event) => {
    const message = JSON.parse(event.data);
    const index = waiters.findIndex((waiter) => waiter.test(message));
    if (index >= 0) waiters.splice(index, 1)[0].resolve(message);
    else queue.push(message);
  };
  const next = (test, label) =>
    new Promise((resolve, reject) => {
      const index = queue.findIndex(test);
      if (index >= 0) return resolve(queue.splice(index, 1)[0]);
      const timer = setTimeout(() => reject(new Error(`${phase}: timed out waiting for ${label}`)), 10000);
      waiters.push({ test, resolve: (message) => (clearTimeout(timer), resolve(message)) });
    });
  const send = (message) => ws.send(JSON.stringify(message));
  return new Promise((resolve, reject) => {
    ws.onopen = () => resolve({ ws, next, send });
    ws.onerror = () => reject(new Error(`cannot connect to ${url}`));
  });
}

const fail = (message) => {
  throw new Error(`${phase}: ${message}`);
};
const onlyPane = (state) => {
  const ids = Object.keys(state.workspace.panes);
  if (ids.length !== 1) fail(`expected one pane, found ${ids.length}`);
  return ids[0];
};
const session = (state, pane) => state.workspace.panes[pane]?.session;

/** Collects a pane's output until it contains `text`. */
async function waitForOutput(conn, pane, text) {
  let seen = '';
  for (;;) {
    const message = await conn.next((m) => (m.t === 'replay' || m.t === 'output') && m.pane === pane, `"${text}"`);
    seen = message.t === 'replay' ? decode(message.data) : seen + decode(message.data);
    if (seen.includes(text)) return seen;
  }
}

const conn = await open();
const state = await conn.next((m) => m.t === 'state', 'state');
const pane = onlyPane(state);
conn.send({ t: 'attach', pane });

switch (phase) {
  case 'engine': {
    await waitForOutput(conn, pane, '%');
    conn.send({
      t: 'input',
      pane,
      data: `echo marker-$((40+2)); printf '\\033]777;notify;naru://test;{"op":"ping","x":"a;b"}\\007'\r`,
    });
    const osc = await conn.next((m) => m.t === 'osc' && m.osc.kind === 'notify', 'OSC 777');
    if (osc.osc.title !== 'naru://test' || osc.osc.body !== '{"op":"ping","x":"a;b"}') {
      fail(`wrong OSC: ${JSON.stringify(osc.osc)}`);
    }
    conn.ws.close();
    const again = await open();
    const back = await again.next((m) => m.t === 'state', 'state');
    if (onlyPane(back) !== pane) fail('pane ended when the screen left');
    again.send({ t: 'attach', pane });
    await waitForOutput(again, pane, 'marker-42');
    again.send({ t: 'split', pane, direction: 'right' });
    const split = await again.next((m) => m.t === 'state' && Object.keys(m.workspace.panes).length === 2, 'split');
    const added = Object.keys(split.workspace.panes).find((id) => id !== pane);
    again.send({ t: 'close_pane', pane: added });
    await again.next((m) => m.t === 'state' && Object.keys(m.workspace.panes).length === 1, 'closed pane');
    again.ws.close();
    break;
  }
  case 'agent': {
    conn.send({ t: 'input', pane, data: `${home}/bin/claude\r` });
    const done = await conn.next(
      (m) => m.t === 'state' && session(m, pane)?.status.kind === 'success' && session(m, pane)?.session_id === sessionId,
      'finished turn',
    );
    if (session(done, pane).seen) fail('a finished turn should read as done');
    if (session(done, pane).launcher !== `${home}/bin/claude`) fail('launcher not recorded');
    break;
  }
  case 'resumed':
  case 'resumed-again': {
    const restored = session(state, pane);
    if (restored?.session_id !== sessionId) fail(`session not restored: ${JSON.stringify(restored)}`);
    // The first restore keeps the unhandled turn as done; the user then marks it handled.
    if (restored.seen !== (phase === 'resumed-again')) fail(`seen is ${restored.seen}`);
    await waitForOutput(conn, pane, `fake-claude --resume ${sessionId}`);
    if (phase === 'resumed') {
      conn.send({ t: 'mark_seen', pane });
      await conn.next((m) => m.t === 'state' && session(m, pane)?.seen === true, 'marked seen');
    } else {
      conn.send({ t: 'input', pane, data: '/exit\r' });
      await conn.next((m) => m.t === 'state' && session(m, pane) === null, 'agent exited');
    }
    break;
  }
  case 'shell': {
    if (session(state, pane) !== null) fail('an exited agent came back');
    const output = await waitForOutput(conn, pane, '%');
    await new Promise((resolve) => setTimeout(resolve, 1500));
    if (output.includes('fake-claude')) fail('the exited agent was started again');
    break;
  }
  default:
    fail('unknown phase');
}
conn.ws.close();
console.log(`${phase} ok`);
