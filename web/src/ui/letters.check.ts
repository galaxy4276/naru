import type { Letter } from '../connection.ts';
import { ME, pairsOf, stateOf, threadOf, waitsForAnswer } from './letters.ts';

const same = (actual: unknown, expected: unknown, message = '') => {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`${message}: ${JSON.stringify(actual)} ≠ ${JSON.stringify(expected)}`);
};

const now = 1_000_000;
let next = 0;
const letter = (from: string | null, to: string, more: Partial<Letter> = {}): Letter => ({
  id: ++next,
  from,
  to,
  from_name: '',
  to_name: '',
  from_title: '',
  to_title: '',
  kind: 'request',
  text: '',
  at: now - 60,
  reply_to: null,
  state: 'taken',
  why: null,
  state_at: now,
  answered_by: null,
  ...more,
});

const asked = letter('a', 'b', { kind: 'block', answered_by: 3 });
const letters = [
  asked,
  letter('b', 'a', { kind: 'report' }),
  letter('b', 'a', { kind: 'request', reply_to: 1 }),
  letter(null, 'b'),
  letter('a', 'c', { state: 'dropped', why: "its recipient's pane was closed" }),
];
const [ab, me, ac] = pairsOf(letters, now);
same([ab.a, ab.b, ab.ab, ab.ba], ['a', 'b', 1, 2], 'counts each way');
same(ab.waiting, true, 'the reply is itself a request nobody answered');
same([me.a, me.b, me.waiting], ['b', ME, false], "the user's letters wait on nothing");
same([ac.failed, ac.lastFrom], [true, 'a']);
same(waitsForAnswer(asked, now), false, 'answered');
same(waitsForAnswer(letter('a', 'b', { at: now - 25 * 3600 }), now), false, 'a day passed');
same(stateOf(letters[4], now), { text: '못 전함 · 받는 pane이 닫혔어요', tone: 'fail' });
same(stateOf(letter('a', 'b', { at: now - 7200 }), now).text, '받음 · 답 기다림 2시간');

const healed = pairsOf([letter('a', 'c', { state: 'untaken' }), letter('a', 'c')], now)[0];
same(healed.failed, false, 'a letter that got through after a failure clears it');

same(
  threadOf(letters, 'a', 'b').map((l) => l.id),
  [1, 2, 3, 4],
  "the pair's letters and the user's letters to either",
);

console.log('letters ok');
