import { useEffect, useRef, useState } from 'react';
import { Folder, Layers, Palette, Plus, Search, Settings, X } from 'lucide-react';
import type { ClientMsg, PaneId, Tab, Workspace } from '@/connection';
import { Button } from '@/components/ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';
import { type Proofs, type QueueEntry, type Task, type Teams, TASK_SECTIONS, attention, cardLine, groupKey, paneProjectKey, panesOf, paneTitle, projectName, projects, tabTitle, tabTone, tasks, toneOf, unread } from './model';
import { Ring } from './status';
import { useEdgeWidth } from './frame';
import { BackgroundMenu, CardBackground } from './background';
import { getStored, setStored } from './stored';
import { useTheme } from './theme';
import { UI_SIZE, usePrefs } from './prefs';

const WIDTH_KEY = 'naru.sidebar.width';
const LENS_KEY = 'naru.sidebar.lens';

interface Props {
  /** The rail's selected project: `all`, a group id, or `loose`. */
  project: string;
  ws: Workspace;
  teams: Teams;
  now: number;
  proofs: Proofs;
  send: (message: ClientMsg) => void;
  /** Opens a document from a 볼 것 row. */
  onView: (view: number) => void;
  onClose: (panes: PaneId[]) => void;
  onSearch: () => void;
  onTheme: () => void;
  onSettings: () => void;
}

export function Sidebar({ project, ws, teams, now, proofs, send, onView, onClose, onSearch, onTheme, onSettings }: Props) {
  const { key: theme } = useTheme();
  const [width, grab] = useEdgeWidth(WIDTH_KEY, 256, 200, 520, 'right');
  const zoom = usePrefs().ui / UI_SIZE.base;
  const focus = (pane: PaneId) => send({ t: 'focus', pane });
  const [lens, setLens] = useState<'projects' | 'tasks'>(() => (getStored(LENS_KEY) === 'tasks' ? 'tasks' : 'projects'));
  const pick = (next: 'projects' | 'tasks') => {
    setStored(LENS_KEY, next);
    setLens(next);
  };
  const all = project === 'all';
  const inProject = (pane: PaneId) => all || paneProjectKey(ws, pane) === project;
  // Waiting on the user shows from every project; a row from another one names its project.
  const asks = attention(ws, teams, now, proofs).map((entry) =>
    inProject(entry.pane) ? entry : { ...entry, title: `${projectName(ws, entry.pane)} · ${entry.title}` },
  );
  const reads = unread(ws, now, proofs).filter((entry) => inProject(entry.pane));
  const title = all ? '모든 프로젝트' : (ws.groups.find((g) => String(g.id) === project)?.name ?? 'Terminal');
  const visibleTasks = tasks(ws, teams).filter((task) => all || groupKey(task.project) === project);
  const close = (tab: Tab) => onClose(panesOf(tab.layout));

  return (
    <aside className={cn('relative flex shrink-0 flex-col bg-canvas text-foreground', theme === 'black-sidebar' && 'dark')} style={{ width }}>
      <div onPointerDown={grab} className="absolute inset-y-0 right-0 z-10 w-1.5 cursor-col-resize" />
      <div data-tauri-drag-region className="h-10 shrink-0" />
      <div className="flex gap-1.5 px-2.5 pb-2">
        <button onClick={onSearch} className="flex h-8 flex-1 items-center gap-2 rounded-[10px] bg-background px-2.5 text-muted-foreground hover:text-foreground">
          <Search className="size-3.5" />
          <span className="flex-1 text-left">검색</span>
          <kbd className="rounded border px-1 font-mono text-[10.5px]">⌘K</kbd>
        </button>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button variant="ghost" size="icon-sm" className="rounded-[10px] bg-background" onClick={() => send({ t: 'new_tab' })} aria-label="새 세션">
              <Plus />
            </Button>
          </TooltipTrigger>
          <TooltipContent>새 세션 ⌘T</TooltipContent>
        </Tooltip>
      </div>
      <div className="flex items-center gap-2 px-3 pb-2">
        {all ? <Layers className="size-3.5 shrink-0 text-muted-foreground" /> : <Folder className="size-3.5 shrink-0 text-muted-foreground" />}
        <b className="min-w-0 flex-1 truncate font-semibold">{title}</b>
        <div className="flex shrink-0 rounded-lg bg-muted p-0.5 text-[12px]">
          {(['projects', 'tasks'] as const).map((key) => (
            <button
              key={key}
              onClick={() => pick(key)}
              className={cn('rounded-md px-2 py-0.5 font-medium', lens === key ? 'bg-background text-foreground shadow-xs' : 'text-muted-foreground')}
            >
              {key === 'projects' ? '프로젝트' : '태스크'}
            </button>
          ))}
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-2.5 pb-3" style={{ zoom }}>
        {lens === 'tasks' ? (
          <TaskList ws={ws} teams={teams} tasks={visibleTasks} showProject={all} activeTab={ws.active_tab} onFocus={focus} send={send} />
        ) : (
          <>
        <Queue title="내 차례" entries={asks} onPick={(entry) => (focus(entry.pane), entry.view && onView(entry.view))} />
        <Queue title="읽을 것" entries={reads} onPick={(entry) => focus(entry.pane)} />
        {projects(ws).filter(({ group }) => all || groupKey(group) === project).map(({ group, cards }) => (
          <section key={group?.id ?? 'loose'}>
            <div className="group/head mt-3.5 mb-1.5 flex items-center gap-1.5 px-1 text-[0.88em] font-semibold text-muted-foreground">
              <Folder className="size-3.5" />
              <span className="truncate">{group?.name ?? 'Terminal'}</span>
              <span className="ml-auto font-medium">{cards.length}</span>
              <button
                aria-label="이 프로젝트에 새 세션"
                className="hidden rounded p-0.5 hover:bg-accent group-hover/head:block"
                onClick={() => send(group ? { t: 'new_tab', group: group.id, ...(group.path ? { cwd: group.path } : {}) } : { t: 'new_tab' })}
              >
                <Plus className="size-3.5" />
              </button>
            </div>
            {cards.map(({ tab, depth }) => (
              <SessionCard
                key={tab.id}
                ws={ws}
                teams={teams}
                tab={tab}
                depth={depth}
                active={tab.id === ws.active_tab}
                onFocus={focus}
                onClose={() => close(tab)}
                send={send}
              />
            ))}
          </section>
        ))}
          </>
        )}
      </div>
      <div className="flex items-center gap-1.5 px-2.5 py-2 text-[12px] text-muted-foreground">
        <Button variant="ghost" size="xs" className="text-muted-foreground hover:bg-background" onClick={onSettings}>
          <Settings />
          설정
          <kbd className="font-mono text-[10.5px] opacity-70">⌘,</kbd>
        </Button>
        <Button variant="ghost" size="icon-xs" className="ml-auto bg-background" onClick={onTheme} aria-label="테마 고르기">
          <Palette />
        </Button>
      </div>
    </aside>
  );
}

function Queue({ title, entries, onPick }: { title: string; entries: QueueEntry[]; onPick: (entry: QueueEntry) => void }) {
  if (!entries.length) return null;
  return (
    <section>
      <div className="mt-2 mb-1.5 flex justify-between px-1 text-[0.8em] font-semibold text-muted-foreground">
        <span>{title}</span>
        <span>{entries.length}</span>
      </div>
      {entries.map((entry) => (
        <button
          key={`${entry.pane}-${entry.detail}`}
          onClick={() => onPick(entry)}
          className={cn(
            'mb-1 flex w-full items-start gap-2.5 rounded-xl bg-background px-2.5 py-2 text-left leading-snug hover:bg-muted',
            entry.dim && 'opacity-60',
          )}
        >
          <Ring tone={entry.tone} className="mt-[3px]" />
          <span className="min-w-0 flex-1">
            <b className="block truncate font-semibold">{entry.detail}</b>
            <span className="block truncate text-[0.9em] text-muted-foreground">{entry.title}</span>
          </span>
          {entry.trailing && <span className="shrink-0 text-[0.85em] text-muted-foreground">{entry.trailing}</span>}
        </button>
      ))}
    </section>
  );
}

interface CardProps {
  ws: Workspace;
  teams: Teams;
  tab: Tab;
  depth: number;
  active: boolean;
  onFocus: (pane: PaneId) => void;
  onClose: () => void;
  send: (message: ClientMsg) => void;
}

function SessionCard({ ws, teams, tab, depth, active, onFocus, onClose, send }: CardProps) {
  const panes = panesOf(tab.layout);
  const line = cardLine(ws, teams, tab);
  const tone = toneOf(ws.panes[tab.focused]?.session ?? null, teams[tab.focused]);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (active) ref.current?.scrollIntoView({ block: 'nearest' });
  }, [active]);
  return (
    <div ref={ref} className="relative mb-1.5" style={{ marginLeft: depth * 16 }}>
      {depth > 0 && <span className="absolute top-[-6px] -left-2.5 h-5 w-2 rounded-bl-md border-b border-l border-muted-foreground/40" />}
      <BackgroundMenu tab={tab} send={send} onClose={onClose}>
        <div
          role="button"
          tabIndex={0}
          onClick={() => onFocus(tab.focused)}
          onKeyDown={(event) => event.key === 'Enter' && onFocus(tab.focused)}
          className={cn(
            'group/card relative cursor-default overflow-hidden rounded-xl bg-background px-2.5 py-2 outline-none',
            active ? 'before:absolute before:top-2 before:bottom-2 before:left-0 before:z-10 before:w-[3px] before:rounded-r-full before:bg-primary' : 'hover:ring-1 hover:ring-border',
          )}
        >
          <CardBackground tab={tab} />
          <div className="relative">
            <div className="flex items-center gap-2">
              <Ring tone={tone} />
              <span className="flex-1 truncate font-semibold">{tabTitle(ws, tab)}</span>
              {teams[tab.focused]?.id && (
                <span className="font-mono text-[10.5px] text-muted-foreground group-hover/card:hidden">{teams[tab.focused].id}</span>
              )}
              <button
                aria-label="세션 닫기"
                onClick={(event) => (event.stopPropagation(), onClose())}
                className="hidden rounded p-0.5 text-muted-foreground hover:bg-muted group-hover/card:block"
              >
                <X className="size-3.5" />
              </button>
            </div>
            {line && (
              <div
                className={cn(
                  'mt-0.5 ml-[19px] truncate text-[0.88em]',
                  line.tone === 'turn' ? 'text-turn' : line.tone === 'err' ? 'text-destructive' : 'text-muted-foreground',
                )}
              >
                {line.text}
              </div>
            )}
            {panes.length > 1 && (
              <div className="mt-1.5 ml-[19px] flex flex-col gap-1">
                {panes.map((pane) => (
                  <button
                    key={pane}
                    onClick={(event) => (event.stopPropagation(), onFocus(pane))}
                    className={cn(
                      'relative flex items-center gap-2 overflow-hidden rounded-md bg-muted px-2 py-1 text-left text-[12px] hover:bg-accent',
                      pane === tab.focused && 'font-semibold before:absolute before:top-1 before:bottom-1 before:left-0 before:w-[2px] before:rounded-r-full before:bg-primary',
                    )}
                  >
                    <Ring tone={toneOf(ws.panes[pane]?.session ?? null, teams[pane])} className="size-2" />
                    <span className="truncate">{paneTitle(ws, pane)}</span>
                  </button>
                ))}
              </div>
            )}
          </div>
        </div>
      </BackgroundMenu>
    </div>
  );
}

function TaskList({ ws, teams, tasks, showProject, activeTab, onFocus, send }: { ws: Workspace; teams: Teams; tasks: Task[]; showProject: boolean; activeTab: number | null; onFocus: (pane: PaneId) => void; send: (message: ClientMsg) => void }) {
  if (!tasks.length) return <p className="px-1 pt-3 text-muted-foreground">이 프로젝트에는 아직 태스크가 없어요</p>;
  return (
    <>
      {TASK_SECTIONS.map(({ title, tones }) => {
        const rows = tasks.filter((task) => tones.includes(task.tone));
        if (!rows.length) return null;
        return (
          <section key={title}>
            <div className="mt-2 mb-1.5 flex justify-between px-1 text-[0.8em] font-semibold text-muted-foreground">
              <span>{title}</span>
              <span>{rows.length}</span>
            </div>
            {rows.map(({ root, kids, project, tone }) => {
              const line = cardLine(ws, teams, root);
              return (
                <BackgroundMenu key={root.id} tab={root} send={send}>
                  <div
                    role="button"
                    tabIndex={0}
                    onClick={() => onFocus(root.focused)}
                    onKeyDown={(event) => event.key === 'Enter' && onFocus(root.focused)}
                    className={cn('relative mb-1 cursor-default overflow-hidden rounded-xl bg-background px-2.5 py-2 outline-none', root.id === activeTab ? 'before:absolute before:top-2 before:bottom-2 before:left-0 before:z-10 before:w-[3px] before:rounded-r-full before:bg-primary' : 'hover:ring-1 hover:ring-border')}
                  >
                    <CardBackground tab={root} />
                    <div className="relative">
                      <div className="flex items-center gap-2">
                        <Ring tone={tone} />
                        <span className="flex-1 truncate font-semibold">{tabTitle(ws, root)}</span>
                        {showProject && <span className="max-w-[40%] shrink-0 truncate rounded bg-muted px-1.5 text-[10.5px] text-muted-foreground">{project?.name ?? 'Terminal'}</span>}
                      </div>
                      {line && (
                        <div className={cn('mt-0.5 ml-[19px] truncate text-[0.88em]', line.tone === 'turn' ? 'text-turn' : line.tone === 'err' ? 'text-destructive' : 'text-muted-foreground')}>{line.text}</div>
                      )}
                      {kids.map((kid) => (
                        <button
                          key={kid.id}
                          onClick={(event) => (event.stopPropagation(), onFocus(kid.focused))}
                          className="mt-1 ml-[19px] flex w-[calc(100%-19px)] items-center gap-2 text-left text-[0.88em] text-muted-foreground hover:text-foreground"
                        >
                          <Ring tone={tabTone(ws, teams, kid)} className="size-2" />
                          <span className="truncate">{tabTitle(ws, kid)}</span>
                        </button>
                      ))}
                    </div>
                  </div>
                </BackgroundMenu>
              );
            })}
          </section>
        );
      })}
    </>
  );
}
