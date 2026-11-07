import type { Terminal } from '@xterm/xterm';

/**
 * Takes input-method text (Hangul) away from xterm. In WKWebView xterm's composition helper
 * slices the committed syllable out of its textarea and loses it when the next syllable starts
 * right away ("안녕" → "ㅇㄴ"), and it sends a syllable's first jamo on keydown. Here the
 * syllable being composed is drawn in xterm's composition view and exactly what the input
 * method commits is sent.
 */
const NEWLINE = '\x1b\r';

export function routeInputMethod(terminal: Terminal, send: (data: string) => void): () => void {
  const textarea = terminal.textarea!;
  const view = terminal.element!.querySelector<HTMLElement>('.composition-view');
  // A key xterm was told to skip; if the browser then inserts plain text for it, that text is ours.
  let skipped = false;

  // Set between compositionstart and compositionend.
  let composing = false;
  // A ⇧↩ pressed while a syllable is still composing; sent after the syllable it commits.
  let newlineAfter = false;

  terminal.attachCustomKeyEventHandler((event) => {
    if (event.key === 'Enter' && event.shiftKey && !event.ctrlKey && !event.altKey && !event.metaKey) {
      // ESC CR: what Claude Code (and Option+Enter) reads as a newline instead of submit.
      if (event.type === 'keydown') {
        if (composing) newlineAfter = true;
        else send(NEWLINE);
      }
      event.preventDefault();
      return false;
    }
    if (event.type !== 'keydown') return !skipped;
    skipped = event.isComposing || event.keyCode === 229 || (event.key.length === 1 && event.key.charCodeAt(0) > 0x7f && !event.ctrlKey && !event.metaKey);
    return !skipped;
  });

  // Cells of text sent but not echoed yet; the next syllable is drawn after them, not on them.
  let pending = 0;
  let shown = '';
  const place = () => {
    const screen = terminal.element!.querySelector<HTMLElement>('.xterm-screen');
    if (!view || !shown || !screen) return;
    const cell = { width: screen.clientWidth / terminal.cols, height: screen.clientHeight / terminal.rows };
    const buffer = terminal.buffer.active;
    const x = buffer.cursorX + pending;
    const wrapped = x >= terminal.cols;
    view.style.left = `${(wrapped ? x - terminal.cols : x) * cell.width}px`;
    view.style.top = `${Math.min(buffer.cursorY + (wrapped ? 1 : 0), terminal.rows - 1) * cell.height}px`;
    view.style.height = `${cell.height}px`;
    view.style.lineHeight = `${cell.height}px`;
    view.style.fontFamily = String(terminal.options.fontFamily);
    view.style.fontSize = `${terminal.options.fontSize}px`;
  };
  const show = (text: string) => {
    shown = text;
    if (!view) return;
    view.textContent = text;
    view.classList.toggle('active', text !== '');
    place();
  };
  const cursorMoved = terminal.onCursorMove(() => {
    pending = 0;
    place();
  });
  const own = (event: Event) => event.stopImmediatePropagation();
  const mine = <E extends Event>(handle: (event: E) => void) => (event: Event) => {
    if (event.target === textarea) handle(event as E);
  };
  const start = (event: Event) => {
    own(event);
    composing = true;
    show('');
  };
  const update = (event: CompositionEvent) => {
    own(event);
    show(event.data);
  };
  const end = (event: CompositionEvent) => {
    own(event);
    show('');
    composing = false;
    if (event.data) {
      send(event.data);
      for (const char of event.data) pending += char.charCodeAt(0) >= 0x1100 ? 2 : 1;
    }
    if (newlineAfter) {
      newlineAfter = false;
      send(NEWLINE);
    }
    textarea.value = '';
  };
  const input = (event: Event) => {
    const { inputType, data, isComposing } = event as InputEvent;
    if (isComposing || inputType === 'insertCompositionText' || inputType === 'insertFromComposition') return own(event);
    if (skipped && inputType === 'insertText' && data) {
      own(event);
      skipped = false;
      send(data);
    }
  };
  // xterm listens on its textarea and on its own element; the window's capture phase is first.
  const listeners: [string, (event: Event) => void][] = [
    ['compositionstart', mine(start)],
    ['compositionupdate', mine(update)],
    ['compositionend', mine(end)],
    ['input', mine(input)],
  ];
  for (const [type, listener] of listeners) window.addEventListener(type, listener, true);
  return () => {
    cursorMoved.dispose();
    for (const [type, listener] of listeners) window.removeEventListener(type, listener, true);
  };
}
