import type { Connection } from '@/connection';

/**
 * Screen settings under `naru.*` keys. The daemon keeps them (`ui.json`), so the installed app and
 * a development screen see the same ones; localStorage keeps a copy for the first paint.
 */
const values = new Map<string, string>();
for (let i = 0; i < localStorage.length; i++) {
  const key = localStorage.key(i);
  const value = key?.startsWith('naru.') ? localStorage.getItem(key) : null;
  if (key && value !== null) values.set(key, value);
}
const listeners = new Map<string, Set<() => void>>();
// Changed while no daemon was listening: these win over the daemon's copy on the next connect.
const unsent = new Set<string>();
let daemon: Connection | null = null;

export function getStored(key: string): string | null {
  return values.get(key) ?? null;
}

/** Saves `value` under `key`, or forgets the key for `null`. */
export function setStored(key: string, value: string | null) {
  if (value === null) values.delete(key);
  else values.set(key, value);
  mirror(key, value);
  if (daemon?.isOpen) daemon.send({ t: 'ui_set', key, value });
  else unsent.add(key);
  listeners.get(key)?.forEach((listener) => listener());
}

/** Runs `listener` whenever `key` changes, here or by the daemon's copy arriving. */
export function onStored(key: string, listener: () => void): () => void {
  const set = listeners.get(key) ?? new Set();
  listeners.set(key, set);
  set.add(listener);
  return () => set.delete(listener);
}

/** On every connect of `conn`: takes the daemon's settings and hands it the ones it lacks. */
export function syncStored(conn: Connection) {
  conn.onMessage((message) => {
    if (message.t !== 'ui') return;
    daemon = conn;
    const changed: string[] = [];
    for (const [key, value] of Object.entries(message.values)) {
      if (unsent.has(key) || values.get(key) === value) continue;
      values.set(key, value);
      mirror(key, value);
      changed.push(key);
    }
    for (const key of unsent) conn.send({ t: 'ui_set', key, value: values.get(key) ?? null });
    for (const [key, value] of values) if (!unsent.has(key) && !(key in message.values)) conn.send({ t: 'ui_set', key, value });
    unsent.clear();
    for (const key of changed) listeners.get(key)?.forEach((listener) => listener());
  });
}

function mirror(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch (error) {
    // The daemon's copy is the one that counts; this one only speeds up the first paint.
    console.warn(`localStorage copy of ${key} not kept`, error);
  }
}
