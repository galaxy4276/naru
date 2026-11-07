/** Finding a passage of a translated page again: by its words and a little text on each side. */

export interface TextAnchor {
  quote: string;
  prefix: string;
  suffix: string;
}

const ROOM = 64;
/** Characters of a figure's quote that pick the block it follows. */
const TAIL = 24;

export function anchorAt(text: string, start: number, end: number): TextAnchor {
  return { quote: text.slice(start, end), prefix: text.slice(Math.max(0, start - ROOM), start), suffix: text.slice(end, end + ROOM) };
}

/** Where the anchor's quote is in `text`: of several, the one whose surroundings match best. */
export function locate(text: string, anchor: TextAnchor): [number, number] | null {
  if (!anchor.quote) return null;
  let best: [number, number] | null = null;
  let bestScore = -1;
  for (let at = text.indexOf(anchor.quote); at !== -1; at = text.indexOf(anchor.quote, at + 1)) {
    const end = at + anchor.quote.length;
    const score = sharedEnd(text.slice(Math.max(0, at - ROOM), at), anchor.prefix) + sharedStart(text.slice(end, end + ROOM), anchor.suffix);
    if (score > bestScore) [best, bestScore] = [[at, end], score];
  }
  return best;
}

function sharedEnd(a: string, b: string): number {
  let n = 0;
  while (n < a.length && n < b.length && a[a.length - 1 - n] === b[b.length - 1 - n]) n++;
  return n;
}

function sharedStart(a: string, b: string): number {
  let n = 0;
  while (n < a.length && n < b.length && a[n] === b[n]) n++;
  return n;
}

/** The block a figure follows: the one holding the end of its quote, else the block it was put after. */
export function blockOf(blocks: string[], figure: { quote: string; block: number }): number {
  if (!blocks.length) return -1;
  const tail = figure.quote.trim().slice(-TAIL);
  const found = tail ? blocks.findIndex((text) => text.includes(tail)) : -1;
  return found >= 0 ? found : Math.min(figure.block, blocks.length - 1);
}

/** Text nodes under `root` in reading order, leaving figures out. */
function textNodes(root: Node): Text[] {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
    acceptNode: (node) =>
      node instanceof Element ? (node.hasAttribute('data-figure') ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_SKIP) : NodeFilter.FILTER_ACCEPT,
  });
  const nodes: Text[] = [];
  for (let node = walker.nextNode(); node; node = walker.nextNode()) nodes.push(node as Text);
  return nodes;
}

export function textOf(root: Node): string {
  return textNodes(root)
    .map((node) => node.data)
    .join('');
}

/** The position in `textOf(root)` of a DOM point, such as a selection's end. */
export function offsetAt(root: Node, node: Node, offset: number): number {
  const point = document.createRange();
  point.setStart(node, offset);
  let total = 0;
  for (const text of textNodes(root)) {
    if (text === node) return total + offset;
    if (point.comparePoint(text, 0) >= 0) return total;
    total += text.data.length;
  }
  return total;
}

export function rangeAt(root: Node, start: number, end: number): Range | null {
  const range = document.createRange();
  let total = 0;
  let started = false;
  for (const text of textNodes(root)) {
    const length = text.data.length;
    if (!started && start < total + length) {
      range.setStart(text, start - total);
      started = true;
    }
    if (started && end <= total + length) {
      range.setEnd(text, end - total);
      return range;
    }
    total += length;
  }
  return null;
}
