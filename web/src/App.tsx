import { useEffect, useRef, useState } from 'react';
import { BookOpen, CircleCheck, Coins, Megaphone, FileDiff, FileText, GitPullRequest, History, Lightbulb, Network, PencilLine, Server, SquareTerminal } from 'lucide-react';
import { Connection, type ClientMsg, type Id, type Layout, type Mark, type PaneId, type Proposal, type Workspace } from './connection';
import { TerminalView, selections } from './TerminalView';
import { Button } from '@/components/ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';
import { type Proofs, type Teams, family, groupKey, panesOf, paneTitle, projectKeyOf, projects } from './ui/model';
import { CloseConfirm } from './ui/CloseConfirm';
import { Brain } from './ui/Brain';
import { CodeMap } from './ui/CodeMap';
import { Overview } from './ui/Overview';
import { ThemeGallery } from './ui/ThemeGallery';
import { DocLane } from './ui/DocLane';
import { AiPanel, useAi } from './ui/AiPanel';
import { LearnMap } from './ui/LearnMap';
import { type LearnFocus, LearnPanel } from './ui/LearnPanel';
import { RecordPanel } from './ui/RecordPanel';
import { ProofPanel, useReviews } from './ui/ProofPanel';
import { PaneHead } from './ui/PaneHead';
import { PrPanel } from './ui/PrPanel';
import { ServerPanel } from './ui/ServerPanel';
import { CostPanel } from './ui/CostPanel';
import { MetaAdsPanel } from './ui/MetaAdsPanel';
import { ShortcutSheet } from './ui/ShortcutSheet';
import { useNotify } from './ui/notify';
import { Palette } from './ui/Palette';
import { Rail, type RailItem } from './ui/Rail';
import { ProjectRail, shownRailKeys, useRailProject } from './ui/ProjectRail';
import { syncStored } from './ui/stored';
import { syncBackgrounds } from './ui/background';
import { Sidebar } from './ui/Sidebar';
import { Library } from './ui/Library';
import { Marketing } from './ui/Marketing';
import { Boundary } from './ui/Boundary';
import { useAccounts } from './ui/Accounts';
import { UsageBar } from './ui/UsageBar';
import { type SettingsAt, SettingsDialog } from './ui/Settings';
import { useShortcuts } from './ui/shortcuts';

declare global {
  interface Window {
    __NARU_WS__?: string;
    __NARU_ERROR__?: string;
  }
}

// The app window hands over the daemon's address; a browser in development gets it from Vite.
const url = window.__NARU_WS__ ?? (import.meta.env.VITE_NARU_WS as string | undefined);

function useNow(intervalMs: number): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(timer);
  }, [intervalMs]);
  return now;
}

export function App() {
  const [workspace, setWorkspace] = useState<Workspace | null>(null);
  const [teams, setTeams] = useState<Teams>({});
  const [conn] = useState(() => {
    if (!url) return null;
    const opened = new Connection(url);
    syncStored(opened);
    syncBackgrounds(opened);
    return opened;
  });
  const [online, setOnline] = useState(false);
  const [panel, setPanel] = useState<string | null>(null);
  const [finding, setFinding] = useState<PaneId | null>(null);
  const [keysOpen, setKeysOpen] = useState(false);
  const [lane, setLane] = useState<{ open: boolean; active: Id | null }>({ open: false, active: null });
  const newestView = useRef<number | null>(null);
  const [asking, setAsking] = useState<{ panes: PaneId[]; children: PaneId[]; busy: string[] } | null>(null);
  const [learn, setLearn] = useState<{ marks: Mark[]; proposals: Proposal[]; memoryOn: boolean; error: string | null }>({
    marks: [],
    proposals: [],
    memoryOn: true,
    error: null,
  });
  const [learnFocus, setLearnFocus] = useState<LearnFocus | null>(null);
  const [notice, setNotice] = useState<{ text: string; at: number } | null>(null);
  const [brain, setBrain] = useState(false);
  const [palette, setPalette] = useState(false);
  const [overview, setOverview] = useState(false);
  const [codeMap, setCodeMap] = useState(false);
  const [learnMap, setLearnMap] = useState(false);
  const [themes, setThemes] = useState(false);
  const [library, setLibrary] = useState(false);
  /** The 서재 stays mounted once opened, so coming back does not reopen its book and session. */
  const [shelfOpened, setShelfOpened] = useState(false);
  /** Left the 서재 by picking a session on the graph: a button leads back. */
  const [backToLibrary, setBackToLibrary] = useState(false);
  const openLibrary = () => {
    setMarketing(false);
    setLibrary(true);
    setShelfOpened(true);
    setBackToLibrary(false);
  };
  const [marketing, setMarketing] = useState(false);
  const [marketingOpened, setMarketingOpened] = useState(false);
  const openMarketing = () => {
    setLibrary(false);
    setMarketing(true);
    setMarketingOpened(true);
  };
  const [settings, setSettings] = useState<SettingsAt | null>(null);
  const [settingsTab, setSettingsTab] = useState<SettingsAt>({ page: 'screen', agent: 'claude' });
  const [railProject, setRailProject] = useRailProject();
  const now = useNow(30_000);
  const reviews = useReviews(conn, workspace);

  // A document an agent just opened shows in the lane at once, but stays unseen (and in 내 차례)
  // until the user picks it.
  useEffect(() => {
    if (!workspace) return;
    const newest = workspace.views.filter((view) => !view.proof).reduce((max, view) => Math.max(max, view.id), 0);
    if (newestView.current !== null && newest > newestView.current) setLane({ open: true, active: newest });
    newestView.current = newest;
  }, [workspace]);

  useEffect(() => {
    if (!conn) return;
    const connection = conn;
    const offMessage = connection.onMessage((message) => {
      if (message.t === 'state') {
        setWorkspace(message.workspace);
        setTeams(message.team);
      } else if (message.t === 'learn') {
        setLearn({ marks: message.marks, proposals: message.proposals, memoryOn: message.memory_on, error: message.error });
      } else if (message.t === 'learn_marked') {
        setPanel('learn');
        setLearnFocus({ id: message.id, retitle: true });
      } else if (message.t === 'learn_failure' && message.id === null) {
        setNotice({ text: message.message, at: Date.now() });
      } else if (message.t === 'notice') {
        setNotice({ text: message.text, at: Date.now() });
      } else if (message.t === 'error') {
        console.error('naru-d:', message.message);
      }
    });
    const offOpen = connection.onOpen(() => {
      setOnline(true);
      connection.send({ t: 'read_learn' });
    });
    const offClose = connection.onClose(() => setOnline(false));
    connection.start();
    return () => {
      offMessage();
      offOpen();
      offClose();
      connection.close();
    };
  }, [conn]);

  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(null), 3500);
    return () => clearTimeout(timer);
  }, [notice]);

  const shortcuts = useShortcuts();
  const front = workspace && (workspace.tabs.find((tab) => tab.id === workspace.active_tab) ?? workspace.tabs[0]);
  // Last focused tab per rail project, so switching back to a project lands where the user left it.
  const lastTab = useRef(new Map<string, number>());
  // Rail projects, most recently used first, for ⌥Tab.
  const recent = useRef<string[]>([]);
  // The order ⌥Tab steps through while ⌥ stays down, and where it is.
  const cycle = useRef<{ keys: string[]; at: number } | null>(null);
  useEffect(() => {
    const end = () => (cycle.current = null);
    const release = (event: KeyboardEvent) => event.key === 'Alt' && end();
    window.addEventListener('keyup', release, true);
    window.addEventListener('blur', end);
    return () => {
      window.removeEventListener('keyup', release, true);
      window.removeEventListener('blur', end);
    };
  }, []);
  const used = (key: string) => {
    if (key !== 'all') recent.current = [key, ...recent.current.filter((k) => k !== key)];
  };
  // The sidebar always shows the focused session: focus moving into another project selects it.
  useEffect(() => {
    if (!workspace || !front) return;
    const key = projectKeyOf(workspace, front);
    lastTab.current.set(key, front.id);
    used(key);
    if (railProject !== 'all' && railProject !== key) setRailProject(key);
  }, [front?.id]);
  const ai = useAi(conn, online ? (front?.focused ?? null) : null, panel === 'suggest');
  useNotify(workspace, teams, online);
  const accounts = useAccounts(conn, online, (text) => setNotice({ text, at: Date.now() }));

  if (!url) {
    const reason = window.__NARU_ERROR__ ?? 'VITE_NARU_WS가 없습니다. script/dev로 띄워 주세요.';
    return <p className="p-4">{reason}</p>;
  }
  if (!conn || !workspace) return null;
  const send = (message: ClientMsg) => conn.send(message);
  const active = workspace.tabs.find((tab) => tab.id === workspace.active_tab) ?? workspace.tabs[0];
  const laneViews = workspace.views.filter((view) => !view.proof);
  const proofs: Proofs = Object.fromEntries(
    Object.entries(reviews).flatMap(([view, review]) => (review ? [[view, review.findings.length]] : [])),
  );
  const unseenViews = laneViews.filter((view) => !view.seen).length;
  const openView = (view: Id) => {
    setLane({ open: true, active: view });
    send({ t: 'view_seen', view });
  };
  const close = (panes: PaneId[]) => panes.forEach((pane) => send({ t: 'close_pane', pane }));
  const requestClose = (panes: PaneId[]) => {
    const { children, busy } = family(workspace, teams, panes);
    if (busy.length) setAsking({ panes, children, busy });
    else close([...panes, ...children]);
  };
  const toggle = (key: string) => {
    if (key === 'brain') return setBrain((open) => !open);
    if (key === 'graph') return setOverview((open) => !open);
    if (key === 'code') return setCodeMap((open) => !open);
    if (key !== 'docs') return setPanel((open) => (open === key ? null : key));
    if (lane.open) return setLane((now) => ({ ...now, open: false }));
    const mine = laneViews.filter((view) => view.from === active?.focused);
    const pick = (mine.length ? mine : laneViews).at(-1);
    if (pick) openView(pick.id);
  };
  const focused = active?.focused ?? null;
  const openSettings = (at: SettingsAt | null) => {
    if (at) {
      setSettingsTab(at);
      accounts.refresh();
    }
    setSettings(at);
  };
  const projectCards = (key: string) => projects(workspace).filter(({ group }) => key === 'all' || groupKey(group) === key).flatMap((project) => project.cards.map((card) => card.tab));
  const cards = projectCards(railProject);
  const selectProject = (key: string) => {
    setRailProject(key);
    used(key);
    if (key === 'all') return;
    const tabs = projectCards(key);
    const tab = tabs.find((t) => t.id === lastTab.current.get(key)) ?? tabs[0];
    if (tab) send({ t: 'focus', pane: tab.focused });
  };
  const railKeys = shownRailKeys(workspace);
  // Cycling skips projects without sessions: landing there would leave no focused card in sight.
  // Like the app switcher: each ⌥Tab while ⌥ is held goes one project further back.
  const recentProject = (step: number) => {
    if (!cycle.current) {
      const keys = recent.current.filter((key) => railKeys.includes(key) && projectCards(key).length);
      // From a project without sessions (left out of keys), the first step lands on keys[0].
      cycle.current = { keys, at: keys.indexOf(railProject) };
    }
    const { keys, at } = cycle.current;
    if (!keys.length || (keys.length === 1 && at === 0)) return;
    cycle.current.at = (((at + step) % keys.length) + keys.length) % keys.length;
    selectProject(keys[cycle.current.at]);
  };
  const stepProject = (step: number) => {
    const live = railKeys.filter((key) => key === railProject || projectCards(key).length);
    const at = live.indexOf(railProject);
    const next = live[at < 0 ? (step > 0 ? 0 : live.length - 1) : (at + step + live.length) % live.length];
    if (next && next !== railProject) selectProject(next);
  };
  const goTab = (step: number) => {
    const at = cards.findIndex((tab) => tab.id === active?.id);
    const next = cards[(at + step + cards.length) % cards.length];
    if (next) send({ t: 'focus', pane: next.focused });
  };
  const goPane = (step: number) => {
    const panes = active ? panesOf(active.layout) : [];
    const next = panes[(panes.indexOf(focused ?? '') + step + panes.length) % panes.length];
    if (next) send({ t: 'focus', pane: next });
  };
  shortcuts.current = {
    'mod+k': () => setPalette((open) => !open),
    'mod+f': () => focused && setFinding(focused),
    'mod+/': () => setKeysOpen((open) => !open),
    'mod+alt+b': () => setBrain((open) => !open),
    'mod+alt+g': () => setOverview((open) => !open),
    'mod+,': () => openSettings(settings ? null : settingsTab),
    'mod+shift+l': () => {
      const text = (focused && selections.get(focused)?.()) || '';
      if (focused && text.trim()) send({ t: 'learn_mark', pane: focused, text });
      else setNotice({ text: '먼저 학습할 부분을 드래그해서 고르세요', at: Date.now() });
    },
    'mod+t': () => send({ t: 'new_tab', group: active?.group ?? undefined, cwd: (focused && workspace.panes[focused]?.cwd) || undefined }),
    'mod+d': () => focused && send({ t: 'split', pane: focused, direction: 'right' }),
    'mod+shift+d': () => focused && send({ t: 'split', pane: focused, direction: 'down' }),
    'mod+w': () => focused && requestClose([focused]),
    'mod+shift+]': () => goTab(1),
    'mod+shift+[': () => goTab(-1),
    'mod+]': () => goPane(1),
    'mod+[': () => goPane(-1),
    ...Object.fromEntries(
      cards.slice(0, 9).map((tab, index) => [`mod+${index + 1}`, () => send({ t: 'focus', pane: tab.focused })]),
    ),
    'alt+0': () => selectProject('all'),
    ...Object.fromEntries(railKeys.slice(0, 9).map((key, index) => [`alt+${index + 1}`, () => selectProject(key)])),
    'alt+tab': () => recentProject(1),
    'alt+shift+tab': () => recentProject(-1),
    'mod+alt+]': () => stepProject(1),
    'mod+alt+[': () => stepProject(-1),
  };
  const rail: RailItem[] = [
    { key: 'record', label: '기록', icon: History },
    { key: 'docs', label: '문서', icon: FileText, badge: unseenViews, urgent: unseenViews > 0 },
    { key: 'proof', label: '증명', icon: CircleCheck },
    { key: 'suggest', label: 'Naru AI', icon: PencilLine, badge: ai.view?.badge ?? 0, urgent: (ai.view?.badge ?? 0) > 0 },
    { key: 'graph', label: '한눈에 보기', icon: Network },
    { key: 'code', label: '코드 지도', icon: FileDiff },
    { key: 'prs', label: 'PR', icon: GitPullRequest },
    { key: 'servers', label: '개발 서버', icon: Server },
    { key: 'costs', label: '비용', icon: Coins },
    { key: 'meta', label: 'Meta 광고', icon: Megaphone },
    { key: 'learn', label: '학습함', icon: BookOpen, badge: learn.marks.filter((mark) => mark.stage === 'new').length + learn.proposals.length, bottom: true },
    { key: 'brain', label: 'Brain', icon: Lightbulb, bottom: true },
  ];

  const render = (layout: Layout): React.ReactNode => {
    if (layout.kind === 'pane') {
      const focused = active?.focused === layout.pane;
      return (
        <section
          key={layout.pane}
          onMouseDown={() => send({ t: 'focus', pane: layout.pane })}
          className={cn(
            'flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-2xl bg-background',
          )}
        >
          <Boundary name="이 pane">
            <PaneHead conn={conn} ws={workspace} teams={teams} pane={layout.pane} focused={focused} send={send} onClose={() => requestClose([layout.pane])} finding={finding === layout.pane} onFindEnd={() => setFinding(null)} />
            <div className="relative min-h-0 flex-1">
              <TerminalView conn={conn} pane={layout.pane} focused={focused} />
            </div>
          </Boundary>
        </section>
      );
    }
    return (
      <div className={cn('flex min-h-0 min-w-0 flex-1 gap-2', !layout.row && 'flex-col')}>
        {layout.children.map((child, index) => (
          <div key={index} className="flex min-h-0 min-w-0" style={{ flex: layout.flex[index] }}>
            {render(child)}
          </div>
        ))}
      </div>
    );
  };

  return (
    <div className="flex h-full flex-col">
      <div className="flex min-h-0 flex-1">
        <Boundary name="프로젝트 레일" className="w-14 shrink-0 bg-canvas p-1 pt-10 text-[10px]">
          <ProjectRail ws={workspace} teams={teams} now={now} proofs={proofs} selected={railProject} onSelect={(key) => (setLibrary(false), setMarketing(false), selectProject(key))} send={send} conn={conn} library={library} onLibrary={openLibrary} marketing={marketing} onMarketing={openMarketing} />
        </Boundary>
        {overview && (
          // The graph is a floating window over the 서재 too; picking a session there leaves it.
          <Boundary name="한눈에 보기">
            <Overview conn={conn} ws={workspace} teams={teams} onClose={() => setOverview(false)} onPicked={() => library && (setLibrary(false), setBackToLibrary(true))} />
          </Boundary>
        )}
        {codeMap && (
          <Boundary name="코드 지도">
            <CodeMap conn={conn} ws={workspace} pane={active?.focused ?? null} onClose={() => setCodeMap(false)} />
          </Boundary>
        )}
        {shelfOpened && (
          <div className={library ? 'contents' : 'hidden'}>
            <Boundary name="서재" className="flex min-w-0 flex-1">
              <Library conn={conn} ws={workspace} now={now} visible={library} />
            </Boundary>
          </div>
        )}
        {marketingOpened && (
          <div className={marketing ? 'contents' : 'hidden'}>
            <Boundary name="마케팅" className="flex min-w-0 flex-1">
              <Marketing conn={conn} visible={marketing} />
            </Boundary>
          </div>
        )}
        {/* Hidden rather than unmounted: remounting re-creates every pane's terminal and replays its output. */}
        <div className={library || marketing ? 'hidden' : 'contents'}>
            <Boundary name="사이드바" className="w-64 shrink-0 bg-canvas p-3 pt-12 text-[12.5px]">
              <Sidebar project={railProject} ws={workspace} teams={teams} now={now} proofs={proofs} send={send} onView={openView} onClose={requestClose} onSearch={() => setPalette(true)} onTheme={() => setThemes(true)} onSettings={() => openSettings(settingsTab)} />
            </Boundary>
            <main className="relative flex min-w-0 flex-1 gap-2 py-2 pr-2">
              {!online && (
                <div className="absolute top-4 left-1/2 z-10 flex -translate-x-1/2 items-center gap-2 rounded-full bg-primary px-3.5 py-1.5 text-primary-foreground">
                  <i className="size-3 animate-spin rounded-full border-2 border-current border-t-transparent" />
                  naru-d와 연결이 끊겼어요 · 다시 잇는 중
                </div>
              )}
              {backToLibrary && (
                <div className="absolute top-[60px] right-[76px] z-20 animate-[naru-float_4.5s_ease-in-out_infinite]">
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <button
                        aria-label="서재로 돌아가기"
                        onClick={openLibrary}
                        className="grid size-11 animate-in place-items-center rounded-full bg-primary text-primary-foreground shadow-[0_6px_18px_rgba(9,9,11,0.28)] duration-500 fade-in slide-in-from-right-4 zoom-in-90 hover:scale-105"
                      >
                        <BookOpen className="size-[19px]" />
                      </button>
                    </TooltipTrigger>
                    <TooltipContent side="left">서재로 돌아가기</TooltipContent>
                  </Tooltip>
                </div>
              )}
              <div className={cn('flex min-w-[280px] flex-[1_1_280px]', !online && 'pointer-events-none opacity-45')}>
                {active ? render(active.layout) : <Empty onNew={() => send({ t: 'new_tab' })} />}
              </div>
              {panel === 'record' && (
                <Boundary name="기록 패널">
                  <RecordPanel
                    conn={conn}
                    pane={active?.focused ?? null}
                    title={active ? paneTitle(workspace, active.focused) : ''}
                    onClose={() => setPanel(null)}
                  />
                </Boundary>
              )}
              {panel === 'proof' && (
                <Boundary name="증명 패널">
                  <ProofPanel
                    conn={conn}
                    ws={workspace}
                    pane={active?.focused ?? null}
                    title={active ? paneTitle(workspace, active.focused) : ''}
                    reviews={reviews}
                    onClose={() => setPanel(null)}
                  />
                </Boundary>
              )}
              {panel === 'suggest' && (
                <Boundary name="Naru AI 패널">
                  <AiPanel
                    conn={conn}
                    ws={workspace}
                    pane={active?.focused ?? null}
                    view={ai.view}
                    error={ai.error}
                    onOpenSession={(pane) => {
                      send({ t: 'focus', pane });
                      setPanel('record');
                    }}
                    onClose={() => setPanel(null)}
                  />
                </Boundary>
              )}
              {panel === 'learn' && (
                <Boundary name="학습함">
                  <LearnPanel
                    conn={conn}
                    ws={workspace}
                    marks={learn.marks}
                    proposals={learn.proposals}
                    memoryOn={learn.memoryOn}
                    error={learn.error}
                    focus={learnFocus}
                    onOpenAi={() => setPanel('suggest')}
                    onOpenMap={() => setLearnMap(true)}
                    onClose={() => setPanel(null)}
                  />
                </Boundary>
              )}
              {panel === 'prs' && (
                <Boundary name="PR 패널">
                  <PrPanel conn={conn} ws={workspace} send={send} onClose={() => setPanel(null)} />
                </Boundary>
              )}
              {panel === 'costs' && (
                <Boundary name="비용 패널">
                  <CostPanel conn={conn} send={send} onClose={() => setPanel(null)} />
                </Boundary>
              )}
              {panel === 'meta' && (
                <Boundary name="Meta 광고 패널">
                  <MetaAdsPanel conn={conn} send={send} onSettings={() => openSettings({ ...settingsTab, page: 'meta' })} onClose={() => setPanel(null)} />
                </Boundary>
              )}
              {panel === 'servers' && (
                <Boundary name="개발 서버 패널">
                  <ServerPanel conn={conn} ws={workspace} send={send} onClose={() => setPanel(null)} />
                </Boundary>
              )}
              {notice && (
                <div
                  key={notice.at}
                  className="absolute bottom-5 left-1/2 z-[60] max-w-[480px] -translate-x-1/2 animate-in truncate rounded-full bg-primary px-4 py-2 text-primary-foreground fade-in slide-in-from-bottom-2"
                >
                  {notice.text}
                </div>
              )}
              {lane.open && laneViews.length > 0 && <Boundary name="문서 칸"><DocLane conn={conn} views={laneViews} active={lane.active} onPick={openView} /></Boundary>}
              <CloseConfirm
                busy={asking?.busy ?? null}
                onCancel={() => setAsking(null)}
                onOnly={() => (asking && close(asking.panes), setAsking(null))}
                onAll={() => (asking && close([...asking.panes, ...asking.children]), setAsking(null))}
              />
              <Rail
                items={rail}
                open={[...(lane.open && laneViews.length > 0 ? ['docs'] : []), ...(panel ? [panel] : []), ...(brain ? ['brain'] : []), ...(overview ? ['graph'] : []), ...(codeMap ? ['code'] : [])]}
                onToggle={toggle}
              />
              {brain && <Boundary name="Brain"><Brain conn={conn} onClose={() => setBrain(false)} /></Boundary>}
              {themes && <Boundary name="테마 고르기"><ThemeGallery onClose={() => setThemes(false)} /></Boundary>}
              {learnMap && <Boundary name="학습 지도"><LearnMap conn={conn} marks={learn.marks} proposals={learn.proposals} onClose={() => setLearnMap(false)} /></Boundary>}
              {keysOpen && <Boundary name="단축키"><ShortcutSheet onClose={() => setKeysOpen(false)} /></Boundary>}
              {palette && <Boundary name="검색"><Palette conn={conn} ws={workspace} teams={teams} now={Date.now()} onClose={() => setPalette(false)} /></Boundary>}
              {settings && (
                <Boundary name="설정">
                  <SettingsDialog
                    conn={conn}
                    at={settings}
                    accounts={accounts}
                    now={Date.now()}
                    onAt={openSettings}
                    onTheme={() => (setSettings(null), setThemes(true))}
                    onClose={() => setSettings(null)}
                  />
                </Boundary>
              )}
            </main>
        </div>
      </div>
      <Boundary name="하단 바" className="flex h-7 items-center gap-2 overflow-hidden px-3 text-[11px] [&_*]:m-0 [&_button]:py-0">
        <UsageBar accounts={accounts} online={online} now={now} onManage={(view) => openSettings({ page: 'accounts', agent: view.agent })} />
      </Boundary>
    </div>
  );
}

function Empty({ onNew }: { onNew: () => void }) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-2 rounded-2xl bg-background text-muted-foreground">
      <span className="mb-1 grid size-11 place-items-center rounded-xl bg-muted text-foreground">
        <SquareTerminal className="size-5" />
      </span>
      <b className="text-[15px] font-semibold text-foreground">첫 세션을 열어 보세요</b>
      <span>프로젝트 폴더를 고르면 그 아래로 세션 카드가 모여요</span>
      <Button className="mt-2" onClick={onNew}>
        새 세션 ⌘T
      </Button>
    </div>
  );
}
