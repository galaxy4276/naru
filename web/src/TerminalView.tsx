import { useEffect, useRef } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebglAddon } from '@xterm/addon-webgl';
import { SearchAddon } from '@xterm/addon-search';
import '@xterm/xterm/css/xterm.css';
import { type Connection, type PaneId, decodeBase64 } from './connection';
import { isDark, onTheme } from './ui/theme';
import { getPrefs, onPrefs, terminalFont } from './ui/prefs';
import { routeInputMethod } from './ime';
import { linkTerminal } from './terminalLinks';

const light = {
  background: '#ffffff',
  foreground: '#18181b',
  cursor: '#18181b',
  cursorAccent: '#ffffff',
  selectionBackground: 'rgba(24, 24, 27, 0.15)',
  black: '#18181b',
  red: '#dc2626',
  green: '#15803d',
  yellow: '#a16207',
  blue: '#1d4ed8',
  magenta: '#9333ea',
  cyan: '#0e7490',
  white: '#a1a1aa',
  brightBlack: '#71717a',
  brightRed: '#ef4444',
  brightGreen: '#16a34a',
  brightYellow: '#ca8a04',
  brightBlue: '#2563eb',
  brightMagenta: '#a855f7',
  brightCyan: '#0891b2',
  brightWhite: '#d4d4d8',
};

const dark = {
  background: '#18181b',
  foreground: '#e4e4e7',
  cursor: '#fafafa',
  cursorAccent: '#18181b',
  selectionBackground: 'rgba(250, 250, 250, 0.2)',
  black: '#27272a',
  red: '#f87171',
  green: '#4ade80',
  yellow: '#facc15',
  blue: '#60a5fa',
  magenta: '#c084fc',
  cyan: '#22d3ee',
  white: '#d4d4d8',
  brightBlack: '#71717a',
  brightRed: '#fca5a5',
  brightGreen: '#86efac',
  brightYellow: '#fde047',
  brightBlue: '#93c5fd',
  brightMagenta: '#d8b4fe',
  brightCyan: '#67e8f9',
  brightWhite: '#fafafa',
};
const themeNow = () => (isDark() ? dark : light);

/** What each mounted pane has selected, for ⌘⇧L. */
export const selections = new Map<PaneId, () => string>();

/** Each mounted pane's ⌘F search, and a way to hand the keyboard back to its terminal. */
export const searches = new Map<PaneId, { addon: SearchAddon; focus: () => void }>();

/** One attached pane. Only mounted panes hold a terminal, so only they use a WebGL context. */
export function TerminalView({ conn, pane, focused }: { conn: Connection; pane: PaneId; focused: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  const term = useRef<Terminal | null>(null);

  useEffect(() => {
    const element = host.current!;
    const prefs = getPrefs();
    const terminal = new Terminal({ fontFamily: terminalFont(prefs.font), fontSize: prefs.term, lineHeight: 1.2, scrollback: 10000, theme: themeNow(), allowProposedApi: true });
    term.current = terminal;
    selections.set(pane, () => terminal.getSelection());
    const offTheme = onTheme(() => (terminal.options.theme = themeNow()));
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    const search = new SearchAddon({ highlightLimit: 2000 });
    terminal.loadAddon(search);
    searches.set(pane, { addon: search, focus: () => terminal.focus() });
    terminal.open(element);
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl.dispose());
      terminal.loadAddon(webgl);
    } catch {
      // The DOM renderer stays in place.
    }
    const sync = () => {
      // A hidden pane (display: none) would fit to the minimum size and shrink its program's PTY.
      if (!element.offsetWidth || !element.offsetHeight) return;
      fit.fit();
      conn.send({ t: 'resize', pane, cols: terminal.cols, rows: terminal.rows });
    };
    const attach = () => {
      conn.send({ t: 'attach', pane });
      sync();
    };
    // The WebGL atlas caches glyphs from whatever font was there when it drew first; once the
    // monospace font has loaded, redraw with it and refit to its cell size.
    let disposed = false;
    const loadFont = () =>
      void document.fonts.load(`${terminal.options.fontSize}px "${getPrefs().font}"`).then(() => {
        if (disposed) return;
        terminal.clearTextureAtlas();
        sync();
      });
    loadFont();
    const offPrefs = onPrefs(() => {
      const { term, font } = getPrefs();
      const family = terminalFont(font);
      if (terminal.options.fontSize === term && terminal.options.fontFamily === family) return;
      terminal.options.fontSize = term;
      terminal.options.fontFamily = family;
      loadFont();
    });
    const offMessage = conn.onMessage((message) => {
      if (message.t === 'replay' && message.pane === pane) {
        // A full reset (RIS) through the write queue: reset() is immediate and would let a replay
        // still queued from an earlier attach land after it.
        terminal.write('\x1bc');
        terminal.write(decodeBase64(message.data));
      } else if (message.t === 'output' && message.pane === pane) {
        terminal.write(decodeBase64(message.data));
      }
    });
    const offOpen = conn.onOpen(attach);
    if (conn.isOpen) attach();
    // ponytail: onBinary (legacy X10 mouse bytes) is not forwarded; add it with a bytes field.
    const input = terminal.onData((data) => conn.send({ t: 'input', pane, data }));
    const offIme = routeInputMethod(terminal, (data) => conn.send({ t: 'input', pane, data }));
    const links = linkTerminal(terminal, (link) =>
      conn.send(link.url ? { t: 'open_link', url: link.text } : { t: 'open_path', pane, path: link.text }),
    );
    const observer = new ResizeObserver(sync);
    observer.observe(element);
    return () => {
      disposed = true;
      selections.delete(pane);
      searches.delete(pane);
      offTheme();
      offPrefs();
      observer.disconnect();
      input.dispose();
      offIme();
      links.dispose();
      offMessage();
      offOpen();
      conn.send({ t: 'detach', pane });
      terminal.dispose();
    };
  }, [conn, pane]);

  useEffect(() => {
    if (focused) term.current?.focus();
  }, [focused]);

  return <div ref={host} className="absolute inset-0 pr-1 pb-2 pl-3.5" />;
}
