import { useEffect, useRef, useState } from 'react';
import { ChevronRight, RefreshCw } from 'lucide-react';
import type { ClientMsg, Connection, MetaAct, MetaAdAccount, MetaAdsView, MetaLevel, MetaPreset, MetaRow } from '@/connection';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { cn } from '@/lib/utils';
import { SidePanel } from './SidePanel';
import { Toggle } from './Settings';

// Insights lag a few minutes and count against Meta's call limits; five minutes is plenty.
const POLL_MS = 5 * 60_000;
const PRESETS: { key: MetaPreset; label: string }[] = [
  { key: 'today', label: '오늘' },
  { key: 'yesterday', label: '어제' },
  { key: 'last_7d', label: '7일' },
  { key: 'last_30d', label: '30일' },
];
const CHILD: Record<MetaLevel, MetaLevel | null> = { campaign: 'adset', adset: 'ad', ad: null };
const EFFECTIVE: Record<string, string> = {
  CAMPAIGN_PAUSED: '캠페인 꺼짐',
  ADSET_PAUSED: '광고세트 꺼짐',
  PENDING_REVIEW: '검토 중',
  DISAPPROVED: '거절됨',
  WITH_ISSUES: '문제 있음',
  IN_PROCESS: '처리 중',
  PENDING_BILLING_INFO: '결제 정보 필요',
  ARCHIVED: '보관됨',
};
/** A budget change past this ratio either way asks for the new amount again. */
const RETYPE_RATIO = 2;

/** A change waiting in the confirm dialog. */
interface Pending {
  row: MetaRow;
  act: MetaAct;
  before: string;
  after: string;
  /** The amount to type again, for a large budget change. */
  retype: string | null;
}

const number = new Intl.NumberFormat('ko-KR');

function money(amount: number | null | undefined, currency: string) {
  if (amount == null) return '';
  try {
    return new Intl.NumberFormat('ko-KR', { style: 'currency', currency }).format(amount);
  } catch {
    return `${number.format(amount)} ${currency}`;
  }
}

/** Campaigns, ad sets and ads of a Meta ad account, with their insights, pause switches and budgets. */
export function MetaAdsPanel({ conn, send, onSettings, onClose }: { conn: Connection; send: (message: ClientMsg) => void; onSettings: () => void; onClose: () => void }) {
  const [preset, setPreset] = useState<MetaPreset>('last_7d');
  const [account, setAccount] = useState<string | null>(null);
  const [top, setTop] = useState<MetaAdsView | null>(null);
  const [children, setChildren] = useState<Record<string, MetaAdsView>>({});
  const [open, setOpen] = useState<{ id: string; level: MetaLevel }[]>([]);
  const [loading, setLoading] = useState(true);
  const [pending, setPending] = useState<Pending | null>(null);
  const [sending, setSending] = useState<string | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const asked = useRef({ preset, account, open });
  asked.current = { preset, account, open };

  const ask = (level: MetaLevel, parent: string | null) => conn.send({ t: 'meta_ads', account: asked.current.account, level, parent, preset: asked.current.preset });
  const refresh = () => {
    setLoading(true);
    ask('campaign', null);
    for (const { id, level } of asked.current.open) ask(level, id);
  };

  useEffect(() => {
    const off = conn.onMessage((message) => {
      if (message.t === 'meta_ads_done' && message.id !== null) {
        setSending(null);
        setFailure(message.error);
        refresh();
        return;
      }
      if (message.t !== 'meta_ads' || message.preset !== asked.current.preset) return;
      if (message.level === 'campaign') {
        setTop(message);
        setLoading(false);
        if (!asked.current.account && message.account) setAccount(message.account);
      } else if (message.parent) {
        setChildren((old) => ({ ...old, [message.parent!]: message }));
      }
    });
    return off;
  }, [conn]);

  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, POLL_MS);
    return () => clearInterval(timer);
  }, [conn, preset, account]);

  const accounts = top?.accounts ?? [];
  const current: MetaAdAccount | undefined = accounts.find((item) => item.id === (account ?? top?.account));
  const currency = current?.currency ?? '';
  const offset = current?.offset ?? null;

  const toggleOpen = (row: MetaRow) => {
    const level = CHILD[row.level];
    if (!level) return;
    if (open.some((item) => item.id === row.id)) {
      setOpen(open.filter((item) => item.id !== row.id));
      return;
    }
    setOpen([...open, { id: row.id, level }]);
    ask(level, row.id);
  };

  const confirmStatus = (row: MetaRow, on: boolean) => {
    const to = on ? 'ACTIVE' : 'PAUSED';
    setPending({ row, act: { kind: 'status', from: row.status, to }, before: on ? '꺼짐' : '켜짐', after: on ? '켜짐' : '꺼짐', retype: null });
  };
  const confirmBudget = (row: MetaRow, major: number) => {
    if (!row.budget || offset === null) return;
    const to = Math.round(major * offset);
    const from = row.budget.minor;
    if (to <= 0 || to === from) return;
    const large = to > from * RETYPE_RATIO || to * RETYPE_RATIO < from;
    setPending({
      row,
      act: { kind: 'budget', field: row.budget.kind, from, to },
      before: money(from / offset, currency),
      after: money(to / offset, currency),
      retype: large ? String(major) : null,
    });
  };
  const apply = () => {
    if (!pending || !current) return;
    setSending(pending.row.id);
    setFailure(null);
    send({ t: 'meta_ads_act', account: current.id, id: pending.row.id, act: pending.act });
    setPending(null);
  };

  const noToken = top && !top.token_tail && !top.error;
  return (
    <SidePanel
      title="Meta 광고"
      width={720}
      sub={current ? `${current.name} · ${current.currency}` : undefined}
      actions={
        <>
          <span className="flex rounded-lg bg-muted p-0.5">
            {PRESETS.map(({ key, label }) => (
              <button key={key} onClick={() => setPreset(key)} className={cn('h-6 rounded-md px-2 text-[11.5px] font-semibold', preset === key ? 'bg-background shadow-sm' : 'text-muted-foreground')}>
                {label}
              </button>
            ))}
          </span>
          <Button variant="ghost" size="icon-xs" onClick={refresh} aria-label="새로고침">
            <RefreshCw className={cn(loading && 'animate-spin')} />
          </Button>
        </>
      }
      onClose={onClose}
    >
      {accounts.length > 1 && (
        <div className="px-4 pb-2">
          <select
            value={current?.id ?? ''}
            onChange={(event) => {
              setAccount(event.target.value);
              setOpen([]);
              setChildren({});
            }}
            className="h-7 rounded-md border border-input bg-transparent px-2 text-[12.5px]"
          >
            {accounts.map((item) => (
              <option key={item.id} value={item.id}>
                {item.name || item.id} · {item.currency}
                {item.active ? '' : ' (비활성)'}
              </option>
            ))}
          </select>
        </div>
      )}
      {noToken && (
        <div className="flex flex-col items-start gap-2 px-4 py-6 text-muted-foreground">
          설정에서 Meta 시스템 사용자 토큰을 넣어 주세요.
          <Button size="sm" variant="secondary" onClick={onSettings}>
            설정 열기
          </Button>
        </div>
      )}
      {top?.error && <p className="mx-4 mb-2 rounded-lg bg-red-50 px-3 py-2 text-[12.5px] text-red-600 dark:bg-red-950 dark:text-red-400">{top.error}</p>}
      {failure && <p className="mx-4 mb-2 rounded-lg bg-red-50 px-3 py-2 text-[12.5px] text-red-600 dark:bg-red-950 dark:text-red-400">{failure}</p>}
      {current && offset === null && <p className="mx-4 mb-2 text-[12px] text-muted-foreground">{currency} 예산 단위를 몰라서 예산은 고칠 수 없어요.</p>}
      {top && !noToken && !top.error && (
        <table className="w-full text-[12px] tabular-nums">
          <thead className="sticky top-0 bg-background text-[11px] text-muted-foreground">
            <tr className="[&>th]:px-1.5 [&>th]:py-1 [&>th]:text-right [&>th]:font-medium">
              <th className="!text-left !pl-4">이름</th>
              <th>켜기</th>
              <th>예산</th>
              <th>지출</th>
              <th>노출</th>
              <th>클릭</th>
              <th>CTR</th>
              <th>CPC</th>
              <th>결과</th>
              <th className="!pr-4">결과당</th>
            </tr>
          </thead>
          <tbody>
            {top.rows.length === 0 && (
              <tr>
                <td colSpan={10} className="px-4 py-6 text-muted-foreground">
                  캠페인이 없어요
                </td>
              </tr>
            )}
            {top.rows.map((row) => (
              <Tree
                key={row.id}
                row={row}
                depth={0}
                open={open}
                kids={children}
                currency={currency}
                offset={offset}
                sending={sending}
                onToggleOpen={toggleOpen}
                onStatus={confirmStatus}
                onBudget={confirmBudget}
              />
            ))}
          </tbody>
        </table>
      )}
      {pending && <Confirm pending={pending} currency={currency} onCancel={() => setPending(null)} onApply={apply} />}
    </SidePanel>
  );
}

interface TreeProps {
  row: MetaRow;
  depth: number;
  open: { id: string; level: MetaLevel }[];
  kids: Record<string, MetaAdsView>;
  currency: string;
  offset: number | null;
  sending: string | null;
  onToggleOpen: (row: MetaRow) => void;
  onStatus: (row: MetaRow, on: boolean) => void;
  onBudget: (row: MetaRow, major: number) => void;
}

function Tree(props: TreeProps) {
  const { row, depth, open, kids } = props;
  const expanded = open.some((item) => item.id === row.id);
  const view = kids[row.id];
  return (
    <>
      <Line {...props} expanded={expanded} />
      {expanded && !view && (
        <tr>
          <td colSpan={10} className="py-1 text-muted-foreground" style={{ paddingLeft: 32 + (depth + 1) * 16 }}>
            읽는 중이에요
          </td>
        </tr>
      )}
      {expanded && view?.error && (
        <tr>
          <td colSpan={10} className="py-1 text-red-600 dark:text-red-400" style={{ paddingLeft: 32 + (depth + 1) * 16 }}>
            {view.error}
          </td>
        </tr>
      )}
      {expanded && view?.rows.map((child) => <Tree key={child.id} {...props} row={child} depth={depth + 1} />)}
    </>
  );
}

function Line({ row, depth, expanded, currency, offset, sending, onToggleOpen, onStatus, onBudget }: TreeProps & { expanded: boolean }) {
  const [editing, setEditing] = useState<string | null>(null);
  const metrics = row.metrics;
  const note = row.effective_status !== row.status ? EFFECTIVE[row.effective_status] ?? row.effective_status : null;
  const busy = sending === row.id;
  const canToggle = row.status === 'ACTIVE' || row.status === 'PAUSED';
  return (
    <tr className={cn('border-t border-border/60 [&>td]:px-1.5 [&>td]:py-1.5 [&>td]:text-right', busy && 'opacity-50')}>
      <td className="max-w-[200px] !text-left" style={{ paddingLeft: 16 + depth * 16 }}>
        <div className="flex items-center gap-1">
          {CHILD[row.level] ? (
            <button onClick={() => onToggleOpen(row)} aria-label={expanded ? '접기' : '펼치기'} className="shrink-0 text-muted-foreground">
              <ChevronRight className={cn('size-3.5 transition-transform', expanded && 'rotate-90')} />
            </button>
          ) : (
            <span className="w-3.5 shrink-0" />
          )}
          <span className="min-w-0">
            <span className="block truncate" title={row.name}>
              {row.name}
            </span>
            {(note || row.outcome) && <span className="block truncate text-[10.5px] text-muted-foreground">{[note, row.outcome?.label].filter(Boolean).join(' · ')}</span>}
          </span>
        </div>
      </td>
      <td>{canToggle ? <Toggle on={row.status === 'ACTIVE'} label={`${row.name} 켜기`} onChange={(on) => !busy && onStatus(row, on)} /> : <span className="text-muted-foreground">{row.status}</span>}</td>
      <td>
        {row.budget && editing !== null ? (
          <form
            onSubmit={(event) => {
              event.preventDefault();
              const major = Number(editing.replaceAll(',', ''));
              setEditing(null);
              if (Number.isFinite(major) && major > 0) onBudget(row, major);
            }}
          >
            <Input autoFocus value={editing} onChange={(event) => setEditing(event.target.value)} onBlur={() => setEditing(null)} onKeyDown={(event) => event.key === 'Escape' && setEditing(null)} inputMode="decimal" className="h-6 w-24 px-1.5 text-right text-[12px]" />
          </form>
        ) : row.budget ? (
          <button
            disabled={offset === null || busy}
            onClick={() => offset !== null && setEditing(String(row.budget!.minor / offset))}
            className={cn('whitespace-nowrap', offset !== null && 'underline decoration-dotted underline-offset-2')}
            title={row.budget.kind === 'daily' ? '일 예산' : '총 예산'}
          >
            {offset === null ? `${number.format(row.budget.minor)} (최소 단위)` : money(row.budget.minor / offset, currency)}
            <span className="ml-0.5 text-[10px] text-muted-foreground">{row.budget.kind === 'daily' ? '/일' : '총'}</span>
          </button>
        ) : null}
      </td>
      <td className="whitespace-nowrap">{money(metrics?.spend, currency)}</td>
      <td>{metrics?.impressions != null && number.format(metrics.impressions)}</td>
      <td>{metrics?.clicks != null && number.format(metrics.clicks)}</td>
      <td>{metrics?.ctr != null && `${metrics.ctr.toFixed(2)}%`}</td>
      <td className="whitespace-nowrap">{money(metrics?.cpc, currency)}</td>
      <td>{row.outcome && number.format(row.outcome.count)}</td>
      <td className="whitespace-nowrap !pr-4">{money(row.outcome?.cost, currency)}</td>
    </tr>
  );
}

function Confirm({ pending, currency, onCancel, onApply }: { pending: Pending; currency: string; onCancel: () => void; onApply: () => void }) {
  const [typed, setTyped] = useState('');
  const { row, act, before, after, retype } = pending;
  const what = act.kind === 'status' ? (act.to === 'ACTIVE' ? '켜기' : '끄기') : act.field === 'daily' ? '일 예산' : '총 예산';
  const ready = retype === null || Number(typed.replaceAll(',', '')) === Number(retype);
  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="rounded-2xl border-0 sm:max-w-[420px]" showCloseButton={false}>
        <DialogHeader>
          <DialogTitle className="text-[15px]">
            {row.name} {what}
          </DialogTitle>
          <DialogDescription>바로 Meta에 반영돼요. 그사이 다른 곳에서 값이 바뀌었으면 바꾸지 않아요.</DialogDescription>
        </DialogHeader>
        <p className="rounded-xl bg-muted p-3 text-[15px] font-semibold tabular-nums">
          {act.kind === 'budget' && <span className="mr-2 text-[12.5px] font-normal text-muted-foreground">{what}</span>}
          {before} → {after}
        </p>
        {retype !== null && (
          <label className="flex flex-col gap-1 text-[12.5px]">
            2배 넘게 바뀌어요. 확인하려면 새 금액을 다시 적어 주세요 ({currency}).
            <Input autoFocus value={typed} onChange={(event) => setTyped(event.target.value)} inputMode="decimal" placeholder={retype} />
          </label>
        )}
        <DialogFooter>
          <Button variant="secondary" onClick={onCancel}>
            취소
          </Button>
          <Button variant={act.kind === 'budget' ? 'default' : 'secondary'} disabled={!ready} onClick={onApply}>
            {act.kind === 'status' ? what : '예산 바꾸기'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
