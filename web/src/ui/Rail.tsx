import type { LucideIcon } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';

export interface RailItem {
  key: string;
  label: string;
  icon: LucideIcon;
  /** Shown in the icon's corner; orange when it waits on the user. */
  badge?: number;
  urgent?: boolean;
  /** Pinned to the bottom of the rail. */
  bottom?: boolean;
}

/** The icon column at the window's right edge; each icon opens one panel. */
export function Rail({ items, open, onToggle }: { items: RailItem[]; open: string[]; onToggle: (key: string) => void }) {
  const button = (item: RailItem) => (
    <Tooltip key={item.key}>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={item.label}
          aria-pressed={open.includes(item.key)}
          onClick={() => onToggle(item.key)}
          className={cn('relative rounded-[10px] text-muted-foreground', open.includes(item.key) && 'bg-muted text-foreground')}
        >
          <item.icon />
          {!!item.badge && (
            <span
              className={cn(
                'absolute -top-0.5 -right-0.5 grid h-[15px] min-w-[15px] place-items-center rounded-full border-[1.5px] border-background px-1 text-[9.5px] font-bold',
                item.urgent ? 'bg-turn text-white' : 'bg-primary text-primary-foreground',
              )}
            >
              {item.badge}
            </span>
          )}
        </Button>
      </TooltipTrigger>
      <TooltipContent side="left">{item.label}</TooltipContent>
    </Tooltip>
  );
  return (
    <nav className="flex w-11 shrink-0 flex-col items-center gap-1 rounded-2xl bg-background py-2">
      {items.filter((item) => !item.bottom).map(button)}
      <span className="flex-1" />
      {items.filter((item) => item.bottom).map(button)}
    </nav>
  );
}
