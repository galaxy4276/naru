// Runs with `pnpm check:model`. A connection closed and restarted at once (React's development
// double effect) must end with one live socket, and the replaced socket must stay silent.
import { Connection } from './connection.ts';

class FakeSocket {
  static OPEN = 1;
  static all: FakeSocket[] = [];
  readyState = 0;
  onopen?: () => void;
  onmessage?: (event: { data: string }) => void;
  onclose?: () => void;
  constructor() {
    FakeSocket.all.push(this);
  }
  open() {
    this.readyState = 1;
    this.onopen?.();
  }
  close() {
    this.readyState = 3;
    queueMicrotask(() => this.onclose?.());
  }
  send() {}
}
(globalThis as { WebSocket?: unknown }).WebSocket = FakeSocket;

const conn = new Connection('ws://test');
let opens = 0;
const seen: string[] = [];
conn.onOpen(() => (opens += 1));
conn.onMessage((message) => seen.push(message.t));
conn.start();
conn.close();
conn.start();
await new Promise((done) => setTimeout(done, 1200));
const [first, second] = FakeSocket.all;
second.open();
first.onmessage?.({ data: JSON.stringify({ t: 'stale' }) });
second.onmessage?.({ data: JSON.stringify({ t: 'live' }) });
if (FakeSocket.all.length !== 2) throw new Error(`expected 2 sockets, got ${FakeSocket.all.length}`);
if (opens !== 1 || seen.join() !== 'live') throw new Error(`stale socket leaked: opens=${opens} seen=${seen}`);
console.log('connection ok');
