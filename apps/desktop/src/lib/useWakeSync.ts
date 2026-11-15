import { useEffect } from 'react';

/** At most one check per this long, however often the window is focused. */
export const WAKE_SYNC_GAP_MS = 30_000;

/**
 * Check for new mail when the app comes back: the window is focused or shown
 * again (a phone resuming, a laptop waking) or the network returns. After
 * sleep or a network change the old connection may be dead; this doesn't
 * wait for it to time out.
 */
export function useWakeSync(syncNow: () => void, now: () => number = Date.now): void {
  useEffect(() => {
    let last = now();
    const check = () => {
      if (document.visibilityState === 'hidden') return;
      const time = now();
      if (time - last < WAKE_SYNC_GAP_MS) return;
      last = time;
      syncNow();
    };
    window.addEventListener('focus', check);
    window.addEventListener('online', check);
    document.addEventListener('visibilitychange', check);
    return () => {
      window.removeEventListener('focus', check);
      window.removeEventListener('online', check);
      document.removeEventListener('visibilitychange', check);
    };
  }, [syncNow, now]);
}
