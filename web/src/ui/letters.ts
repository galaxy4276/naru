import type { Letter, LetterKind, PaneId } from '../connection.ts';

/** "나", the user, as an end of a letter line. */
export const ME = 'me';
export type End = PaneId | typeof ME;

/** How long a request or question waits on its answer before it stops waiting. */
const ANSWER_WAIT = 24 * 60 * 60;

/** Everything two ends wrote each other, as one line on the map. */
export interface Pair {
  key: string;
  /** The ends in a stable order; `ab` counts letters from `a` to `b`. */
  a: End;
  b: End;
  ab: number;
  ba: number;
  /** A request or question between them still waits on its answer. */
  waiting: boolean;
  /** A letter between them failed, and none got through after it. */
  failed: boolean;
  /** When the last letter was sent, and who sent it. */
  last: number;
  lastFrom: End;
}

export const KIND_LABEL: Record<LetterKind, string> = { request: '요청', block: '질문', report: '보고', progress: '진행' };

const WHY: Record<string, string> = {
  'waited 60 minutes without being delivered': '60분 동안 전하지 못했어요',
  "its recipient's pane was closed": '받는 pane이 닫혔어요',
};

export const endOf = (pane: PaneId | null): End => pane ?? ME;

const ordered = (x: End, y: End): [End, End] => (x < y ? [x, y] : [y, x]);

export const pairKey = (x: End, y: End) => ordered(x, y).join('|');

export const failed = (letter: Letter) => letter.state === 'dropped' || letter.state === 'untaken';

/** An agent's request or question nobody answered yet, sent less than a day ago. */
export function waitsForAnswer(letter: Letter, now: number): boolean {
  return (
    letter.from !== null &&
    (letter.kind === 'request' || letter.kind === 'block') &&
    letter.answered_by === null &&
    letter.state !== 'dropped' &&
    now - letter.at < ANSWER_WAIT
  );
}

export function pairsOf(letters: Letter[], now: number): Pair[] {
  const pairs = new Map<string, Pair>();
  // Oldest first, so a letter that got through clears an earlier failure.
  for (const letter of [...letters].sort((x, y) => x.id - y.id)) {
    const from = endOf(letter.from);
    const [a, b] = ordered(from, letter.to);
    const key = `${a}|${b}`;
    const pair = pairs.get(key) ?? { key, a, b, ab: 0, ba: 0, waiting: false, failed: false, last: 0, lastFrom: from };
    if (from === a) pair.ab++;
    else pair.ba++;
    pair.waiting ||= waitsForAnswer(letter, now);
    if (failed(letter)) pair.failed = true;
    else if (letter.state === 'taken') pair.failed = false;
    if (letter.at >= pair.last) [pair.last, pair.lastFrom] = [letter.at, from];
    pairs.set(key, pair);
  }
  return [...pairs.values()];
}

/** The letters between `a` and `b`, oldest first, with the user's letters to either of them. */
export function threadOf(letters: Letter[], a: End, b: End): Letter[] {
  return letters
    .filter((letter) => {
      const from = endOf(letter.from);
      return (from === a && letter.to === b) || (from === b && letter.to === a) || (from === ME && (letter.to === a || letter.to === b));
    })
    .sort((x, y) => x.id - y.id);
}

const span = (seconds: number) => (seconds < 3600 ? `${Math.max(1, Math.floor(seconds / 60))}분` : `${Math.floor(seconds / 3600)}시간`);

/** What became of a letter, and how a screen colours it. */
export function stateOf(letter: Letter, now: number): { text: string; tone: 'muted' | 'wait' | 'fail' } {
  if (letter.state === 'dropped') return { text: `못 전함 · ${WHY[letter.why ?? ''] ?? letter.why ?? ''}`, tone: 'fail' };
  if (letter.state === 'untaken') return { text: '안 받음 · 입력창에 넣었지만 받지 않았어요', tone: 'fail' };
  const word = { held: '기다림', typed: '넣음', taken: '받음' }[letter.state];
  return waitsForAnswer(letter, now) ? { text: `${word} · 답 기다림 ${span(now - letter.at)}`, tone: 'wait' } : { text: word, tone: 'muted' };
}
