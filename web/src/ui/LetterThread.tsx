import { useEffect, useRef, useState } from 'react';
import { X } from 'lucide-react';
import type { Connection, Letter, PaneId } from '@/connection';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { type End, KIND_LABEL, ME, type Pair, failed, stateOf } from './letters';

const clock = (at: number) => {
  const date = new Date(at * 1000);
  const time = date.toLocaleTimeString('ko-KR', { hour: '2-digit', minute: '2-digit', hour12: false });
  return date.toDateString() === new Date().toDateString() ? time : `${date.getMonth() + 1}/${date.getDate()} ${time}`;
};

const TONE = { muted: 'text-muted-foreground', wait: 'text-foreground/80', fail: 'text-destructive' };

/** The letters two ends wrote each other, oldest first, and a box to write either agent as "나". */
export function LetterThread({
  conn,
  pair,
  letters,
  nameOf,
  titleOf,
  onClose,
}: {
  conn: Connection;
  pair: Pair;
  letters: Letter[];
  nameOf: (end: End) => string;
  titleOf: (end: End) => string;
  onClose: () => void;
}) {
  const agents = [pair.a, pair.b].filter((end): end is PaneId => end !== ME);
  const [to, setTo] = useState(agents[0]);
  const [text, setText] = useState('');
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => {
    list.current?.scrollTo({ top: list.current.scrollHeight });
  }, [letters.length]);

  const now = Date.now() / 1000;
  const send = () => {
    const body = text.trim();
    if (!body || !to) return;
    conn.send({ t: 'letter_send', to, text: body });
    setText('');
  };
  const end = (which: End) => (
    <span className="flex min-w-0 items-center gap-1">
      <code className="shrink-0 rounded-md bg-muted px-1 text-[11px]">{nameOf(which)}</code>
      <b className="truncate text-[12.5px] font-semibold">{titleOf(which)}</b>
    </span>
  );

  return (
    <aside aria-label="두 세션의 편지" className="flex w-[260px] shrink-0 flex-col pr-2.5 pb-3 pl-3.5">
      <header className="flex items-center gap-1.5 border-b pb-2">
        {end(pair.a)}
        <span className="shrink-0 text-muted-foreground">↔</span>
        {end(pair.b)}
        <Button variant="ghost" size="icon-xs" aria-label="편지 닫기" className="ml-auto shrink-0" onClick={onClose}>
          <X />
        </Button>
      </header>
      <div ref={list} className="min-h-0 flex-1 overflow-y-auto">
        {letters.length === 0 && <p className="py-3 text-[12px] text-muted-foreground">아직 오간 편지가 없어요</p>}
        {letters.map((letter) => {
          const state = stateOf(letter, now);
          return (
            <article key={letter.id} className="border-b py-2.5">
              <div className="flex items-baseline gap-1.5 text-[11.5px] text-muted-foreground">
                <code className="rounded-md bg-muted px-1">l{letter.id}</code>
                <span className="truncate">
                  {letter.from_name} → {letter.to_name}
                </span>
                <b className="shrink-0 font-semibold text-foreground">{KIND_LABEL[letter.kind]}</b>
                <span className="ml-auto shrink-0">{clock(letter.at)}</span>
              </div>
              {letter.reply_to !== null && <div className="mt-0.5 text-[11.5px] text-muted-foreground">l{letter.reply_to}에 답</div>}
              <p className="mt-1 text-[13px] leading-[1.55] break-words whitespace-pre-wrap">{letter.text}</p>
              <div className={cn('mt-1 text-[11.5px]', TONE[state.tone])}>{state.text}</div>
              {failed(letter) && (
                <Button variant="outline" size="xs" className="mt-1.5" onClick={(event) => {
                    conn.send({ t: 'letter_retry', id: letter.id });
                    // The button goes once the letter is back in line; keep Esc working.
                    event.currentTarget.closest<HTMLElement>('section')?.focus();
                  }}>
                  다시 보내기
                </Button>
              )}
            </article>
          );
        })}
      </div>
      {to && (
        <footer className="border-t pt-2">
          <label className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
            나로 보내기 · 받는 사람
            <select value={to} onChange={(event) => setTo(event.target.value)} className="rounded-md bg-muted px-1 py-0.5 text-foreground">
              {agents.map((agent) => (
                <option key={agent} value={agent}>
                  {nameOf(agent)}
                </option>
              ))}
            </select>
          </label>
          <div className="mt-1.5 flex items-end gap-1.5">
            <textarea
              value={text}
              rows={2}
              onChange={(event) => setText(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
                  event.preventDefault();
                  send();
                }
              }}
              placeholder="편지 내용"
              className="min-w-0 flex-1 resize-none rounded-lg bg-muted px-2.5 py-1.5 text-[12.5px] outline-none"
            />
            <Button size="sm" onClick={send} disabled={!text.trim()}>
              보내기
            </Button>
          </div>
        </footer>
      )}
    </aside>
  );
}
