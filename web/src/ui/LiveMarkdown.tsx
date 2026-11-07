import { useEffect, useRef } from 'react';
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { HighlightStyle, syntaxHighlighting, syntaxTree } from '@codemirror/language';
import { EditorState, type Extension, type Range } from '@codemirror/state';
import { Decoration, type DecorationSet, EditorView, ViewPlugin, type ViewUpdate, WidgetType, keymap } from '@codemirror/view';
import { tags } from '@lezer/highlight';

class Bullet extends WidgetType {
  eq() {
    return true;
  }
  toDOM() {
    const dot = document.createElement('span');
    dot.className = 'cm-bullet';
    dot.textContent = '•';
    return dot;
  }
}

class Check extends WidgetType {
  constructor(
    readonly done: boolean,
    readonly at: number,
  ) {
    super();
  }
  eq(other: Check) {
    return other.done === this.done && other.at === this.at;
  }
  toDOM(view: EditorView) {
    const box = document.createElement('input');
    box.type = 'checkbox';
    box.checked = this.done;
    box.className = 'cm-check';
    // The cursor stays where it was; the box only flips the marker.
    box.onmousedown = (event) => event.preventDefault();
    box.onclick = (event) => {
      event.preventDefault();
      view.dispatch({ changes: { from: this.at + 1, to: this.at + 2, insert: this.done ? ' ' : 'x' } });
    };
    return box;
  }
}

class Rule extends WidgetType {
  eq() {
    return true;
  }
  toDOM() {
    const line = document.createElement('span');
    line.className = 'cm-rule';
    return line;
  }
}

const hide = Decoration.replace({});
const WIKILINK = /\[\[([^\]\n]+)\]\]/g;
const TAG = /(^|\s)(#[\p{L}\p{N}_/-]+)/gu;

/** Lines holding a cursor or selection show their raw markdown; every other line reads rendered. */
function decorations(view: EditorView): DecorationSet {
  const { state } = view;
  const active = new Set<number>();
  for (const range of state.selection.ranges) {
    for (let line = state.doc.lineAt(range.from).number; line <= state.doc.lineAt(range.to).number; line++) active.add(line);
  }
  const shown = (pos: number) => active.has(state.doc.lineAt(pos).number);
  const out: Range<Decoration>[] = [];
  const lineClass = (from: number, to: number, cls: string) => {
    for (let line = state.doc.lineAt(from).number; line <= state.doc.lineAt(to).number; line++) {
      out.push(Decoration.line({ class: cls }).range(state.doc.line(line).from));
    }
  };
  for (const { from, to } of view.visibleRanges) {
    syntaxTree(state).iterate({
      from,
      to,
      enter: (node) => {
        const name = node.name;
        const heading = /^ATXHeading(\d)$/.exec(name);
        if (heading) lineClass(node.from, node.from, `cm-h${heading[1]}`);
        else if (name === 'FencedCode') lineClass(node.from, node.to, 'cm-codeblock');
        else if (name === 'Blockquote') lineClass(node.from, node.to, 'cm-quote');
        if (shown(node.from)) return;
        switch (name) {
          case 'HeaderMark': {
            const space = state.sliceDoc(node.to, node.to + 1) === ' ' ? 1 : 0;
            out.push(hide.range(node.from, node.to + space));
            break;
          }
          case 'EmphasisMark':
          case 'StrikethroughMark':
          case 'QuoteMark':
            out.push(hide.range(node.from, node.to));
            break;
          case 'CodeMark':
            if (node.node.parent?.name === 'InlineCode') out.push(hide.range(node.from, node.to));
            break;
          case 'LinkMark':
          case 'URL':
          case 'LinkTitle':
            if (node.node.parent?.name === 'Link') out.push(hide.range(node.from, node.to));
            break;
          case 'ListMark':
            if (node.node.parent?.parent?.name === 'BulletList' && !/^\s*\[[ xX]\]/.test(state.sliceDoc(node.to + 1, node.to + 5))) {
              out.push(Decoration.replace({ widget: new Bullet() }).range(node.from, node.to));
            } else if (node.node.parent?.parent?.name === 'BulletList') {
              out.push(hide.range(node.from, node.to + 1));
            }
            break;
          case 'TaskMarker':
            out.push(Decoration.replace({ widget: new Check(/x/i.test(state.sliceDoc(node.from, node.to)), node.from) }).range(node.from, node.to));
            break;
          case 'HorizontalRule':
            out.push(Decoration.replace({ widget: new Rule() }).range(node.from, node.to));
            break;
        }
      },
    });
    // Obsidian's own syntax the markdown grammar does not know.
    const text = state.sliceDoc(from, to);
    for (const match of text.matchAll(WIKILINK)) {
      const at = from + (match.index ?? 0);
      const end = at + match[0].length;
      out.push(Decoration.mark({ class: 'cm-wikilink' }).range(at + 2, end - 2));
      if (!shown(at)) out.push(hide.range(at, at + 2), hide.range(end - 2, end));
    }
    for (const match of text.matchAll(TAG)) {
      const at = from + (match.index ?? 0) + match[1].length;
      out.push(Decoration.mark({ class: 'cm-tag' }).range(at, at + match[2].length));
    }
  }
  return Decoration.set(out, true);
}

const livePreview = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = decorations(view);
    }
    update(update: ViewUpdate) {
      if (update.docChanged || update.selectionSet || update.viewportChanged || syntaxTree(update.startState) !== syntaxTree(update.state)) {
        this.decorations = decorations(update.view);
      }
    }
  },
  { decorations: (plugin) => plugin.decorations },
);

const style = HighlightStyle.define([
  { tag: tags.strong, fontWeight: '700' },
  { tag: tags.emphasis, fontStyle: 'italic' },
  { tag: tags.strikethrough, textDecoration: 'line-through' },
  { tag: tags.heading, fontWeight: '700' },
  { tag: tags.link, textDecoration: 'underline', textUnderlineOffset: '3px' },
  { tag: tags.monospace, fontFamily: 'ui-monospace, Menlo, monospace', fontSize: '0.92em' },
  { tag: [tags.processingInstruction, tags.url, tags.meta], color: 'var(--color-muted-foreground)' },
  { tag: tags.quote, color: 'var(--color-muted-foreground)' },
]);

const theme = EditorView.theme({
  '&': { height: '100%', fontSize: '13.5px', backgroundColor: 'transparent' },
  '&.cm-focused': { outline: 'none' },
  '.cm-scroller': { fontFamily: 'inherit', lineHeight: '1.7', padding: '0 0 16px' },
  '.cm-content': { padding: '0 16px', caretColor: 'var(--color-foreground)' },
  '.cm-line': { padding: '0' },
  '.cm-h1': { fontSize: '1.55em', fontWeight: '700', paddingTop: '6px' },
  '.cm-h2': { fontSize: '1.3em', fontWeight: '700', paddingTop: '4px' },
  '.cm-h3': { fontSize: '1.12em', fontWeight: '700' },
  '.cm-codeblock': { fontFamily: 'ui-monospace, Menlo, monospace', fontSize: '0.9em', backgroundColor: 'var(--color-muted)' },
  '.cm-quote': { borderLeft: '3px solid var(--color-border)', paddingLeft: '10px !important' },
  '.cm-bullet': { display: 'inline-block', width: '1ch', color: 'var(--color-muted-foreground)' },
  '.cm-check': { margin: '0 4px 0 0', verticalAlign: '-2px', accentColor: 'var(--color-foreground)', cursor: 'pointer' },
  '.cm-rule': { display: 'inline-block', width: '100%', height: '1px', verticalAlign: 'middle', backgroundColor: 'var(--color-border)' },
  '.cm-wikilink': { textDecoration: 'underline', textUnderlineOffset: '3px', fontWeight: '600' },
  '.cm-tag': { backgroundColor: 'var(--color-muted)', borderRadius: '6px', padding: '0 4px' },
  '.cm-selectionBackground, &.cm-focused .cm-selectionBackground': { backgroundColor: 'color-mix(in oklab, var(--color-foreground) 14%, transparent) !important' },
});

const extensions = (onChange: (text: string) => void, readOnly: boolean): Extension[] => [
  history(),
  keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
  markdown({ base: markdownLanguage }),
  syntaxHighlighting(style),
  livePreview,
  theme,
  EditorView.lineWrapping,
  EditorState.readOnly.of(readOnly),
  EditorView.updateListener.of((update) => update.docChanged && onChange(update.state.doc.toString())),
];

/** Markdown that renders where you type, like Obsidian's live preview; the text is kept as written. */
export function LiveMarkdown({ value, onChange, readOnly, focus }: { value: string; onChange: (text: string) => void; readOnly: boolean; focus: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const change = useRef(onChange);
  change.current = onChange;

  useEffect(() => {
    const editor = new EditorView({ parent: host.current!, state: EditorState.create({ doc: value, extensions: extensions((text) => change.current(text), readOnly) }) });
    view.current = editor;
    return () => editor.destroy();
    // The editor is made once per read-only state; `value` flows in below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [readOnly]);

  // A note read from disk replaces what the editor shows.
  useEffect(() => {
    const editor = view.current;
    if (!editor || editor.state.doc.toString() === value) return;
    const head = Math.min(editor.state.selection.main.head, value.length);
    editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: value }, selection: { anchor: head } });
  }, [value]);

  useEffect(() => {
    if (focus) view.current?.focus();
  }, [focus, readOnly]);

  return <div ref={host} className="min-h-0 flex-1 overflow-hidden" />;
}
