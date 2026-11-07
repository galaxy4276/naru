import type { End, Pair } from './letters';

export type PairHit = { kind: 'pair'; key: string; x: number; y: number; r: number; points: [number, number][] };

export interface Palette {
  ink: string;
  map: string;
  err: string;
  font: string;
}

interface LetterScene {
  pairs: Pair[];
  /** Where a shown end is on screen; `undefined` for an end the map does not show. */
  at: (end: End) => [number, number] | undefined;
  /** An end's orb radius, so arrows stop at its edge. */
  radius: (end: End) => number;
  lit: (pair: Pair) => boolean;
  /** Unix seconds. */
  now: number;
  t: number;
  still: boolean;
  k: number;
}

const LINE = 'rgba(255,255,255,.8)';
const FADED = 0.18;
/** How long after a letter its dot keeps running along the line. */
const FLOW_SECONDS = 60;
const TRIP_MS = 1600;

type Point = [number, number];

const along = ([x0, y0]: Point, [cx, cy]: Point, [x1, y1]: Point, u: number): Point => [
  (1 - u) ** 2 * x0 + 2 * (1 - u) * u * cx + u ** 2 * x1,
  (1 - u) ** 2 * y0 + 2 * (1 - u) * u * cy + u ** 2 * y1,
];

function arrow(ctx: CanvasRenderingContext2D, [x, y]: Point, [cx, cy]: Point, gap: number, k: number) {
  const d = Math.hypot(x - cx, y - cy) || 1;
  const [ux, uy] = [(x - cx) / d, (y - cy) / d];
  const [tx, ty] = [x - ux * gap, y - uy * gap];
  ctx.beginPath();
  ctx.moveTo(tx, ty);
  ctx.lineTo(tx - ux * 7 * k - uy * 4 * k, ty - uy * 7 * k + ux * 4 * k);
  ctx.lineTo(tx - ux * 7 * k + uy * 4 * k, ty - uy * 7 * k - ux * 4 * k);
  ctx.closePath();
  ctx.fill();
}

/**
 * One gently bent line per pair that wrote each other, arrowheads at whoever received, and the
 * letter count in the middle. Faint unless lit; dashed while a letter waits on its answer, red
 * once one failed. Returns what can be clicked.
 */
export function drawLetters(ctx: CanvasRenderingContext2D, s: LetterScene, palette: Palette): PairHit[] {
  const hits: PairHit[] = [];
  const { k, t } = s;
  for (const pair of s.pairs) {
    const p0 = s.at(pair.a);
    const p1 = s.at(pair.b);
    if (!p0 || !p1) continue;
    const length = Math.hypot(p1[0] - p0[0], p1[1] - p0[1]) || 1;
    // Bent to one side, so the line clears the straight parent tie between the same two orbs.
    const bend = Math.min(length * 0.18, 60 * k);
    const c: Point = [(p0[0] + p1[0]) / 2 - ((p1[1] - p0[1]) / length) * bend, (p0[1] + p1[1]) / 2 + ((p1[0] - p0[0]) / length) * bend];
    const lit = s.lit(pair);
    const color = pair.failed ? palette.err : LINE;
    ctx.globalAlpha = lit ? 1 : FADED;
    ctx.strokeStyle = color;
    ctx.fillStyle = color;
    ctx.lineWidth = (lit ? 1.8 : 1.3) * k;
    if (pair.failed || pair.waiting) {
      ctx.setLineDash([6 * k, 4 * k]);
      ctx.lineDashOffset = s.still ? 0 : -t / 40;
    }
    ctx.beginPath();
    ctx.moveTo(...p0);
    ctx.quadraticCurveTo(...c, ...p1);
    ctx.stroke();
    ctx.setLineDash([]);
    if (pair.ab) arrow(ctx, p1, c, s.radius(pair.b) + 5 * k, k);
    if (pair.ba) arrow(ctx, p0, c, s.radius(pair.a) + 5 * k, k);

    if (!s.still && s.now - pair.last < FLOW_SECONDS) {
      const u = (t % TRIP_MS) / TRIP_MS;
      const [dx, dy] = along(p0, c, p1, pair.lastFrom === pair.a ? u : 1 - u);
      ctx.globalAlpha = 1;
      ctx.fillStyle = '#fff';
      ctx.shadowColor = '#fff';
      ctx.shadowBlur = 10 * k;
      ctx.beginPath();
      ctx.arc(dx, dy, 2.8 * k, 0, Math.PI * 2);
      ctx.fill();
      ctx.shadowBlur = 0;
    }

    const [bx, by] = along(p0, c, p1, 0.5);
    const label = String(pair.ab + pair.ba);
    ctx.globalAlpha = lit ? 1 : 0.45;
    ctx.font = `700 ${10 * k}px ${palette.font}`;
    const w = Math.max(ctx.measureText(label).width + 8 * k, 15 * k);
    ctx.fillStyle = pair.failed ? palette.err : palette.ink;
    ctx.beginPath();
    ctx.roundRect(bx - w / 2, by - 7.5 * k, w, 15 * k, 7.5 * k);
    ctx.fill();
    ctx.fillStyle = pair.failed ? '#fff' : palette.map;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(label, bx, by + 0.5);
    const points = Array.from({ length: 13 }, (_, i) => along(p0, c, p1, i / 12));
    hits.push({ kind: 'pair', key: pair.key, x: bx, y: by, r: Math.max(w / 2, 9), points });
  }
  ctx.globalAlpha = 1;
  return hits;
}

/** Who wrote whom how many letters, in a dark tag above the hovered orb. */
export function drawHint(ctx: CanvasRenderingContext2D, [x, y]: Point, lines: string[], k: number, palette: Palette) {
  if (!lines.length) return;
  ctx.font = `600 ${11 * k}px ${palette.font}`;
  const w = Math.max(...lines.map((line) => ctx.measureText(line).width)) + 16 * k;
  const h = (lines.length * 16 + 8) * k;
  const top = y - h;
  ctx.globalAlpha = 0.92;
  ctx.fillStyle = palette.map;
  ctx.strokeStyle = 'rgba(255,255,255,.18)';
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.roundRect(x - w / 2, top, w, h, 8 * k);
  ctx.fill();
  ctx.stroke();
  ctx.globalAlpha = 1;
  ctx.fillStyle = palette.ink;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'top';
  lines.forEach((line, i) => ctx.fillText(line, x, top + (4 + i * 16) * k));
}
