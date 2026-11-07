import { X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { UI_SIZE, usePrefs } from './prefs';

/** A rail panel's card beside the rail. */
export function SidePanel({
  title,
  sub,
  actions,
  width = 360,
  onClose,
  children,
}: {
  title: string;
  sub?: string;
  /** Buttons before the close button. */
  actions?: React.ReactNode;
  /** Pixels; wide panels such as Meta 광고 need more than the default. */
  width?: number;
  onClose: () => void;
  children: React.ReactNode;
}) {
  const { ui } = usePrefs();
  return (
    <aside className="flex min-w-[300px] flex-col overflow-hidden rounded-2xl bg-background" style={{ width }}>
      <div className="flex h-11 shrink-0 items-center gap-2 pr-2 pl-4">
        <b className="text-[14px] font-bold">{title}</b>
 {sub && <span className="min-w-0 flex-1 truncate text-muted-foreground">{sub}</span>}
        <span className="ml-auto" />
        {actions}
        <Button variant="ghost" size="icon-xs" onClick={onClose} aria-label="닫기">
          <X />
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto" style={{ zoom: ui / UI_SIZE.base }}>
        {children}
      </div>
    </aside>
  );
}
