import { useState } from 'react';
import { Check } from 'lucide-react';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog';
import { cn } from '@/lib/utils';
import { THEMES, type ThemeKey, setTheme, useTheme } from './theme';

type Tone = 'all' | 'light' | 'dark';
const TONES: { key: Tone; label: string }[] = [
  { key: 'all', label: '전체' },
  { key: 'light', label: '밝은' },
  { key: 'dark', label: '어두운' },
];

/** A shrunken Naru screen in a theme's colors: sidebar cards, a pane and the rail. */
function Screen({ dark, darkSidebar, className, style }: { dark: boolean; darkSidebar: boolean; className?: string; style?: React.CSSProperties }) {
  const bar = (width: string, color = 'bg-muted-foreground/35') => <i className={cn('block h-1 rounded-full', color)} style={{ width }} />;
  return (
    <div className={cn(dark && 'dark', 'absolute inset-0 flex gap-1 bg-canvas p-1.5', className)} style={style}>
      <div className={cn(darkSidebar && !dark && 'dark', 'flex w-[28%] flex-col gap-1 rounded-md bg-canvas p-0.5')}>
        <i className="mb-0.5 block h-2.5 rounded bg-background" />
        {['bg-turn', 'border-[1.5px] border-foreground border-t-transparent', 'bg-foreground'].map((ring, index) => (
          <div key={index} className={cn('flex items-center gap-1 rounded bg-background px-1 py-1', index === 1 && 'ring-1 ring-foreground')}>
            <i className={cn('block size-1.5 shrink-0 rounded-full', ring)} />
            {bar(index === 0 ? '70%' : '55%', 'bg-foreground/70')}
          </div>
        ))}
      </div>
      <div className="flex flex-1 flex-col gap-1 rounded-lg bg-background p-1.5">
        {bar('45%', 'bg-foreground/80')}
        <span className="mt-1 flex flex-col gap-1">
          {bar('80%')}
          {bar('60%', 'bg-green-600 dark:bg-green-400')}
          {bar('72%')}
          {bar('40%', 'bg-blue-700 dark:bg-blue-400')}
          {bar('66%')}
        </span>
      </div>
      <div className="w-2.5 rounded-md bg-background" />
    </div>
  );
}

/** Pick how Naru looks; a pick applies at once. */
export function ThemeGallery({ onClose }: { onClose: () => void }) {
  const { key: chosen } = useTheme();
  const [tone, setTone] = useState<Tone>('all');
  const shown = THEMES.filter((theme) => tone === 'all' || theme.tone === tone || theme.tone === 'both');
  const preview = (key: ThemeKey) =>
    key === 'system' ? (
      <>
        <Screen dark={false} darkSidebar={false} />
        <Screen dark darkSidebar={false} style={{ clipPath: 'polygon(58% 0, 100% 0, 100% 100%, 42% 100%)' }} />
      </>
    ) : (
      <Screen dark={key === 'dark'} darkSidebar={key === 'black-sidebar'} />
    );
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="gap-3 rounded-2xl border-0 p-5 sm:max-w-[720px]">
        <div className="flex items-center gap-2">
          <DialogTitle className="mr-2 text-[15px]">테마</DialogTitle>
          {TONES.map((item) => (
            <button
              key={item.key}
              onClick={() => setTone(item.key)}
              className={cn('h-6 rounded-lg px-2.5 text-[12px]', tone === item.key ? 'bg-primary text-primary-foreground' : 'bg-muted text-muted-foreground')}
            >
              {item.label}
            </button>
          ))}
        </div>
        <DialogDescription className="sr-only">누르면 바로 바뀌어요.</DialogDescription>
        <div className="grid grid-cols-2 gap-3">
          {shown.map((theme) => (
            <button
              key={theme.key}
              onClick={() => setTheme(theme.key)}
              aria-pressed={chosen === theme.key}
              className={cn('rounded-2xl p-1.5 text-left transition-shadow hover:bg-muted', chosen === theme.key && 'ring-2 ring-primary')}
            >
              <div className="relative aspect-[16/10] overflow-hidden rounded-xl shadow-[0_1px_3px_rgba(9,9,11,0.12)]">{preview(theme.key)}</div>
              <div className="flex items-center gap-1.5 px-1.5 pt-2 pb-1">
                <b className="font-semibold">{theme.name}</b>
                {chosen === theme.key && <Check className="size-3.5" />}
              </div>
              <p className="px-1.5 pb-1 text-[12px] text-muted-foreground">{theme.note}</p>
            </button>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  );
}
