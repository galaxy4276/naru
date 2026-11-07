import { Check, SquareTerminal } from 'lucide-react';
import { cn } from '@/lib/utils';
import { type Tone, toneLabel } from './model';

/** The state ring. Shape differs by state too, so it reads without color. */
export function Ring({ tone, className }: { tone: Tone | null; className?: string }) {
  const base = 'inline-grid size-[11px] shrink-0 place-items-center rounded-full';
  switch (tone) {
    case null:
      return <SquareTerminal className={cn('size-3.5 shrink-0 text-muted-foreground', className)} />;
    case 'work':
      return <i role="img" aria-label={toneLabel[tone]} className={cn(base, 'animate-spin border-2 border-current border-t-transparent', className)} />;
    case 'turn':
      return <i role="img" aria-label={toneLabel[tone]} className={cn(base, 'bg-turn', className)} />;
    case 'err':
      return <i role="img" aria-label={toneLabel[tone]} className={cn(base, 'bg-destructive', className)} />;
    case 'done':
      return (
        <i role="img" aria-label={toneLabel[tone]} className={cn(base, 'bg-current', className)}>
          <Check className="size-2 text-background" strokeWidth={4} />
        </i>
      );
    case 'wait':
      return <i role="img" aria-label={toneLabel[tone]} className={cn(base, 'border-[1.5px] border-wait', className)} />;
  }
}

export function StatusChip({ tone }: { tone: Tone }) {
  return (
    <span className="inline-flex h-5 shrink-0 items-center gap-1.5 rounded-md border bg-background px-2 text-[11.5px] font-medium">
      <Ring tone={tone} className="size-2 border-[1.5px]" />
      {toneLabel[tone]}
    </span>
  );
}
