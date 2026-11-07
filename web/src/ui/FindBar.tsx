import { useEffect, useRef, useState } from 'react';
import { ChevronDown, ChevronUp, Search, X } from 'lucide-react';
import type { ISearchOptions } from '@xterm/addon-search';
import type { PaneId } from '@/connection';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { searches } from '../TerminalView';
import { isDark } from './theme';

// The overview ruler only takes #RRGGBB.
const decorations = () => {
  const match = isDark() ? '#52525b' : '#e4e4e7';
  return { matchBackground: match, matchOverviewRuler: match, activeMatchBackground: '#fb923c', activeMatchBorder: '#f97316', activeMatchColorOverviewRuler: '#f97316' };
};

/** ⌘F inside a pane's head line: the line turns into the search field for that pane's output. */
export function FindBar({ pane, onClose }: { pane: PaneId; onClose: () => void }) {
  const [query, setQuery] = useState('');
  const [caseSensitive, setCase] = useState(false);
  const [regex, setRegex] = useState(false);
  const [result, setResult] = useState<{ index: number; count: number } | null>(null);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const search = searches.get(pane);
    if (!search) return;
    const sub = search.addon.onDidChangeResults(({ resultIndex, resultCount }) => setResult({ index: resultIndex, count: resultCount }));
    return () => {
      sub.dispose();
      search.addon.clearDecorations();
    };
  }, [pane]);

  const options = (incremental = false): ISearchOptions => ({ caseSensitive, regex, incremental, decorations: decorations() });
  const find = (back: boolean, incremental = false) => {
    const addon = searches.get(pane)?.addon;
    if (!addon) return;
    if (!query) {
      addon.clearDecorations();
      return setResult(null);
    }
    try {
      // The addon keeps its highlights for the same words, even after Aa or .* changed.
      if (incremental) addon.clearDecorations();
      if (back) addon.findPrevious(query, options());
      else addon.findNext(query, options(incremental));
    } catch {
      // A regex still being typed ("(") is not an error worth showing.
      setResult({ index: -1, count: 0 });
    }
  };
  // Typing searches as you go, from where the last match was.
  useEffect(() => find(false, true), [query, caseSensitive, regex]);
  useEffect(() => input.current?.focus(), []);

  const close = () => {
    onClose();
    searches.get(pane)?.focus();
  };
  const toggle = (on: boolean, label: string, text: string, set: () => void) => (
    <button
      aria-label={label}
      aria-pressed={on}
      title={label}
      onClick={() => {
        set();
        input.current?.focus();
      }}
      className={cn('h-5 rounded-md px-1.5 font-mono text-[11px] font-semibold', on ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:bg-muted')}
    >
      {text}
    </button>
  );
  return (
    <div className="flex h-10 shrink-0 items-center gap-1.5 px-3.5" onKeyDown={(event) => event.key === 'Escape' && close()}>
      <Search className="size-3.5 shrink-0 text-muted-foreground" />
      <input
        ref={input}
        value={query}
        placeholder="이 pane에서 찾기"
        onChange={(event) => setQuery(event.target.value)}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing) return;
          if (event.key === 'Enter') {
            event.preventDefault();
            find(event.shiftKey);
          }
        }}
        className="min-w-0 flex-1 bg-transparent text-[13.5px] font-medium outline-none placeholder:text-muted-foreground"
      />
      {query && result && (
        <span className={cn('shrink-0 tabular-nums', result.count ? 'text-muted-foreground' : 'text-red-500')}>
          {result.count ? `${result.index + 1}/${result.count}` : '없음'}
        </span>
      )}
      <Button variant="ghost" size="icon-xs" aria-label="이전 (⇧↩)" title="이전 (⇧↩)" onClick={() => find(true)}>
        <ChevronUp />
      </Button>
      <Button variant="ghost" size="icon-xs" aria-label="다음 (↩)" title="다음 (↩)" onClick={() => find(false)}>
        <ChevronDown />
      </Button>
      {toggle(caseSensitive, '대소문자 구분', 'Aa', () => setCase((on) => !on))}
      {toggle(regex, '정규식', '.*', () => setRegex((on) => !on))}
      <Button variant="ghost" size="icon-xs" aria-label="닫기 (esc)" title="닫기 (esc)" onClick={close}>
        <X />
      </Button>
    </div>
  );
}
