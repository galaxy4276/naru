import { useState } from 'react';
import { getStored, setStored } from './stored';

/** A floating window's spot and size, kept as a screen setting. */
export interface Frame {
  x: number;
  y: number;
  w: number;
  h: number;
}

export function loadFrame(key: string, fallback: () => Frame): Frame {
  try {
    const saved = JSON.parse(getStored(key) ?? '') as Frame;
    if ([saved.x, saved.y, saved.w, saved.h].every(Number.isFinite)) return saved;
  } catch {
    // Nothing kept yet: the fallback.
  }
  return fallback();
}

/** The frame moved back inside the window, as far as it fits, and no smaller than `min`. */
export function fitted(frame: Frame, min: { w: number; h: number }): Frame {
  const w = Math.max(min.w, Math.min(frame.w, window.innerWidth));
  const h = Math.max(min.h, Math.min(frame.h, window.innerHeight));
  return { w, h, x: Math.max(0, Math.min(frame.x, window.innerWidth - w)), y: Math.max(0, Math.min(frame.y, window.innerHeight - h)) };
}

/** Pointer handling for a frame's head (move) or corner (resize); saves where it ends. */
export function grabFrame(event: React.PointerEvent, frame: Frame, resize: boolean, min: { w: number; h: number }, key: string, set: (frame: Frame) => void) {
  if ((event.target as HTMLElement).closest('button') && !resize) return;
  event.preventDefault();
  const start = { x: event.clientX, y: event.clientY };
  const at = (e: PointerEvent): Frame => {
    const dx = e.clientX - start.x;
    const dy = e.clientY - start.y;
    return fitted(resize ? { ...frame, w: frame.w + dx, h: frame.h + dy } : { ...frame, x: frame.x + dx, y: frame.y + dy }, min);
  };
  const move = (e: PointerEvent) => set(at(e));
  const up = (e: PointerEvent) => {
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', up);
    setStored(key, JSON.stringify(at(e)));
  };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up);
}

/** A panel width dragged from its `edge` handle, kept as the screen setting `key`. */
export function useEdgeWidth(key: string, initial: number, min: number, max: number, edge: 'left' | 'right') {
  const [width, setWidth] = useState(() => Number(getStored(key)) || initial);
  const grab = (event: React.PointerEvent) => {
    event.preventDefault();
    const start = event.clientX;
    const at = (e: PointerEvent) => Math.min(max, Math.max(min, width + (edge === 'right' ? e.clientX - start : start - e.clientX)));
    const move = (e: PointerEvent) => setWidth(at(e));
    const up = (e: PointerEvent) => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      setStored(key, String(at(e)));
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  };
  return [width, grab] as const;
}
