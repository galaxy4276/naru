import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { AArrowDown, AArrowUp, BookOpen, ChevronLeft, ChevronRight, NotebookPen, PanelRightClose, PanelRightOpen, Plus, RotateCw } from 'lucide-react';
import { ContextMenu as Menu } from 'radix-ui';
import type { Book, BookMark, Connection, Figure, PaneId, Workspace } from '@/connection';
import { TerminalView } from '@/TerminalView';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { cn } from '@/lib/utils';
import { BookText } from './BookText';
import { agoLabel } from './model';
import { menuItem } from './ProjectRail';
import { type Pdf, allText, base64Of, coverOf, drawPage, loadPdf, pageText } from './pdf';
import { PDF, type Picked, pickFiles } from './pick';
import { getStored, setStored } from './stored';

const MAX_BOOK = 150 * 1024 * 1024;
const MODE_KEY = 'naru.library.view';
const DISCUSS_KEY = 'naru.library.discuss';
const TEXT_KEY = 'naru.library.textSize';
const TEXT_SIZES = { min: 14, max: 26, initial: 18 };

const MODES = ['side', 'original', 'translation'] as const;
type Mode = (typeof MODES)[number];
const MODE_LABEL: Record<Mode, string> = { side: '나란히', original: '원문', translation: '번역' };

const textSizeOf = (value: number) => Math.min(Math.max(Math.round(value), TEXT_SIZES.min), TEXT_SIZES.max);
type Translation = { text: string } | { error: string } | { pending: true };

// The book the reader had open, so leaving the 서재 and coming back lands on it again.
let lastReading: string | null = null;

const spinner = 'size-3 shrink-0 animate-spin rounded-full border-2 border-current border-t-transparent';

/** 시안 A: the shelf (이어 읽기 and a grid of covers), and a reader with the book's session beside it. */
export function Library({ conn, ws, now, visible }: { conn: Connection; ws: Workspace; now: number; visible: boolean }) {
  const [books, setBooks] = useState<Book[] | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [adding, setAdding] = useState(0);
  const [reading, setReading] = useState<string | null>(lastReading);
  const [query, setQuery] = useState('');
  const [removing, setRemoving] = useState<Book | null>(null);

  useEffect(() => {
    const off = conn.onMessage((message) => {
      if (message.t === 'library') {
        setBooks(message.books);
        if (message.error) setFailure(message.error);
        setAdding((count) => Math.max(0, count - 1));
      } else if (message.t === 'library_failure') {
        setFailure(message.message);
        setAdding((count) => Math.max(0, count - 1));
      } else if (message.t === 'library_progress') {
        setBooks((all) => all && all.map((book) => (book.id === message.book ? { ...book, translated: message.translated } : book)));
      }
    });
    conn.send({ t: 'read_library' });
    return off;
  }, [conn]);

  const read = (book: string | null) => {
    lastReading = book;
    setReading(book);
    setFailure(null);
    if (!book) conn.send({ t: 'read_library' });
  };
  const add = async (files: Picked[]) => {
    for (const file of files) {
      setAdding((count) => count + 1);
      try {
        const blob = await file.blob();
        if (blob.size > MAX_BOOK) {
          setAdding((count) => Math.max(0, count - 1));
          setFailure(`${file.name}: 150MB보다 커서 넣지 못해요`);
          continue;
        }
        const { pdf, close } = await loadPdf(new Uint8Array(await blob.arrayBuffer()));
        const pages = pdf.numPages;
        const [cover, texts] = await Promise.all([coverOf(pdf), allText(pdf)]).finally(close);
        const source = file.path ? { path: file.path } : { pdf: await base64Of(blob) };
        conn.send({ t: 'library_add', name: file.name, pages, cover, texts, ...source });
      } catch (error) {
        setAdding((count) => Math.max(0, count - 1));
        setFailure(`${file.name}: PDF를 열지 못했어요 (${error instanceof Error ? error.message : String(error)})`);
      }
    }
  };

  const open = reading && books?.find((book) => book.id === reading);
  if (open) {
    return (
      <Reader
        key={open.id}
        conn={conn}
        visible={visible}
        ws={ws}
        book={open}
        onBack={() => read(null)}
        onPage={(page) => setBooks((all) => all && all.map((book) => (book.id === open.id ? { ...book, page, opened: Date.now() } : book)))}
        onTexts={(translatable) => setBooks((all) => all && all.map((book) => (book.id === open.id ? { ...book, translatable } : book)))}
      />
    );
  }

  const sorted = [...(books ?? [])].sort((a, b) => (b.opened ?? b.added) - (a.opened ?? a.added));
  const needle = query.trim().toLowerCase();
  const shown = sorted.filter((book) => book.title.toLowerCase().includes(needle));
  const current = !needle && sorted.find((book) => book.opened !== null);
  const pick = () => {
    pickFiles(PDF, true).then(add, (error) => setFailure(`파일을 고르지 못했어요: ${error instanceof Error ? error.message : String(error)}`));
  };

  return (
    <main className="flex min-w-0 flex-1 flex-col gap-2 py-2 pr-2">
      <header className="flex items-center gap-2 rounded-2xl bg-background px-4 py-2.5">
        <b className="text-[15px] font-semibold">서재</b>
        <span className="text-muted-foreground">{books?.length ?? 0}권</span>
        {adding > 0 && (
          <span className="flex items-center gap-1.5 text-muted-foreground">
            <i className={spinner} />넣는 중
          </span>
        )}
        <span className="flex-1" />
        <Input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="제목 찾기" className="h-8 w-48" />
        <Button size="sm" onClick={pick}>
          <Plus />
          PDF 넣기
        </Button>
      </header>
      {failure && <Failure text={failure} onClose={() => setFailure(null)} />}
      {current && (
        <section className="flex items-center gap-4 rounded-2xl bg-background px-4 py-3">
          <Cover book={current} className="w-14" small />
          <div className="min-w-0 flex-1">
            <div className="text-muted-foreground">이어 읽기 · {agoLabel(current.opened, now)}</div>
            <b className="block truncate text-[15px] font-semibold">{current.title}</b>
            <div className="mt-0.5 text-muted-foreground">
              {current.page} / {current.pages}쪽{current.note_exists ? ' · 노트 있음' : ''}
              {translating(current) === 'running' && ` · 번역 ${current.translated}/${current.translatable}`}
            </div>
            <Progress book={current} className="mt-1.5 w-64" />
          </div>
          <Button onClick={() => read(current.id)}>{current.page}쪽부터 읽기</Button>
        </section>
      )}
      <section className="min-h-0 flex-1 overflow-y-auto rounded-2xl bg-background p-5">
        {books && books.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-2 text-muted-foreground">
            <span className="mb-1 grid size-11 place-items-center rounded-xl bg-muted text-foreground">
              <BookOpen className="size-5" />
            </span>
            <b className="text-[15px] font-semibold text-foreground">PDF를 넣어 서재를 채워 보세요</b>
            <span>마지막으로 본 쪽에서 다시 열리고, 책마다 claude와 논의한 내용이 노트에 쌓여요</span>
            <Button className="mt-2" onClick={pick}>
              PDF 넣기
            </Button>
          </div>
        ) : (
          <div className="grid grid-cols-[repeat(auto-fill,minmax(128px,1fr))] gap-x-5 gap-y-6">
            {shown.map((book) => (
              <Menu.Root key={book.id}>
                <Menu.Trigger asChild>
                  <button className="group min-w-0 text-left" onClick={() => read(book.id)}>
                    <Cover book={book} className="w-full transition group-hover:-translate-y-0.5 group-hover:shadow-md" />
                    <Progress book={book} className="mt-2" />
                    <b className="mt-1.5 block truncate text-[12px] font-medium">{book.title}</b>
                    <span className="block text-[11px] text-muted-foreground">{progressLabel(book)}</span>
                  </button>
                </Menu.Trigger>
                <Menu.Portal>
                  <Menu.Content className="z-50 min-w-40 rounded-xl bg-popover p-1 text-popover-foreground shadow-lg ring-1 ring-border">
                    <Menu.Item className={menuItem} onSelect={() => read(book.id)}>
                      읽기
                    </Menu.Item>
                    <Menu.Item className={menuItem} onSelect={() => setRemoving(book)}>
                      서재에서 빼기…
                    </Menu.Item>
                  </Menu.Content>
                </Menu.Portal>
              </Menu.Root>
            ))}
          </div>
        )}
      </section>
      <Dialog open={removing !== null} onOpenChange={(opened) => !opened && setRemoving(null)}>
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>서재에서 빼기</DialogTitle>
          </DialogHeader>
          <p className="mt-2 text-muted-foreground">
            {removing?.title}의 Naru 사본과 읽은 위치, 번역을 지워요. 원래 PDF 파일과 Obsidian 노트는 그대로 남아요.
          </p>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setRemoving(null)}>
              취소
            </Button>
            <Button
              variant="destructive"
              onClick={() => {
                if (removing) conn.send({ t: 'library_remove', book: removing.id });
                setRemoving(null);
              }}
            >
              빼기
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </main>
  );
}

function progressLabel(book: Book): string {
  const read = book.opened === null ? '안 읽음' : book.page >= book.pages ? '다 읽음' : `${book.page} / ${book.pages}쪽`;
  return translating(book) === 'done' ? `${read} · 번역 완료` : read;
}

/** Where the background translation of the book is. */
function translating(book: Book): 'unknown' | 'running' | 'done' {
  if (book.translatable === null) return 'unknown';
  return book.translated >= book.translatable ? 'done' : 'running';
}

/** `small` covers leave the translation badge to the text beside them. */
function Cover({ book, className, small }: { book: Book; className?: string; small?: boolean }) {
  return (
    <div className={cn('relative', className)}>
      {book.cover ? (
        <img
          src={`data:image/jpeg;base64,${book.cover}`}
          alt=""
          draggable={false}
          className="aspect-[2/3] w-full rounded-md object-cover object-top shadow-sm ring-1 ring-black/5"
        />
      ) : (
        <div className="grid aspect-[2/3] w-full place-items-center rounded-md bg-muted p-2 text-center text-[11px] font-semibold">{book.title}</div>
      )}
      {!small && translating(book) === 'running' && (
        <span className="absolute bottom-1.5 left-1.5 flex items-center gap-1 rounded-[5px] bg-zinc-950/80 px-1.5 py-0.5 text-[10px] font-semibold text-white tabular-nums">
          <i className="size-2 shrink-0 animate-spin rounded-full border-[1.5px] border-current border-t-transparent" />
          번역 {book.translated}/{book.translatable}
        </span>
      )}
    </div>
  );
}

function Progress({ book, className }: { book: Book; className?: string }) {
  const read = book.opened === null ? 0 : book.page / book.pages;
  return (
    <div className={cn('h-1 overflow-hidden rounded-full bg-muted', className)}>
      <div className="h-full rounded-full bg-primary" style={{ width: `${Math.round(read * 100)}%` }} />
    </div>
  );
}

function Failure({ text, onClose }: { text: string; onClose: () => void }) {
  return (
    <div className="flex items-center gap-2 rounded-2xl bg-background px-4 py-2 text-destructive">
      <span className="min-w-0 flex-1">{text}</span>
      <Button variant="ghost" size="xs" onClick={onClose}>
        닫기
      </Button>
    </div>
  );
}

interface ReaderProps {
  conn: Connection;
  /** The 서재 is on screen; hidden, it leaves the arrow keys to the rest of Naru. */
  visible: boolean;
  ws: Workspace;
  book: Book;
  onBack: () => void;
  onPage: (page: number) => void;
  /** The book's text was sent, with this many pages to translate. */
  onTexts: (translatable: number) => void;
}

function Reader({ conn, visible, ws, book, onBack, onPage, onTexts }: ReaderProps) {
  const [pdf, setPdf] = useState<Pdf | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [page, setPage] = useState(Math.min(Math.max(book.page, 1), book.pages));
  const [typed, setTyped] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>(() => MODES.find((each) => each === getStored(MODE_KEY)) ?? 'side');
  const [discuss, setDiscuss] = useState(() => getStored(DISCUSS_KEY) === 'open');
  const [textSize, setTextSize] = useState(() => textSizeOf(Number(getStored(TEXT_KEY)) || TEXT_SIZES.initial));
  const [translations, setTranslations] = useState<Map<number, Translation>>(() => new Map());
  const [started, setStarted] = useState<PaneId | null>(null);
  const [starting, setStarting] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const [box, setBox] = useState<{ width: number; height: number } | null>(null);
  const [marks, setMarks] = useState<{ marks: BookMark[]; figures: Figure[] }>({ marks: [], figures: [] });
  const host = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let alive = true;
    let close: (() => Promise<void>) | null = null;
    loadPdf(conn.bookUrl(book.id)).then(
      (opened) => {
        if (!alive) return void opened.close();
        close = opened.close;
        setPdf(opened.pdf);
      },
      (error) => alive && setLoadError(`PDF를 열지 못했어요: ${error instanceof Error ? error.message : String(error)}`),
    );
    const off = conn.onMessage((message) => {
      if (message.t === 'library_translation' && message.book === book.id) {
        const entry: Translation = message.text !== null ? { text: message.text } : { error: message.error ?? '번역하지 못했어요' };
        setTranslations((all) => new Map(all).set(message.page, entry));
      } else if (message.t === 'library_marks' && message.book === book.id) {
        setMarks({ marks: message.marks, figures: message.figures });
      } else if (message.t === 'library_discussion' && message.book === book.id) {
        setStarted(message.pane);
        setStarting(false);
      } else if (message.t === 'library_failure') {
        setStarting(false);
        setFailure(message.message);
      }
    });
    conn.send({ t: 'library_open', book: book.id });
    conn.send({ t: 'read_library_marks', book: book.id });
    return () => {
      alive = false;
      off();
      void close?.();
    };
  }, [conn, book.id]);

  useLayoutEffect(() => {
    const element = host.current;
    if (!element) return;
    const observer = new ResizeObserver(([entry]) => setBox({ width: entry.contentRect.width, height: entry.contentRect.height }));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const element = host.current;
    if (!pdf || !element || !box || box.width < 20 || box.height < 20) return;
    const drawing = drawPage(pdf, page, box);
    drawing.done.then(
      (canvas) => {
        canvas.className = 'rounded-sm shadow-[0_2px_14px_rgba(0,0,0,0.12)]';
        element.replaceChildren(canvas);
      },
      (error) => {
        if (error?.name !== 'RenderingCancelledException') setFailure(`${page}쪽을 그리지 못했어요: ${error instanceof Error ? error.message : String(error)}`);
      },
    );
    return () => drawing.cancel();
  }, [pdf, page, box]);

  // A book shelved before its text was kept sends it now, so the background translation can start.
  const textsSent = useRef(false);
  useEffect(() => {
    if (!pdf || book.translatable !== null || textsSent.current) return;
    textsSent.current = true;
    allText(pdf).then(
      (texts) => {
        conn.send({ t: 'library_texts', book: book.id, texts });
        onTexts(texts.filter((text) => text.trim()).length);
      },
      (error) => setFailure(`책의 글자를 읽지 못했어요: ${error instanceof Error ? error.message : String(error)}`),
    );
  }, [pdf, book.translatable]);

  const translate = async (number: number, again: boolean) => {
    if (!pdf || number < 1 || number > book.pages) return;
    setTranslations((all) => new Map(all).set(number, { pending: true }));
    try {
      conn.send({ t: 'library_translate', book: book.id, page: number, text: await pageText(pdf, number), again });
    } catch (error) {
      const message = `${number}쪽 글자를 읽지 못했어요: ${error instanceof Error ? error.message : String(error)}`;
      setTranslations((all) => new Map(all).set(number, { error: message }));
    }
  };
  // The next page is asked for too, so turning the page does not wait for claude.
  useEffect(() => {
    if (mode === 'original' || !pdf) return;
    for (const number of [page, page + 1]) if (!translations.has(number)) void translate(number, false);
  }, [mode, pdf, page]);

  const go = (next: number) => {
    const number = Math.min(Math.max(next, 1), book.pages);
    if (number === page) return;
    setPage(number);
    onPage(number);
    conn.send({ t: 'library_page', book: book.id, page: number });
  };
  const switchMode = (next: Mode) => {
    setMode(next);
    setStored(MODE_KEY, next);
  };
  const toggleDiscuss = () => {
    setStored(DISCUSS_KEY, discuss ? null : 'open');
    setDiscuss(!discuss);
  };
  const resize = (step: number) => {
    const next = textSizeOf(textSize + step);
    setStored(TEXT_KEY, String(next));
    setTextSize(next);
  };
  const pageMarks = marks.marks.filter((mark) => mark.page === page);
  const pageFigures = marks.figures.filter((figure) => figure.page === page);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!visible || event.metaKey || event.ctrlKey || event.altKey || event.isComposing) return;
      if ((event.target as HTMLElement | null)?.closest('input, textarea, [contenteditable], .xterm')) return;
      if (event.key === 'ArrowRight' || event.key === 'PageDown') go(page + 1);
      else if (event.key === 'ArrowLeft' || event.key === 'PageUp') go(page - 1);
      else if (event.key === 't' || event.key === 'T' || event.key === 'ㅅ') switchMode(MODES[(MODES.indexOf(mode) + 1) % MODES.length]);
      else return;
      event.preventDefault();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  const sessionPane = book.session ? Object.keys(ws.panes).find((pane) => ws.panes[pane].session?.session_id === book.session) : undefined;
  const pane = (started && ws.panes[started] ? started : null) ?? sessionPane ?? null;
  const entry = translations.get(page);
  const opening = (
    <p className={cn('absolute inset-0 flex items-center justify-center gap-2', loadError ? 'text-destructive' : 'text-muted-foreground')}>
      {loadError ?? (
        <>
          <i className={spinner} />
          책을 여는 중
        </>
      )}
    </p>
  );

  return (
    <main className="flex min-w-0 flex-1 flex-col gap-2 py-2 pr-2">
      <header className="flex items-center gap-2 rounded-2xl bg-background px-3 py-2">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ChevronLeft />
          서재
        </Button>
        <b className="min-w-0 truncate text-[14px] font-semibold">{book.title}</b>
        <span className="flex-1" />
        {translating(book) === 'running' && (
          <span className="flex items-center gap-2 text-[12px] text-muted-foreground tabular-nums" title="책 전체를 뒤에서 번역하고 있어요">
            번역 {book.translated}/{book.translatable}
            <span className="h-1 w-16 overflow-hidden rounded-full bg-muted">
              <span className="block h-full rounded-full bg-primary" style={{ width: `${Math.round((book.translated / (book.translatable || 1)) * 100)}%` }} />
            </span>
          </span>
        )}
        <div className="flex rounded-lg bg-muted p-0.5" title="T 키로 바꾸기">
          {MODES.map((each) => (
            <button
              key={each}
              onClick={() => switchMode(each)}
              className={cn('rounded-md px-2.5 py-1 text-[12px]', mode === each ? 'bg-background font-semibold shadow-sm' : 'text-muted-foreground')}
            >
              {MODE_LABEL[each]}
            </button>
          ))}
        </div>
        <div className="flex items-center gap-1">
          <Button variant="ghost" size="icon-sm" aria-label="이전 쪽" disabled={page <= 1} onClick={() => go(page - 1)}>
            <ChevronLeft />
          </Button>
          <input
            aria-label="쪽"
            value={typed ?? String(page)}
            onChange={(event) => setTyped(event.target.value)}
            onBlur={() => setTyped(null)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && typed !== null && /^\d+$/.test(typed.trim())) go(Number(typed.trim()));
              if (event.key === 'Enter' || event.key === 'Escape') event.currentTarget.blur();
            }}
            className="w-11 rounded-md bg-muted px-1 py-0.5 text-center tabular-nums outline-none"
          />
          <span className="text-muted-foreground tabular-nums">/ {book.pages}</span>
          <Button variant="ghost" size="icon-sm" aria-label="다음 쪽" disabled={page >= book.pages} onClick={() => go(page + 1)}>
            <ChevronRight />
          </Button>
        </div>
        <Button variant="ghost" size="sm" onClick={toggleDiscuss}>
          {discuss ? <PanelRightClose /> : <PanelRightOpen />}
          {discuss ? '논의 접기' : '논의 펴기'}
        </Button>
      </header>
      {failure && <Failure text={failure} onClose={() => setFailure(null)} />}
      <div className="relative flex min-h-0 flex-1 gap-2 overflow-hidden">
        <section className={cn('relative min-w-0 flex-1 overflow-hidden rounded-2xl bg-background', mode === 'translation' && 'hidden')}>
          <div ref={host} className="absolute inset-5 grid place-items-center" />
          {!pdf && opening}
        </section>
        <section className={cn('relative min-w-0 flex-1 overflow-hidden rounded-2xl bg-background', mode === 'original' && 'hidden')}>
          {pdf ? (
            <div className="absolute inset-0 flex flex-col">
              <article className="min-h-0 flex-1 overflow-y-auto px-8 py-6">
                <div className="mx-auto max-w-[680px]">
                  {entry && 'text' in entry ? (
                    <BookText
                      key={page}
                      text={entry.text}
                      marks={pageMarks}
                      figures={pageFigures}
                      size={textSize}
                      onMark={(anchor) => conn.send({ t: 'library_mark', book: book.id, page, ...anchor })}
                      onUnmark={(ids) => ids.forEach((mark) => conn.send({ t: 'library_unmark', book: book.id, mark }))}
                      onDraw={(selected, prompt) =>
                        conn.send({ t: 'library_draw', book: book.id, page, block: selected.block, quote: selected.anchor.quote, context: selected.context, prompt })
                      }
                      onAsk={(selected, question) => {
                        if (!discuss) toggleDiscuss();
                        conn.send({ t: 'library_ask', book: book.id, page, quote: selected.anchor.quote, question });
                      }}
                      onRedraw={(figure) => conn.send({ t: 'library_redraw', book: book.id, figure })}
                      onUndraw={(figure) => conn.send({ t: 'library_undraw', book: book.id, figure })}
                    />
                  ) : entry && 'error' in entry ? (
                    <p className="text-destructive">{entry.error}</p>
                  ) : (
                    <p className="flex items-center gap-2 text-muted-foreground">
                      <i className={spinner} />
                      {page}쪽을 번역하는 중이에요
                    </p>
                  )}
                </div>
              </article>
              <footer className="flex items-center gap-1 border-t px-4 py-2 text-[11.5px] text-muted-foreground">
                <span className="flex-1">
                  {page}쪽 번역{entry && 'text' in entry ? ' · 저장됨' : ''}
                </span>
                <Button variant="ghost" size="icon-xs" aria-label="번역 글자 작게" disabled={textSize <= TEXT_SIZES.min} onClick={() => resize(-1)}>
                  <AArrowDown />
                </Button>
                <span className="w-5 text-center tabular-nums" title="번역 글자 크기">
                  {textSize}
                </span>
                <Button variant="ghost" size="icon-xs" aria-label="번역 글자 크게" disabled={textSize >= TEXT_SIZES.max} onClick={() => resize(1)}>
                  <AArrowUp />
                </Button>
                <Button className="ml-2" variant="ghost" size="xs" disabled={!entry || 'pending' in entry} onClick={() => void translate(page, true)}>
                  <RotateCw />
                  {entry && 'error' in entry ? '다시 시도' : '다시 번역'}
                </Button>
              </footer>
            </div>
          ) : (
            opening
          )}
        </section>
        {/* Kept mounted while folded away, so the discussion's terminal keeps its screen. */}
        <aside
          inert={!discuss}
          className={cn(
            'absolute inset-y-0 right-0 z-10 flex w-[400px] max-w-full flex-col overflow-hidden rounded-2xl bg-background shadow-[0_12px_40px_rgba(0,0,0,0.18)] ring-1 ring-foreground/10 transition-transform duration-300 ease-out',
            !discuss && 'translate-x-[calc(100%+16px)]',
          )}
        >
          <div className="flex items-center gap-2 border-b px-3 py-2">
            <b className="font-semibold">이 책과 논의</b>
            <span className="text-muted-foreground">claude</span>
            <span className="flex-1" />
            <span className="text-muted-foreground">{page}쪽</span>
            <Button variant="ghost" size="xs" onClick={toggleDiscuss}>
              접기
            </Button>
          </div>
            <div className="relative min-h-0 flex-1">
              {pane ? (
                <TerminalView conn={conn} pane={pane} focused={false} />
              ) : (
                <div className="flex h-full flex-col items-center justify-center gap-2 px-8 text-center text-muted-foreground">
                  <NotebookPen className="size-5 text-foreground" />
                  <b className="text-foreground">{book.session ? '이 책과 나눈 논의가 있어요' : '이 책 전용 claude 세션'}</b>
                  <span>읽다가 물어보세요. 정리한 내용은 노트에 쌓이고, 다음에 열면 이어서 논의해요.</span>
                  <Button
                    className="mt-2"
                    disabled={starting}
                    onClick={() => {
                      setStarting(true);
                      conn.send({ t: 'library_discuss', book: book.id });
                    }}
                  >
                    {starting ? '여는 중…' : book.session ? '논의 이어 가기' : '논의 시작'}
                  </Button>
                </div>
              )}
            </div>
            <footer className="flex items-center gap-2 border-t px-3 py-2 text-[11.5px] text-muted-foreground">
              <span className="min-w-0 flex-1 truncate" title={book.note}>
                노트 · {book.note.split('/notes/')[1] ?? book.note}
              </span>
              <Button variant="ghost" size="xs" onClick={() => conn.send({ t: 'library_open_note', book: book.id })}>
                열기
              </Button>
            </footer>
          </aside>
      </div>
    </main>
  );
}
