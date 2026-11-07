import { useSyncExternalStore } from 'react';
import { getStored, onStored, setStored } from './stored';

export type ThemeKey = 'light' | 'black-sidebar' | 'dark' | 'system';

export const THEMES: { key: ThemeKey; name: string; note: string; tone: 'light' | 'dark' | 'both' }[] = [
  { key: 'light', name: '흰 카드', note: '회색 판 위 흰 카드. 기본', tone: 'light' },
  { key: 'black-sidebar', name: '검은 사이드바', note: '사이드바만 어둡고 작업 면은 흰색', tone: 'light' },
  { key: 'dark', name: '다크', note: '화면과 터미널 모두 검정', tone: 'dark' },
  { key: 'system', name: '시스템 따라가기', note: 'macOS 밝기 설정에 맞춰 흰 카드와 다크를 오간다', tone: 'both' },
];

const KEY = 'naru.theme';
const media = window.matchMedia('(prefers-color-scheme: dark)');
const listeners = new Set<() => void>();
const saved = (): ThemeKey => THEMES.find((theme) => theme.key === getStored(KEY))?.key ?? 'light';
let current = saved();

export function isDark(key: ThemeKey = current): boolean {
  return key === 'dark' || (key === 'system' && media.matches);
}

function apply() {
  document.documentElement.classList.toggle('dark', isDark());
  listeners.forEach((listener) => listener());
}

export function setTheme(key: ThemeKey) {
  current = key;
  setStored(KEY, key);
  apply();
}

/** Runs `listener` whenever the screen turns light or dark. */
export function onTheme(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** The chosen theme and whether the screen is dark right now. */
export function useTheme(): { key: ThemeKey; dark: boolean } {
  const key = useSyncExternalStore(onTheme, () => current);
  const dark = useSyncExternalStore(onTheme, () => isDark());
  return { key, dark };
}

media.addEventListener('change', () => current === 'system' && apply());
onStored(KEY, () => {
  current = saved();
  apply();
});
apply();
