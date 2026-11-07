import { useEffect, useState } from 'react';
import { ChevronDown, ChevronRight, RefreshCw } from 'lucide-react';
import type { ClientMsg, Connection, Costs, ProjectCost } from '@/connection';
import { Button } from '@/components/ui/button';
import { SidePanel } from './SidePanel';

const usd = (amount: number) => `$${amount < 10 ? amount.toFixed(2) : Math.round(amount).toLocaleString()}`;

function tokens(count: number): string {
  if (count >= 1e9) return `${(count / 1e9).toFixed(1)}B`;
  if (count >= 1e6) return `${(count / 1e6).toFixed(1)}M`;
  if (count >= 1e3) return `${Math.round(count / 1e3)}K`;
  return String(count);
}

/** `≥` when some calls had no price, so the amount is only a floor. */
const amount = (cost: number, unpriced: number) => `${unpriced ? '≥ ' : ''}${usd(cost)}`;

function Bars({ days, values }: { days: string[]; values: number[] }) {
  const top = Math.max(...values, 0);
  return (
    <div className="flex h-10 items-end gap-1" aria-label="하루별 비용">
      {values.map((value, index) => (
        <div key={days[index]} className="flex flex-1 flex-col items-center gap-0.5" title={`${days[index]} · ${usd(value)}`}>
          <div className="w-full rounded-[3px] bg-foreground/70" style={{ height: top ? `${Math.max((value / top) * 28, value ? 2 : 0)}px` : 0 }} />
          <span className="text-[9.5px] text-muted-foreground">{days[index].slice(3)}</span>
        </div>
      ))}
    </div>
  );
}

function Project({ project, days, send }: { project: ProjectCost; days: string[]; send: (message: ClientMsg) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <li className="flex flex-col gap-2 border-t border-muted px-2 py-3 first:border-t-0">
      <button onClick={() => setOpen((o) => !o)} className="flex items-center gap-1.5 text-left" aria-expanded={open}>
        {open ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
        <b className="min-w-0 flex-1 truncate">{project.name}</b>
        <b className="font-mono text-[14px]">{amount(project.total, project.unpriced)}</b>
      </button>
      <Bars days={days} values={project.days} />
      <div className="text-[11.5px] text-muted-foreground">
        세션 {project.sessions}개{project.unpriced ? ` · 가격 없는 호출 ${project.unpriced}회는 빠졌어요` : ''}
      </div>
      {open && (
        <>
          <ul className="m-0 flex list-none flex-col gap-0.5 p-0 text-[12px]">
            {project.models.map((model) => (
              <li key={model.model} className="flex gap-2">
                <code className="min-w-0 flex-1 truncate">{model.model}</code>
                <span className="text-muted-foreground">{tokens(model.tokens)}</span>
                <span className="w-16 text-right font-mono">{model.cost === null ? '가격 없음' : usd(model.cost)}</span>
              </li>
            ))}
          </ul>
          <div className="text-[11.5px] font-semibold text-muted-foreground">비싼 작업</div>
          <ul className="m-0 flex list-none flex-col gap-1 p-0 text-[12px]">
            {project.top.map((session, index) => (
              <li key={index} className="flex items-center gap-2">
                <span className="w-11 shrink-0 text-muted-foreground">{session.agent}</span>
                {session.pane ? (
                  <button onClick={() => send({ t: 'focus', pane: session.pane! })} className="min-w-0 flex-1 truncate text-left hover:underline" title={session.title}>
                    {session.title}
                  </button>
                ) : (
                  <span className="min-w-0 flex-1 truncate" title={session.title}>
                    {session.title}
                  </span>
                )}
                <span className="font-mono">{amount(session.cost, session.unpriced)}</span>
              </li>
            ))}
          </ul>
        </>
      )}
    </li>
  );
}

/** What the agents spent in each project over the last seven days, at API prices. */
export function CostPanel({ conn, send, onClose }: { conn: Connection; send: (message: ClientMsg) => void; onClose: () => void }) {
  const [costs, setCosts] = useState<Costs | null>(null);
  const [reading, setReading] = useState(true);

  const read = () => {
    setReading(true);
    conn.send({ t: 'costs' });
  };
  useEffect(() => {
    const off = conn.onMessage((message) => {
      if (message.t !== 'costs') return;
      setCosts(message.costs);
      setReading(false);
    });
    conn.send({ t: 'costs' });
    return off;
  }, [conn]);

  const sum = costs?.projects.reduce((total, project) => total + project.total, 0) ?? 0;
  const unpriced = costs?.projects.reduce((total, project) => total + project.unpriced, 0) ?? 0;
  return (
    <SidePanel
      title="비용"
      sub={costs ? `최근 7일 · ${amount(sum, unpriced)}` : undefined}
      actions={
        <Button variant="ghost" size="icon-xs" onClick={read} disabled={reading} aria-label="다시 읽기">
          <RefreshCw className={reading ? 'animate-spin' : undefined} />
        </Button>
      }
      onClose={onClose}
    >
      {!costs && <p className="px-4 text-muted-foreground">대화 기록을 읽는 중이에요</p>}
      {costs?.projects.length === 0 && <p className="px-4 text-muted-foreground">최근 7일 동안 에이전트가 쓴 기록이 없어요</p>}
      <ul className="m-0 flex list-none flex-col px-2 pb-2">
        {costs?.projects.map((project) => <Project key={project.path ?? ''} project={project} days={costs.days} send={send} />)}
      </ul>
      {costs && (
        <p className="px-4 pb-4 text-[11px] leading-relaxed text-muted-foreground">
          이 기기의 Claude·Codex 대화 기록을 API 가격으로 셈한 값이에요. 구독 요금과 다르고, Codex 가격은 OpenAI 공식 표가 아닌 제3자 표(2026-11-07)를 따랐어요.
        </p>
      )}
    </SidePanel>
  );
}
