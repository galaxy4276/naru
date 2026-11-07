import { useEffect, useState, useSyncExternalStore } from 'react';
import { ContextMenu as Menu } from 'radix-ui';
import { type ClientMsg, type Connection, type Tab, decodeBase64 } from '@/connection';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { IMAGE, pickFiles } from './pick';
import { menuItem } from './ProjectRail';

const WIDTH = 768;

/** Object URLs of background images by path: `null` while asked for, an Error when unshowable. */
// ponytail: URLs are never revoked; a handful of 2:1 images per session list.
const images = new Map<string, string | null | Error>();
const listeners = new Set<() => void>();
let daemon: Connection | null = null;

const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => listeners.delete(listener);
};

/** Lets session cards ask `conn` for their background images. */
export function syncBackgrounds(conn: Connection) {
  // On open, not here: development builds make a second connection that never starts.
  conn.onOpen(() => {
    daemon = conn;
  });
  conn.onMessage((message) => {
    if (message.t !== 'background') return;
    if (message.data && message.mime) {
      images.set(message.path, URL.createObjectURL(new Blob([decodeBase64(message.data)], { type: message.mime })));
    } else {
      images.set(message.path, new Error(message.error ?? 'unknown'));
      console.warn(`session background ${message.path} cannot be shown: ${message.error}`);
    }
    listeners.forEach((listener) => listener());
  });
}

function useBackground(tab: Tab): string | null {
  const image = useSyncExternalStore(subscribe, () => (tab.background ? images.get(tab.background) : undefined));
  useEffect(() => {
    if (!tab.background || images.has(tab.background) || !daemon?.isOpen) return;
    images.set(tab.background, null);
    daemon.send({ t: 'read_background', tab: tab.id });
  }, [tab.background, tab.id]);
  return typeof image === 'string' ? image : null;
}

/** 시안 B: the image fills the card's right half and melts into the card leftwards. */
export function CardBackground({ tab }: { tab: Tab }) {
  const image = useBackground(tab);
  if (!image) return null;
  return (
    <>
      <div
        aria-hidden
        className="pointer-events-none absolute inset-y-0 right-0 left-[45%] bg-cover bg-center [mask-image:linear-gradient(90deg,transparent,#000_70%)]"
        style={{ backgroundImage: `url(${image})` }}
      />
      <div aria-hidden className="pointer-events-none absolute inset-0 bg-linear-to-r from-background from-40% to-background/55" />
    </>
  );
}

/** Centre-crops an image to 2:1, at most 768 wide, as a base64 JPEG. */
async function backgroundOf(file: Blob): Promise<string> {
  const bitmap = await createImageBitmap(file);
  const w = Math.min(bitmap.width, bitmap.height * 2);
  const canvas = document.createElement('canvas');
  canvas.width = Math.min(WIDTH, Math.round(w));
  canvas.height = Math.round(canvas.width / 2);
  const context = canvas.getContext('2d');
  if (!context || !canvas.height) throw new Error('이미지를 그릴 수 없어요');
  context.drawImage(bitmap, (bitmap.width - w) / 2, (bitmap.height - w / 2) / 2, w, w / 2, 0, 0, canvas.width, canvas.height);
  bitmap.close();
  return canvas.toDataURL('image/jpeg', 0.88).split(',')[1];
}

/** Right-click on a session card: pick or clear its background image. */
export function BackgroundMenu({ tab, send, onClose, children }: { tab: Tab; send: (message: ClientMsg) => void; onClose?: () => void; children: React.ReactElement }) {
  const [error, setError] = useState<string | null>(null);
  const pick = async () => {
    const [file] = await pickFiles(IMAGE, false).catch((failure) => {
      setError(`이미지를 고르지 못했어요: ${failure instanceof Error ? failure.message : String(failure)}`);
      return [];
    });
    if (!file) return;
    try {
      send({ t: 'set_background', tab: tab.id, image: await backgroundOf(await file.blob()) });
    } catch (failure) {
      setError(`이미지를 읽지 못했어요: ${file.name} (${failure instanceof Error ? failure.message : String(failure)})`);
    }
  };
  return (
    <>
      <Menu.Root>
        <Menu.Trigger asChild>{children}</Menu.Trigger>
        <Menu.Portal>
          <Menu.Content className="z-50 min-w-40 rounded-xl bg-popover p-1 text-popover-foreground shadow-lg ring-1 ring-border">
            <Menu.Item className={menuItem} onSelect={() => void pick()}>
              배경 이미지 고르기…
            </Menu.Item>
            <Menu.Item className={menuItem} disabled={!tab.background} onSelect={() => send({ t: 'set_background', tab: tab.id, image: null })}>
              배경 지우기
            </Menu.Item>
            {onClose && (
              <>
                <Menu.Separator className="mx-1.5 my-1 h-px bg-border" />
                <Menu.Item className={menuItem} onSelect={onClose}>
                  세션 닫기
                </Menu.Item>
              </>
            )}
          </Menu.Content>
        </Menu.Portal>
      </Menu.Root>
      <Dialog open={error !== null} onOpenChange={(open) => !open && setError(null)}>
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>배경 이미지</DialogTitle>
          </DialogHeader>
          <p className="mt-2 text-muted-foreground">{error}</p>
          <DialogFooter>
            <Button onClick={() => setError(null)}>닫기</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
