import { useCallback, useEffect, useRef, useState } from 'react';
import { ExternalLink, X } from 'lucide-react';
import type { BrainNote, BrainWritten, Connection } from '@/connection';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { fitted, grabFrame, loadFrame } from './frame';
import { LiveMarkdown } from './LiveMarkdown';

const FRAME_KEY = 'naru.brain.frame';
const MIN = { w: 320, h: 240 };
const POLL_MS = 2000;
const SAVE_MS = 500;

type Problem = { kind: 'offline' } | { kind: 'changed'; body: string; mtime: number | null } | { kind: 'error'; message: string };

/**
 * Brain: one vault note in a floating window. Edits save half a second after typing stops, and
 * only over the note last read; a change made in Obsidian meanwhile is asked about, never lost.
 */
export function Brain({ conn, onClose }: { conn: Connection; onClose: () => void }) {
  const [frame, setFrame] = useState(() => fitted(loadFrame(FRAME_KEY, () => ({ x: window.innerWidth - 440 - 80, y: 64, w: 440, h: 460 })), MIN));
  const [text, setText] = useState('');
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [problem, setProblem] = useState<Problem | null>(null);
  /** The note on disk the editor is in step with. */
  const base = useRef<number | null>(null);
  const onDisk = useRef('');
  const sent = useRef<string | null>(null);
  const live = useRef({ text, problem, loaded });
  live.current = { text, problem, loaded };

  const save = useCallback(() => {
    if (sent.current !== null || live.current.text === onDisk.current) return;
    sent.current = live.current.text;
    setSaving(true);
    conn.send({ t: 'brain_write', body: sent.current, base: base.current });
  }, [conn]);

  const take = (body: string, mtime: number | null) => {
    onDisk.current = body;
    base.current = mtime;
    setText(body);
    setLoaded(true);
    setProblem(null);
  };

  useEffect(() => {
    const onNote = (note: BrainNote) => {
      const { text, problem, loaded } = live.current;
      const dirty = text !== onDisk.current;
      switch (note.state) {
        case 'offline':
          return setProblem({ kind: 'offline' });
        case 'error':
          return setProblem({ kind: 'error', message: note.message });
        case 'unchanged':
          if (problem?.kind === 'offline') setProblem(null);
          if (dirty) save();
          return;
        case 'found':
          if (!loaded || !dirty) return take(note.body, note.mtime);
          return setProblem({ kind: 'changed', body: note.body, mtime: note.mtime });
      }
    };
    const onWritten = (written: BrainWritten) => {
      const body = sent.current;
      sent.current = null;
      setSaving(false);
      switch (written.state) {
        case 'saved':
          onDisk.current = body ?? onDisk.current;
          base.current = written.mtime;
          setProblem(null);
          if (live.current.text !== onDisk.current) setTimeout(save, SAVE_MS);
          return;
        case 'offline':
          return setProblem({ kind: 'offline' });
        case 'changed':
          return setProblem({ kind: 'changed', body: written.body, mtime: written.mtime });
        case 'error':
          return setProblem({ kind: 'error', message: written.message });
      }
    };
    const off = conn.onMessage((message) => {
      if (message.t === 'brain') onNote(message.note);
      else if (message.t === 'brain_written') onWritten(message.written);
    });
    conn.send({ t: 'brain_read', mtime: null });
    const timer = setInterval(() => conn.send({ t: 'brain_read', mtime: base.current }), POLL_MS);
    return () => {
      clearInterval(timer);
      off();
      // Closing keeps what was typed, unless the note is in question.
      const { text, problem } = live.current;
      if (!problem && sent.current === null && text !== onDisk.current) {
        conn.send({ t: 'brain_write', body: text, base: base.current });
      }
    };
  }, [conn, save]);

  useEffect(() => {
    if (!loaded || problem || text === onDisk.current) return;
    const timer = setTimeout(save, SAVE_MS);
    return () => clearTimeout(timer);
  }, [text, loaded, problem, save]);

  useEffect(() => {
    const refit = () => setFrame((now) => fitted(now, MIN));
    window.addEventListener('resize', refit);
    return () => window.removeEventListener('resize', refit);
  }, []);

  const grab = (event: React.PointerEvent, resize: boolean) => grabFrame(event, frame, resize, MIN, FRAME_KEY, setFrame);

  const status =
    problem?.kind === 'offline'
      ? 'vault 연결 끊김'
      : problem?.kind === 'changed'
        ? 'Obsidian에서 바뀜'
        : problem
          ? '확인 필요'
          : !loaded
            ? '여는 중'
            : saving || text !== onDisk.current
              ? '저장 중'
              : '저장됨';

  return (
    <section
      aria-label="Brain 노트"
      onKeyDown={(event) => event.key === 'Escape' && !event.nativeEvent.isComposing && onClose()}
      className="fixed z-30 flex flex-col overflow-hidden rounded-2xl bg-background shadow-[0_12px_40px_rgba(9,9,11,0.18)] animate-in fade-in zoom-in-95"
      style={{ left: frame.x, top: frame.y, width: frame.w, height: frame.h }}
    >
      <header onPointerDown={(event) => grab(event, false)} className="flex h-10 shrink-0 cursor-grab items-center gap-2 pr-2 pl-4 select-none active:cursor-grabbing">
        <b className="text-[14px] font-bold">Brain</b>
        <span className={cn('text-[12px]', problem ? 'font-semibold text-turn' : 'text-muted-foreground')}>{status}</span>
        <Button variant="ghost" size="icon-xs" className="ml-auto" aria-label="Obsidian에서 열기" title="Obsidian에서 열기" onClick={() => conn.send({ t: 'brain_open' })}>
          <ExternalLink />
        </Button>
        <Button variant="ghost" size="icon-xs" aria-label="닫기" onClick={onClose}>
          <X />
        </Button>
      </header>
      {problem && (
        <div className="mx-3 mb-2 flex items-center gap-2 rounded-xl bg-muted px-3 py-2 text-[12.5px]">
          <span className="min-w-0 flex-1">
            {problem.kind === 'offline'
              ? 'vault에 연결되지 않았습니다. 다시 연결되면 저장합니다.'
              : problem.kind === 'changed'
                ? 'Obsidian에서 바뀌었습니다.'
                : problem.message}
          </span>
          {problem.kind === 'changed' && (
            <>
              <Button size="xs" variant="secondary" className="bg-background" onClick={() => take(problem.body, problem.mtime)}>
                그쪽 불러오기
              </Button>
              <Button
                size="xs"
                onClick={() => {
                  base.current = problem.mtime;
                  setProblem(null);
                  save();
                }}
              >
                내 것 저장
              </Button>
            </>
          )}
        </div>
      )}
      {/* Opening Brain is for writing in it: the keyboard leaves the terminal once the note is in, and
          comes back to the note when the button that held it goes away with a settled problem. */}
      <LiveMarkdown value={text} onChange={setText} readOnly={!loaded} focus={loaded && !problem} />
      <div onPointerDown={(event) => grab(event, true)} className="absolute right-0 bottom-0 size-4 cursor-nwse-resize" aria-hidden />
    </section>
  );
}
