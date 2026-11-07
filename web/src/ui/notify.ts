import { useEffect, useRef } from 'react';
import type { PaneId, Workspace } from '@/connection';
import { type NotifyEvent, type Teams, newAlerts } from './model';
import { getPrefs } from './prefs';

export function askNotifyPermission() {
  if (typeof Notification !== 'undefined' && Notification.permission === 'default') void Notification.requestPermission();
}

/** Raises a macOS notification when an agent starts calling, as the 알림 settings allow. */
export function useNotify(ws: Workspace | null, teams: Teams, online: boolean) {
  const before = useRef<Map<PaneId, NotifyEvent> | null>(null);
  useEffect(() => {
    if (!online || !ws) return;
    // The first state after connecting is where things stand, not news.
    if (!before.current) {
      before.current = new Map();
      newAlerts(before.current, ws, teams);
      return;
    }
    const alerts = newAlerts(before.current, ws, teams);
    const prefs = getPrefs();
    if (prefs.quietInFront && document.hasFocus()) return;
    if (typeof Notification === 'undefined' || Notification.permission === 'denied') return;
    for (const alert of alerts.filter((alert) => prefs.notify[alert.event])) {
      new Notification(alert.title, { body: alert.body, ...(prefs.notifySound ? { sound: 'default' } : {}) } as NotificationOptions);
    }
  }, [ws, teams, online]);
}
