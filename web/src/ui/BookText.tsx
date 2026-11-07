import { createContext, useContext, useLayoutEffect, useRef, useState } from 'react';
import Markdown, { type Components, type Options } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { BookMark, Figure } from '@/connection';
import { cn } from '@/lib/utils';
import { type TextAnchor, anchorAt, blockOf, locate, offsetAt, rangeAt, textOf } from './anchor';

/** What the reader dragged over: where it is and what is around it. */
export interface Selected {
  anchor: TextAnchor;
  /** The block holding the selection's end, which a figure follows. */
  block: number;
  context: string;
  /** Underlines it touches. */
  marks: string[];
  top: number;
  bottom: number;
  left: number;
}

interface Props {
  text: string;
  marks: BookMark[];
  figures: Figure[];
  /** Body text size in px. */
  size: number;
  onMark: (anchor: TextAnchor) => void;
  onUnmark: (marks: string[]) => void;
  onDraw: (selected: Selected, prompt: string) => void;
  onAsk: (selected: Selected, question: string) => void;
  onRedraw: (figure: string) => void;
  onUndraw: (figure: string) => void;
}

const CHIPS = ['구조도', '흐름도', '비교표', '쉽게 풀어 설명'];
const TOOLS_WIDTH = 340;

type HNode = { type: string; tagName?: string; value?: string; properties?: Record<string, unknown>; children?: HNode[] };

const hastText = (node: HNode): string => (node.type === 'text' ? (node.value ?? '') : (node.children ?? []).map(hastText).join(''));

/** Numbers the page's top-level blocks and puts each figure after its block. */
function placeFigures(figures: Figure[]): NonNullable<Options['rehypePlugins']>[number] {
  return () => (tree: HNode) => {
    const children = tree.children ?? [];
    const blocks = children.filter((child) => child.type === 'element');
    blocks.forEach((block, index) => (block.properties = { ...block.properties, dataBlock: index }));
    const texts = blocks.map(hastText);
    const after = new Map<HNode, HNode[]>();
    for (const figure of figures) {
      const at = blockOf(texts, figure);
      if (at < 0) continue;
      after.set(blocks[at], [...(after.get(blocks[at]) ?? []), { type: 'element', tagName: 'naru-figure', properties: { figure: figure.id }, children: [] }]);
    }
    tree.children = children.flatMap((child) => [child, ...(after.get(child) ?? [])]);
  };
}

const FigureContext = createContext<{ figures: Figure[]; onRedraw: (figure: string) => void; onUndraw: (figure: string) => void }>({
  figures: [],
  onRedraw: () => {},
  onUndraw: () => {},
});

const spinner = 'size-3 shrink-0 animate-spin rounded-full border-2 border-current border-t-transparent';

function FigureSlot({ figure: id }: { figure?: string }) {
  const { figures, onRedraw, onUndraw } = useContext(FigureContext);
  const figure = figures.find((each) => each.id === id);
  if (!figure) return null;
  return (
    <div data-figure className="not-prose my-5 font-sans select-none">
      {figure.status === 'done' && figure.svg ? (
        <figure className="overflow-hidden rounded-xl bg-[#fbfaf7] p-3 ring-1 ring-black/5">
          <img src={`data:image/svg+xml;base64,${figure.svg}`} alt={figure.prompt} draggable={false} className="mx-auto max-w-full" />
        </figure>
      ) : figure.status === 'drawing' ? (
        <div className="flex h-20 items-center justify-center gap-2 rounded-xl border-[1.5px] border-dashed border-border bg-muted/40 text-[12.5px] text-muted-foreground">
          <i className={spinner} />
          그림 그리는 중 · 1~3분
        </div>
      ) : (
        <div className="rounded-xl border-[1.5px] border-dashed border-destructive/40 px-4 py-3 text-[12.5px] text-destructive">그리지 못했어요. {figure.error}</div>
      )}
      <div className="mt-1.5 flex items-center gap-3 px-1 text-[11.5px] text-muted-foreground">
        <span className="min-w-0 flex-1 truncate">
          <b className="font-semibold text-foreground">그림</b> · {figure.prompt}
        </span>
        {figure.status !== 'drawing' && (
          <button className="hover:text-foreground" onClick={() => onRedraw(figure.id)}>
            다시 그리기
          </button>
        )}
        <button className="hover:text-foreground" onClick={() => onUndraw(figure.id)}>
          빼기
        </button>
      </div>
    </div>
  );
}

const components = { 'naru-figure': FigureSlot } as Components;

/**
 * A translated page in 마루 부리, with the reader's underlines as a highlighter band and figures
 * between its blocks. Dragging over text brings up 밑줄 · 그림으로 · 묻기 above it.
 */
export function BookText({ text, marks, figures, size, onMark, onUnmark, onDraw, onAsk, onRedraw, onUndraw }: Props) {
  const [selected, setSelected] = useState<Selected | null>(null);
  const [mode, setMode] = useState<'draw' | 'ask' | null>(null);
  const [typed, setTyped] = useState('');
  const wrap = useRef<HTMLDivElement>(null);
  const body = useRef<HTMLDivElement>(null);

  // Highlights paint over the text without touching the DOM React renders.
  useLayoutEffect(() => {
    const root = body.current;
    if (!root || !('highlights' in CSS)) return;
    const flat = textOf(root);
    const ranges = marks.flatMap((mark) => {
      const at = locate(flat, mark);
      const range = at && rangeAt(root, ...at);
      return range ? [range] : [];
    });
    CSS.highlights.set('naru-marks', new Highlight(...ranges));
    return () => void CSS.highlights.delete('naru-marks');
  });

  const close = () => {
    setSelected(null);
    setMode(null);
    setTyped('');
    window.getSelection()?.removeAllRanges();
  };

  const pick = (event: React.MouseEvent) => {
    if ((event.target as Element).closest('[data-tools]')) return;
    const selection = window.getSelection();
    const root = body.current;
    const box = wrap.current;
    if (!selection || selection.isCollapsed || !selection.rangeCount || !root || !box) return;
    const range = selection.getRangeAt(0);
    if (!root.contains(range.commonAncestorContainer)) return;
    const flat = textOf(root);
    const start = offsetAt(root, range.startContainer, range.startOffset);
    const end = offsetAt(root, range.endContainer, range.endOffset);
    if (!flat.slice(start, end).trim()) return;
    // The last selected character's block: a selection ending at the next block's start belongs before it.
    const last = rangeAt(root, end - 1, end)?.startContainer.parentElement?.closest('[data-block]');
    const rect = range.getBoundingClientRect();
    const origin = box.getBoundingClientRect();
    setMode(null);
    setTyped('');
    setSelected({
      anchor: anchorAt(flat, start, end),
      block: Number(last?.getAttribute('data-block') ?? 0),
      context: (last?.textContent ?? flat.slice(start, end)).trim(),
      marks: marks.filter((mark) => {
        const at = locate(flat, mark);
        return at !== null && at[0] < end && at[1] > start;
      }).map((mark) => mark.id),
      top: rect.top - origin.top,
      bottom: rect.bottom - origin.top,
      left: Math.min(Math.max(rect.left - origin.left, 0), Math.max(origin.width - TOOLS_WIDTH, 0)),
    });
  };

  const submit = (chip?: string) => {
    if (!selected || !mode) return;
    const words = typed.trim();
    const request = chip ? (words ? `${chip}: ${words}` : chip) : words;
    if (!request) return;
    if (mode === 'draw') onDraw(selected, request);
    else onAsk(selected, request);
    close();
  };

  const tool = (on: boolean) => cn('rounded-lg px-2.5 py-1.5 hover:bg-background/15', on && 'bg-background/20');

  return (
    <div
      ref={wrap}
      className="relative"
      onMouseUp={pick}
      onMouseDown={(event) => !(event.target as Element).closest('[data-tools]') && selected && setSelected(null)}
      onKeyDown={(event) => event.key === 'Escape' && close()}
    >
      <FigureContext.Provider value={{ figures, onRedraw, onUndraw }}>
        <div
          ref={body}
          className="prose prose-zinc max-w-none font-book leading-[1.95] dark:prose-invert prose-headings:font-book prose-headings:font-bold prose-p:my-4"
          style={{ fontSize: size }}
        >
          <Markdown remarkPlugins={[remarkGfm]} rehypePlugins={[placeFigures(figures)]} components={components}>
            {text}
          </Markdown>
        </div>
      </FigureContext.Provider>
      {selected && (
        <>
          <div
            data-tools
            className="absolute z-10 flex gap-0.5 rounded-xl bg-foreground p-1 text-[12.5px] font-semibold text-background shadow-lg"
            style={{ left: selected.left, top: Math.max(selected.top - 42, 0) }}
          >
            {selected.marks.length ? (
              <button className={tool(false)} onClick={() => (onUnmark(selected.marks), close())}>
                밑줄 지우기
              </button>
            ) : (
              <button className={tool(false)} onClick={() => (onMark(selected.anchor), close())}>
                밑줄
              </button>
            )}
            <button className={tool(mode === 'draw')} onClick={() => setMode('draw')}>
              그림으로
            </button>
            <button className={tool(mode === 'ask')} onClick={() => setMode('ask')}>
              묻기
            </button>
          </div>
          {mode && (
            <div
              data-tools
              className="absolute z-10 rounded-2xl bg-foreground p-2.5 text-background shadow-lg"
              style={{ left: selected.left, top: selected.bottom + 8, width: TOOLS_WIDTH }}
            >
              <input
                autoFocus
                value={typed}
                onChange={(event) => setTyped(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter' && !event.nativeEvent.isComposing) submit();
                  if (event.key === 'Escape') close();
                }}
                placeholder={mode === 'draw' ? '어떤 그림으로 그릴까요?' : '이 부분에 대해 무엇을 물어볼까요?'}
                className="w-full rounded-lg border border-background/20 bg-transparent px-3 py-2 text-[13px] outline-none placeholder:text-background/50"
              />
              {mode === 'draw' && (
                <div className="mt-2 flex flex-wrap gap-1">
                  {CHIPS.map((chip) => (
                    <button key={chip} onClick={() => submit(chip)} className="rounded-md bg-background/10 px-2 py-1 text-[11.5px] font-semibold hover:bg-background/20">
                      {chip}
                    </button>
                  ))}
                </div>
              )}
            </div>
          )}
        </>
      )}
    </div>
  );
}
