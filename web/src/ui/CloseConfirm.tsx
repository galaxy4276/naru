import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';

interface Props {
  busy: string[] | null;
  onCancel: () => void;
  onOnly: () => void;
  onAll: () => void;
}

/** Asks before a close would leave working or asking children without their parent. */
export function CloseConfirm({ busy, onCancel, onOnly, onAll }: Props) {
  return (
    <Dialog open={busy !== null} onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="rounded-2xl border-0 sm:max-w-[420px]" showCloseButton={false}>
        <DialogHeader>
          <DialogTitle className="text-[15px]">자식 에이전트 {busy?.length}개가 아직 돌고 있어요</DialogTitle>
          <DialogDescription>이것만 닫으면 자식은 부모 없이 계속 돌아요.</DialogDescription>
        </DialogHeader>
        <ul className="m-0 flex list-none flex-col gap-1 rounded-xl bg-muted p-3 text-[13px]">
          {busy?.map((line) => <li key={line}>{line}</li>)}
        </ul>
        <DialogFooter>
          <Button variant="secondary" onClick={onCancel}>
            취소
          </Button>
          <Button variant="secondary" onClick={onOnly}>
            이것만 닫기
          </Button>
          <Button variant="destructive" onClick={onAll}>
            모두 닫기
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
