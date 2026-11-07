import { useEffect, useRef } from 'react';

const KEY_NAMES: Record<string, string> = { BracketLeft: '[', BracketRight: ']', Comma: ',', Slash: '/', Tab: 'tab' };

/** `mod+alt+shift+k` for the key pressed, by physical key so ⌥ and Hangul input don't change it. */
export function comboOf(event: KeyboardEvent): string | null {
  const key = KEY_NAMES[event.code] ?? event.code.match(/^(?:Key|Digit)(.)$/)?.[1]?.toLowerCase();
  if (!key) return null;
  return [event.metaKey && 'mod', event.altKey && 'alt', event.shiftKey && 'shift', key].filter(Boolean).join('+');
}

/**
 * App shortcuts, run before the terminal or a field sees the key. Fill the returned bindings
 * during render; a key reads them when pressed.
 */
export function useShortcuts() {
  const bindings = useRef<Record<string, () => void>>({});
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      // ⌥ alone takes only the keys bound here; the terminal keeps ⌥b, ⌥f and the rest.
      if (!(event.metaKey || event.altKey) || event.ctrlKey || event.isComposing) return;
      const action = bindings.current[comboOf(event) ?? ''];
      if (!action) return;
      event.preventDefault();
      event.stopPropagation();
      action();
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, []);
  return bindings;
}

/** What the ⌘/ sheet lists. Keep it in step with the bindings in App.tsx and ime.ts. */
export const SHORTCUT_GROUPS: [string, [string, string][]][] = [
  [
    '세션',
    [
      ['검색', '⌘K'],
      ['새 세션', '⌘T'],
      ['오른쪽으로 나누기', '⌘D'],
      ['아래로 나누기', '⌘⇧D'],
      ['pane 닫기', '⌘W'],
      ['프로젝트 안 세션', '⌘1–9'],
      ['이전·다음 세션', '⌘⇧[ ]'],
      ['이전·다음 pane', '⌘[ ]'],
      ['이 pane에서 찾기', '⌘F'],
    ],
  ],
  [
    '프로젝트',
    [
      ['프로젝트 고르기', '⌥1–9'],
      ['모든 프로젝트', '⌥0'],
      ['최근 프로젝트', '⌥Tab'],
      ['이전·다음 프로젝트', '⌘⌥[ ]'],
    ],
  ],
  [
    '보기',
    [
      ['한눈에 보기', '⌘⌥G'],
      ['Brain', '⌘⌥B'],
      ['학습함에 담기', '⌘⇧L'],
      ['설정', '⌘,'],
      ['단축키', '⌘/'],
    ],
  ],
  [
    '터미널',
    [
      ['줄바꿈 (에이전트 입력)', '⇧↩'],
      ['링크·파일 열기', '⌘클릭'],
      ['찾기: 다음·이전', '↩ ⇧↩'],
    ],
  ],
];
