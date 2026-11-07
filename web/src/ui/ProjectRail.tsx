import { useEffect, useState, useSyncExternalStore } from 'react';
import { BookOpen, Layers, Megaphone, Plus } from 'lucide-react';
import { ContextMenu as Menu } from 'radix-ui';
import { type ClientMsg, type Connection, type Workspace, decodeBase64 } from '@/connection';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';
import { type Proofs, type Teams, attention, basename, paneProjectKey, railProjects } from './model';
import { IMAGE, pickFiles } from './pick';
import { getStored, onStored, setStored } from './stored';

const PROJECT_KEY = 'naru.rail.project';
const ORDER_KEY = 'naru.rail.order';
const HIDDEN_KEY = 'naru.rail.hidden';
const THUMB_KEY = 'naru.rail.thumbs';
const LEGACY_KEY = 'naru.rail.thumbs.legacy';
const THUMB_PX = 72;

function stored<T>(key: string, fallback: T): T {
  try {
    return JSON.parse(getStored(key) ?? '') as T;
  } catch {
    return fallback;
  }
}

/** The selected rail project: `all`, a group id, or `loose`. */
export function useRailProject() {
  // Read before the daemon's copy arrives, so it follows that copy when it does.
  const project = useSyncExternalStore((listener) => onStored(PROJECT_KEY, listener), () => getStored(PROJECT_KEY) || 'all');
  return [project, (key: string) => setStored(PROJECT_KEY, key)] as const;
}

/** Rail project keys in the user's order, without hidden ones. */
export function shownRailKeys(ws: Workspace): string[] {
  const order = stored<string[]>(ORDER_KEY, []);
  const hidden = stored<string[]>(HIDDEN_KEY, []);
  const rank = (key: string) => (order.includes(key) ? order.indexOf(key) : order.length);
  return railProjects(ws)
    .map((p) => p.key)
    .filter((key) => !hidden.includes(key))
    .sort((a, b) => rank(a) - rank(b));
}

/** Centre-crops an image to a square PNG data URL for a rail tile. */
async function thumbnailOf(image: Blob): Promise<string> {
  const bitmap = await createImageBitmap(image);
  const side = Math.min(bitmap.width, bitmap.height);
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = THUMB_PX;
  const context = canvas.getContext('2d');
  if (!context) throw new Error('이미지를 그릴 수 없어요');
  context.drawImage(bitmap, (bitmap.width - side) / 2, (bitmap.height - side) / 2, side, side, 0, 0, THUMB_PX, THUMB_PX);
  bitmap.close();
  return canvas.toDataURL('image/png');
}

const initials = (name: string) => name.replace(/[^\p{L}\p{N}]/gu, '').slice(0, 2).toLowerCase();

export const menuItem = 'flex w-full cursor-default items-center rounded-md px-2.5 py-1.5 text-left outline-none data-[disabled]:opacity-40 data-[highlighted]:bg-accent';

interface Props {
  ws: Workspace;
  teams: Teams;
  now: number;
  proofs: Proofs;
  selected: string;
  onSelect: (key: string) => void;
  send: (message: ClientMsg) => void;
  conn: Connection;
  /** The 서재 screen is open in place of the sessions. */
  library: boolean;
  onLibrary: () => void;
  marketing: boolean;
  onMarketing: () => void;
}

export function ProjectRail({ ws, teams, now, proofs, selected, onSelect, send, conn, library, onLibrary, marketing, onMarketing }: Props) {
  const [order, setOrder] = useState<string[]>(() => stored(ORDER_KEY, []));
  const [hidden, setHidden] = useState<string[]>(() => stored(HIDDEN_KEY, []));
  const [dragging, setDragging] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<{ key: string; name: string } | null>(null);
  const [adding, setAdding] = useState<string | null>(null);
  const [thumbs, setThumbs] = useState<Record<string, string>>(() => stored(THUMB_KEY, {}));
  const [thumbError, setThumbError] = useState<string | null>(null);

  const all = railProjects(ws);
  const rank = (key: string) => (order.includes(key) ? order.indexOf(key) : order.length);
  const sorted = [...all].sort((a, b) => rank(a.key) - rank(b.key));
  const shown = sorted.filter((p) => !hidden.includes(p.key));
  const hiddenNow = sorted.filter((p) => hidden.includes(p.key));
  const effective = shown.some((p) => p.key === selected) ? selected : 'all';

  const waiting = new Map<string, 'turn' | 'err'>();
  for (const entry of attention(ws, teams, now, proofs)) {
    const key = paneProjectKey(ws, entry.pane);
    if (entry.tone === 'turn' || !waiting.has(key)) waiting.set(key, entry.tone === 'err' ? 'err' : 'turn');
  }

  const saveHidden = (next: string[]) => {
    setStored(HIDDEN_KEY, JSON.stringify(next));
    setHidden(next);
  };
  const drop = (target: string) => {
    if (!dragging || dragging === target) return;
    const keys = sorted.map((p) => p.key).filter((key) => key !== dragging);
    keys.splice(keys.indexOf(target), 0, dragging);
    setStored(ORDER_KEY, JSON.stringify(keys));
    setOrder(keys);
  };
  const hide = (key: string) => {
    saveHidden([...hidden, key]);
    if (selected === key) onSelect('all');
  };
  const saveThumbs = (next: Record<string, string>) => {
    setStored(THUMB_KEY, JSON.stringify(next));
    setThumbs(next);
  };
  // Once, the Warp fork's thumbnails fill the projects that have none yet.
  useEffect(() => {
    if (getStored(LEGACY_KEY)) return;
    const off = conn.onMessage(async (message) => {
      if (message.t !== 'legacy_thumbnails') return;
      off();
      const next = stored<Record<string, string>>(THUMB_KEY, {});
      const failed: string[] = [];
      for (const [key, png] of Object.entries(message.thumbnails)) {
        if (next[key]) continue;
        try {
          next[key] = await thumbnailOf(new Blob([decodeBase64(png)], { type: 'image/png' }));
        } catch (error) {
          failed.push(`${groupOf(key)?.name ?? key} (${error instanceof Error ? error.message : String(error)})`);
        }
      }
      saveThumbs(next);
      setStored(LEGACY_KEY, '1');
      if (failed.length) setThumbError(`옛 나루 썸네일을 읽지 못했어요: ${failed.join(', ')}`);
    });
    conn.send({ t: 'legacy_thumbnails' });
    return off;
  }, [conn]);
  const pickThumb = async (key: string) => {
    const [file] = await pickFiles(IMAGE, false).catch((error) => {
      setThumbError(`이미지를 고르지 못했어요: ${error instanceof Error ? error.message : String(error)}`);
      return [];
    });
    if (!file) return;
    try {
      saveThumbs({ ...thumbs, [key]: await thumbnailOf(await file.blob()) });
    } catch (error) {
      setThumbError(`이미지를 읽지 못했어요: ${file.name} (${error instanceof Error ? error.message : String(error)})`);
    }
  };
  const groupOf = (key: string) => ws.groups.find((g) => String(g.id) === key);

  const tile = (key: string, label: string, hint: string | null, content: React.ReactNode, menu: React.ReactNode, drag = false) => {
    const image = thumbs[key];
    const on = !library && !marketing && effective === key;
    const dot = waiting.get(key);
    return (
      <Menu.Root key={key}>
        <Tooltip>
          <TooltipTrigger asChild>
            <Menu.Trigger asChild>
              <button
                aria-label={label}
                draggable={drag}
                onDragStart={() => setDragging(key)}
                onDragOver={(event) => drag && event.preventDefault()}
                onDrop={() => drop(key)}
                onDragEnd={() => setDragging(null)}
                onClick={() => onSelect(key)}
                className={cn(
                  'relative grid size-9 shrink-0 place-items-center rounded-[11px] text-[13px] font-semibold',
                  image
                    ? cn('bg-background', on ? 'ring-2 ring-primary ring-offset-2 ring-offset-canvas' : 'hover:opacity-80')
                    : on
                      ? 'bg-primary text-primary-foreground'
                      : 'bg-background hover:bg-muted',
                  dragging === key && 'opacity-40',
                )}
              >
                {image ? <img src={image} alt="" draggable={false} className="size-full rounded-[11px] object-cover" /> : content}
                {dot && <i className={cn('absolute top-1 right-1 size-2 rounded-full ring-2', dot === 'turn' ? 'bg-turn' : 'bg-destructive', on ? 'ring-primary' : 'ring-background')} />}
              </button>
            </Menu.Trigger>
          </TooltipTrigger>
          <TooltipContent side="right">
            {label}
            {hint && <kbd className="ml-2 font-mono opacity-70">{hint}</kbd>}
          </TooltipContent>
        </Tooltip>
        <Menu.Portal>
          <Menu.Content className="z-50 min-w-40 rounded-xl bg-popover p-1 text-popover-foreground shadow-lg ring-1 ring-border">{menu}</Menu.Content>
        </Menu.Portal>
      </Menu.Root>
    );
  };

  return (
    <nav className="flex w-14 shrink-0 flex-col items-center bg-canvas pb-3">
      <div data-tauri-drag-region className="h-10 w-full shrink-0" />
      <div className="flex min-h-0 flex-1 flex-col items-center gap-2 overflow-y-auto">
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              aria-label="서재"
              onClick={onLibrary}
              className={cn(
                'grid size-9 shrink-0 place-items-center rounded-[11px]',
                library ? 'bg-primary text-primary-foreground' : 'bg-background hover:bg-muted',
              )}
            >
              <BookOpen className="size-4" />
            </button>
          </TooltipTrigger>
          <TooltipContent side="right">서재</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              aria-label="마케팅"
              onClick={onMarketing}
              className={cn(
                'grid size-9 shrink-0 place-items-center rounded-[11px]',
                marketing ? 'bg-primary text-primary-foreground' : 'bg-background hover:bg-muted',
              )}
            >
              <Megaphone className="size-4" />
            </button>
          </TooltipTrigger>
          <TooltipContent side="right">마케팅</TooltipContent>
        </Tooltip>
        {tile(
          'all',
          '모든 프로젝트',
          '⌥0',
          <Layers className="size-4" />,
          <Menu.Item
            className={menuItem}
            disabled={!hiddenNow.length}
            onSelect={() => saveHidden([])}
          >
            숨긴 프로젝트 보이기{hiddenNow.length ? ` (${hiddenNow.length})` : ''}
          </Menu.Item>,
        )}
        {shown.map(({ key, name }, index) =>
          tile(
            key,
            name,
            index < 9 ? `⌥${index + 1}` : null,
            initials(name) || '?',
            <>
              {groupOf(key) && (
                <Menu.Item className={menuItem} onSelect={() => setRenaming({ key, name })}>
                  이름 바꾸기
                </Menu.Item>
              )}
              <Menu.Item className={menuItem} onSelect={() => void pickThumb(key)}>
                썸네일 고르기…
              </Menu.Item>
              {thumbs[key] && (
                <Menu.Item
                  className={menuItem}
                  onSelect={() => {
                    const { [key]: _gone, ...rest } = thumbs;
                    saveThumbs(rest);
                  }}
                >
                  썸네일 지우기
                </Menu.Item>
              )}
              <Menu.Item className={menuItem} onSelect={() => hide(key)}>
                레일에서 숨기기
              </Menu.Item>
              <Menu.Item
                className={menuItem}
                onSelect={() => {
                  const group = groupOf(key);
                  send(group ? { t: 'new_tab', group: group.id, ...(group.path ? { cwd: group.path } : {}) } : { t: 'new_tab' });
                }}
              >
                새 세션
              </Menu.Item>
            </>,
            true,
          ),
        )}
      </div>
      <Tooltip>
        <TooltipTrigger asChild>
          <button aria-label="프로젝트 더하기" onClick={() => setAdding('')} className="grid size-9 shrink-0 place-items-center rounded-[11px] bg-background text-muted-foreground hover:bg-muted hover:text-foreground">
            <Plus className="size-4" />
          </button>
        </TooltipTrigger>
        <TooltipContent side="right">프로젝트 더하기</TooltipContent>
      </Tooltip>

      <Dialog open={thumbError !== null} onOpenChange={(open) => !open && setThumbError(null)}>
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>썸네일 이미지</DialogTitle>
          </DialogHeader>
          <p className="mt-2 text-muted-foreground">{thumbError}</p>
          <DialogFooter>
            <Button onClick={() => setThumbError(null)}>닫기</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={renaming !== null} onOpenChange={(open) => !open && setRenaming(null)}>
        <DialogContent className="max-w-sm">
          <form
            onSubmit={(event) => {
              event.preventDefault();
              const group = renaming && groupOf(renaming.key);
              if (group && renaming.name.trim()) send({ t: 'rename_group', group: group.id, name: renaming.name.trim() });
              setRenaming(null);
            }}
          >
            <DialogHeader>
              <DialogTitle>프로젝트 이름 바꾸기</DialogTitle>
            </DialogHeader>
            <Input autoFocus className="my-4" value={renaming?.name ?? ''} onChange={(event) => setRenaming((r) => r && { ...r, name: event.target.value })} />
            <DialogFooter>
              <Button type="submit" disabled={!renaming?.name.trim()}>
                바꾸기
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <Dialog open={adding !== null} onOpenChange={(open) => !open && setAdding(null)}>
        <DialogContent className="max-w-sm">
          <form
            onSubmit={(event) => {
              event.preventDefault();
              const path = (adding ?? '').trim().replace(/\/+$/, '');
              const name = basename(path);
              if (path.startsWith('/') && name) send({ t: 'add_group', name, path });
              setAdding(null);
            }}
          >
            <DialogHeader>
              <DialogTitle>프로젝트 더하기</DialogTitle>
            </DialogHeader>
            <p className="mt-2 text-muted-foreground">프로젝트 폴더의 전체 경로를 적어 주세요. 폴더 이름이 프로젝트 이름이 돼요.</p>
            <Input autoFocus className="my-4" placeholder="/Users/me/projects/naru" value={adding ?? ''} onChange={(event) => setAdding(event.target.value)} />
            <DialogFooter>
              <Button type="submit" disabled={!(adding ?? '').trim().startsWith('/') || !basename((adding ?? '').trim())}>
                더하기
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </nav>
  );
}
