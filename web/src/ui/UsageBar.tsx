import { Settings, X } from 'lucide-react';
import { Popover } from 'radix-ui';
import type { AgentView } from '@/connection';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { type Accounts, AccountList, AgentMark, agentName, hasReset, limitTone, shownWindows, switchNote, toneText, windowName } from './Accounts';
import { usePrefs } from './prefs';

/** The thin bar under everything: each agent's signed-in account and how much of its limits is used. */
export function UsageBar({ accounts, online, now, onManage }: { accounts: Accounts; online: boolean; now: number; onManage: (view: AgentView) => void }) {
  const prefs = usePrefs();
  const shown = accounts.agents.filter((view) => prefs.bar[view.agent] && (view.installed || view.accounts.length > 0));
  return (
    <footer className="flex h-[26px] shrink-0 items-center gap-0.5 border-t border-border/60 bg-canvas px-2 text-[11.5px] text-foreground/80">
      <div className="flex min-w-0 items-center gap-0.5 overflow-hidden">
        {shown.map((view) => (
          <Chip key={view.agent} view={view} accounts={accounts} now={now} onManage={() => onManage(view)} />
        ))}
      </div>
      <span className="ml-auto flex shrink-0 items-center gap-1.5 pl-2 whitespace-nowrap text-muted-foreground">
        <i className={cn('size-1.5 rounded-full', online ? 'bg-foreground/50' : 'bg-destructive')} />
        {online ? 'naru-d 연결됨' : 'naru-d 연결 끊김'}
      </span>
    </footer>
  );
}

function Chip({ view, accounts, now, onManage }: { view: AgentView; accounts: Accounts; now: number; onManage: () => void }) {
  const prefs = usePrefs();
  const active = view.accounts.find((account) => account.active);
  const windows = active?.usage ? shownWindows(active.usage.windows, prefs).filter((w) => !hasReset(w, now)) : [];
  return (
    <Popover.Root onOpenChange={(open) => open && accounts.refresh()}>
      <Popover.Trigger className="inline-flex h-5 items-center gap-1.5 rounded-md px-1.5 whitespace-nowrap outline-none hover:bg-background data-[state=open]:bg-background">
        <AgentMark kind={view.agent} off={!active} small />
        {agentName(view.agent)}
        <span className="max-w-[160px] truncate text-muted-foreground">{active?.label ?? (view.installed ? '로그인 필요' : '설치 안 됨')}</span>
        {windows.map((w) => (
          <span key={w.minutes} className={cn('font-medium', toneText(limitTone(w.percent)))}>
            · {windowName(w.minutes, true)} {Math.round(w.percent)}%
          </span>
        ))}
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          side="top"
          align="start"
          sideOffset={6}
          collisionPadding={8}
          className="z-50 w-[440px] rounded-2xl bg-background text-[13px] shadow-[0_12px_36px_rgba(9,9,11,.18),0_0_0_1px_rgba(9,9,11,.04)] outline-none data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:slide-in-from-bottom-1"
        >
          <div className="flex items-center gap-2 px-3 pt-2.5 pb-2">
            <AgentMark kind={view.agent} off={!active} />
            <b className="font-semibold">{agentName(view.agent)} 계정</b>
            <span className="ml-auto" />
            <Popover.Close asChild>
              <Button variant="ghost" size="xs" onClick={onManage}>
                <Settings />
                관리
              </Button>
            </Popover.Close>
            <Popover.Close asChild>
              <Button variant="ghost" size="icon-xs" aria-label="닫기">
                <X />
              </Button>
            </Popover.Close>
          </div>
          <div className="max-h-[360px] overflow-y-auto px-3 pt-0.5">
            <AccountList view={view} accounts={accounts} now={now} />
          </div>
          {switchNote(view) && <p className="px-3 pt-0.5 pb-3 text-[12px] text-muted-foreground">{switchNote(view)}</p>}
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}
