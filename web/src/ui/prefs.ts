import { useSyncExternalStore } from 'react';
import type { AgentKind } from '@/connection';
import { getStored, onStored, setStored } from './stored';
import type { NotifyEvent } from './model';

export const AGENTS: { kind: AgentKind; name: string; mark: string }[] = [
  { kind: 'claude', name: 'Claude', mark: 'Cl' },
  { kind: 'codex', name: 'Codex', mark: 'Cx' },
  { kind: 'agy', name: 'agy', mark: 'Ag' },
  { kind: 'grok', name: 'Grok', mark: 'Gk' },
];

export const TERMINAL_FONTS = ['JetBrains Mono', 'D2Coding', 'Menlo'] as const;
export const UI_SIZE = { min: 11, max: 17, base: 13 };
export const TERM_SIZE = { min: 10, max: 24, base: 13 };

export interface Prefs {
  /** 화면 글자: the sidebar and side panels scale by `ui / 13`. */
  ui: number;
  term: number;
  font: (typeof TERMINAL_FONTS)[number];
  /** Agents the bottom bar shows. */
  bar: Record<AgentKind, boolean>;
  fiveHour: boolean;
  week: boolean;
  /** Which agent events raise a system notification. */
  notify: Record<NotifyEvent, boolean>;
  /** No notification while the Naru window is in front. */
  quietInFront: boolean;
  notifySound: boolean;
}

export const NOTIFY_EVENTS: { key: NotifyEvent; name: string; sub: string; on: boolean }[] = [
  { key: 'ask', name: '에이전트가 질문하거나 권한을 물어요', sub: '내 차례가 될 때', on: true },
  { key: 'done', name: '턴이 끝났어요', sub: '에이전트가 일을 마쳤을 때', on: false },
  { key: 'err', name: '오류가 났어요', sub: '', on: true },
  { key: 'child', name: '자식 에이전트가 막혔어요', sub: '부모가 풀지 못할 때', on: true },
];

const KEY = 'naru.prefs';
const OLD_ZOOM_KEY = 'naru.sidebar.zoom';
const clamp = (value: number, { min, max }: { min: number; max: number }) => Math.min(max, Math.max(min, Math.round(value)));

function load(): Prefs {
  const saved = JSON.parse(getStored(KEY) ?? 'null') as Partial<Prefs> | null;
  // The sidebar's old −/+ zoom becomes the screen size.
  const zoom = Number(getStored(OLD_ZOOM_KEY)) || 1;
  return {
    ui: clamp(saved?.ui ?? UI_SIZE.base * zoom, UI_SIZE),
    term: clamp(saved?.term ?? TERM_SIZE.base, TERM_SIZE),
    font: TERMINAL_FONTS.find((font) => font === saved?.font) ?? TERMINAL_FONTS[0],
    bar: Object.fromEntries(AGENTS.map(({ kind }) => [kind, saved?.bar?.[kind] ?? true])) as Prefs['bar'],
    fiveHour: saved?.fiveHour ?? true,
    week: saved?.week ?? true,
    notify: Object.fromEntries(NOTIFY_EVENTS.map(({ key, on }) => [key, saved?.notify?.[key] ?? on])) as Prefs['notify'],
    quietInFront: saved?.quietInFront ?? true,
    notifySound: saved?.notifySound ?? false,
  };
}

let current = load();
const listeners = new Set<() => void>();

export function setPrefs(change: Partial<Prefs>) {
  current = { ...current, ...change, ui: clamp(change.ui ?? current.ui, UI_SIZE), term: clamp(change.term ?? current.term, TERM_SIZE) };
  setStored(KEY, JSON.stringify(current));
  setStored(OLD_ZOOM_KEY, null);
  listeners.forEach((listener) => listener());
}

onStored(KEY, () => {
  current = load();
  listeners.forEach((listener) => listener());
});

export function getPrefs(): Prefs {
  return current;
}

export function onPrefs(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function usePrefs(): Prefs {
  return useSyncExternalStore(onPrefs, getPrefs);
}

/** The CSS font stack for the terminal, the chosen font first. */
export function terminalFont(font: Prefs['font']): string {
  return [font, ...TERMINAL_FONTS.filter((other) => other !== font)].map((name) => `"${name}"`).join(', ') + ', monospace';
}
