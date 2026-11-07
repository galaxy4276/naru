import { useState } from 'react';
import { Keyboard } from 'lucide-react';
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog';
import { SHORTCUT_GROUPS } from './shortcuts';

/** ⌘/: every shortcut in groups, narrowed by what you type. */
export function ShortcutSheet({ onClose }: { onClose: () => void }) {
  const [filter, setFilter] = useState('');
  const want = filter.trim().toLowerCase();
  const groups = SHORTCUT_GROUPS.map(([group, rows]) => [group, rows.filter(([label, keys]) => !want || `${group} ${label} ${keys}`.toLowerCase().includes(want))] as const).filter(
    ([, rows]) => rows.length,
  );
  const half = Math.ceil(groups.length / 2);
  const column = (part: typeof groups) => (
    <div className="flex min-w-0 flex-1 flex-col gap-3">
      {part.map(([group, rows]) => (
        <div key={group} className="flex flex-col">
          <b className="px-2 pb-1 text-[11.5px] font-semibold text-muted-foreground">{group}</b>
          {rows.map(([label, keys]) => (
            <div key={label} className="flex h-8 items-center justify-between gap-3 rounded-lg px-2 hover:bg-muted">
              <span className="truncate">{label}</span>
              <kbd className="shrink-0 rounded-md bg-muted px-1.5 py-0.5 font-sans text-[11.5px] font-semibold">{keys}</kbd>
            </div>
          ))}
        </div>
      ))}
    </div>
  );
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="top-20 translate-y-0 gap-0 rounded-2xl border-0 p-0 text-[13px] sm:max-w-[640px]" showCloseButton={false}>
        <div className="flex h-12 items-center gap-2 border-b px-4">
          <Keyboard className="size-4" />
          <DialogTitle className="text-[14px] font-bold">단축키</DialogTitle>
          <input
            autoFocus
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
            placeholder="거르기"
            className="ml-auto h-7 w-40 rounded-lg bg-muted px-2.5 outline-none placeholder:text-muted-foreground"
          />
        </div>
        <div className="flex max-h-[60vh] gap-4 overflow-y-auto p-3">
          {groups.length ? (
            <>
              {column(groups.slice(0, half))}
              {column(groups.slice(half))}
            </>
          ) : (
            <p className="w-full py-6 text-center text-muted-foreground">맞는 단축키가 없어요</p>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
