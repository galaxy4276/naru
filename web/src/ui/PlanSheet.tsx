import type { ClientMsg, PaneId, PlanView, StepState } from '@/connection';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { cn } from '@/lib/utils';

const stateLabel: Record<StepState, string> = {
  waiting: '대기',
  working: '진행 중',
  blocked: '막힘',
  reported: '보고 옴',
  closed: '닫힘',
};

/** `계획 2/5`: steps reported of all. */
export function planChip(plan: PlanView): string {
  if (plan.review) return '계획 검토';
  return `계획 ${plan.steps.filter((step) => step.state === 'reported').length}/${plan.steps.length}`;
}

/** An agent's plan: its steps and where each stands, with the review's approval. */
export function PlanSheet({
  pane,
  plan,
  open,
  send,
  onClose,
}: {
  pane: PaneId;
  plan: PlanView;
  open: boolean;
  send: (message: ClientMsg) => void;
  onClose: () => void;
}) {
  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()}>
      <DialogContent className="top-24 translate-y-0 rounded-2xl border-0 sm:max-w-[560px]" showCloseButton={false}>
        <DialogHeader>
          <DialogTitle className="text-[15px]">{plan.title}</DialogTitle>
          <DialogDescription>
            {plan.review ? '에이전트가 이 계획대로 해도 되는지 기다려요. 고칠 점은 pane에 직접 말해 주세요.' : `${plan.steps.length}단계`}
          </DialogDescription>
        </DialogHeader>
        <ol className="m-0 flex max-h-[50vh] list-none flex-col gap-1 overflow-y-auto p-0">
          {plan.steps.map((step, index) => (
            <li key={step.key} className="flex items-center gap-2 rounded-lg px-2 py-1.5 hover:bg-muted">
              <span className="w-5 shrink-0 text-right text-muted-foreground">{index + 1}</span>
              <span className="min-w-0 flex-1">{step.title}</span>
              {step.child && (
                <button
                  onClick={() => {
                    send({ t: 'focus', pane: step.child! });
                    onClose();
                  }}
                  className="shrink-0 font-mono text-[12px] text-muted-foreground hover:underline"
                >
                  {step.child_id ?? '자식'}
                </button>
              )}
              <span
                className={cn(
                  'shrink-0 rounded-full px-2 py-0.5 text-[11.5px]',
                  step.state === 'blocked' ? 'bg-turn-soft text-turn' : 'bg-muted text-muted-foreground',
                  step.state === 'reported' && 'text-foreground',
                )}
              >
                {stateLabel[step.state]}
              </span>
            </li>
          ))}
        </ol>
        <DialogFooter>
          <Button variant="secondary" onClick={onClose}>
            닫기
          </Button>
          {plan.review && (
            <Button
              onClick={() => {
                send({ t: 'plan_approve', pane });
                onClose();
              }}
            >
              승인
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
