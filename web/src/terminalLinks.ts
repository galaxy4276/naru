import type { IBufferLine, ILink, Terminal } from '@xterm/xterm';

const URL_RE = /https?:\/\/[^\s'"<>()[\]{}]*[^\s'"<>()[\]{}.,;:!?]/g;
// A path needs a slash or a known file ending, so plain words are not links.
const PATH_RE =
  /(?<![\w/.~-])(?:~|\.{1,2})?\/?[\w.@+-]+(?:\/[\w.@+-]+)+(?::\d+){0,2}|(?<![\w/.-])[\w.-]+\.(?:md|mdx|ts|tsx|js|jsx|mjs|rs|py|go|kt|java|swift|json|toml|ya?ml|html?|css|txt|sh|sql|png|jpe?g|gif|svg|webp)(?::\d+){0,2}/g;

/** A buffer line as text, with the 1-based cell where each character starts. */
function cellsOf(line: IBufferLine, cols: number): { text: string; cell: number[] } {
  let text = '';
  const cell: number[] = [];
  for (let x = 0; x < cols; x++) {
    const c = line.getCell(x);
    if (!c || c.getWidth() === 0) continue;
    const chars = c.getChars() || ' ';
    for (const ch of chars) {
      text += ch;
      cell.push(x + 1);
    }
  }
  return { text, cell };
}

/** Links in one row: web addresses, then file paths that are not part of one. */
export function findLinks(text: string): { start: number; end: number; text: string; url: boolean }[] {
  const found: { start: number; end: number; text: string; url: boolean }[] = [];
  for (const m of text.matchAll(URL_RE)) found.push({ start: m.index, end: m.index + m[0].length, text: m[0], url: true });
  for (const m of text.matchAll(PATH_RE)) {
    const start = m.index;
    const end = start + m[0].length;
    if (found.some((link) => start < link.end && end > link.start)) continue;
    found.push({ start, end, text: m[0], url: false });
  }
  return found;
}

/**
 * ⌘-click opens a web address in the browser and a file path in the document lane, as
 * terminals do; a plain click keeps selecting text.
 */
export function linkTerminal(terminal: Terminal, open: (link: { url: boolean; text: string }) => void) {
  return terminal.registerLinkProvider({
    provideLinks(y, callback) {
      const line = terminal.buffer.active.getLine(y - 1);
      if (!line) return callback(undefined);
      const { text, cell } = cellsOf(line, terminal.cols);
      const links: ILink[] = findLinks(text).map((link) => ({
        text: link.text,
        range: { start: { x: cell[link.start], y }, end: { x: cell[link.end - 1], y } },
        decorations: { underline: true, pointerCursor: true },
        activate: (event) => event.metaKey && open(link),
      }));
      callback(links.length ? links : undefined);
    },
  });
}
