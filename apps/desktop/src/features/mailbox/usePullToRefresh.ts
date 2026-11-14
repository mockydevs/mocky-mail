import { useEffect, useRef, useState, type RefObject } from 'react';

/** How far (px, after damping) the list must be pulled to refresh on release. */
export const PULL_THRESHOLD = 64;
/** The indicator never grows taller than this. */
const PULL_MAX = 96;
/** Finger travel is halved, so the pull feels like stretching. */
const DAMPING = 0.5;

export type PullState = 'idle' | 'pulling' | 'armed' | 'refreshing';

/**
 * Pull-to-refresh for a scroll container, as in most phone mail apps: drag
 * down while the list is at the top, release past the threshold, and
 * `onRefresh` runs. Touch only, so mice and trackpads are unaffected.
 *
 * Returns how far the list is pulled (px) and what the indicator should say.
 */
export function usePullToRefresh(
  scrollRef: RefObject<HTMLElement | null>,
  onRefresh: (() => Promise<void>) | undefined,
): { distance: number; state: PullState } {
  const [distance, setDistance] = useState(0);
  const [refreshing, setRefreshing] = useState(false);
  /** A refresh is running; further pulls wait for it. */
  const busy = useRef(false);
  const refreshRef = useRef(onRefresh);
  useEffect(() => {
    refreshRef.current = onRefresh;
  });
  const enabled = onRefresh !== undefined;

  useEffect(() => {
    const el = scrollRef.current;
    if (!enabled || !el) return;
    let startY: number | null = null;
    let pulled = 0;

    const onStart = (e: TouchEvent) => {
      const touch = e.touches[0];
      const ready = !busy.current && el.scrollTop <= 0 && e.touches.length === 1;
      startY = ready && touch ? touch.clientY : null;
      pulled = 0;
    };
    const onMove = (e: TouchEvent) => {
      const touch = e.touches[0];
      if (startY === null || !touch) return;
      const dy = touch.clientY - startY;
      if (dy <= 0 || el.scrollTop > 0) {
        // Scrolling up through the list as usual.
        if (pulled !== 0) {
          pulled = 0;
          setDistance(0);
        }
        return;
      }
      // Hold the page still while the indicator stretches.
      if (e.cancelable) e.preventDefault();
      pulled = Math.min(PULL_MAX, dy * DAMPING);
      setDistance(pulled);
    };
    const onEnd = () => {
      if (startY === null) return;
      startY = null;
      const refresh = refreshRef.current;
      if (pulled >= PULL_THRESHOLD && refresh) {
        busy.current = true;
        setRefreshing(true);
        setDistance(PULL_THRESHOLD);
        refresh()
          .catch(() => undefined)
          .finally(() => {
            busy.current = false;
            setRefreshing(false);
            setDistance(0);
          });
      } else {
        setDistance(0);
      }
      pulled = 0;
    };

    el.addEventListener('touchstart', onStart, { passive: true });
    // Not passive: it must be able to stop the page from scrolling.
    el.addEventListener('touchmove', onMove, { passive: false });
    el.addEventListener('touchend', onEnd);
    el.addEventListener('touchcancel', onEnd);
    return () => {
      el.removeEventListener('touchstart', onStart);
      el.removeEventListener('touchmove', onMove);
      el.removeEventListener('touchend', onEnd);
      el.removeEventListener('touchcancel', onEnd);
    };
  }, [enabled, scrollRef]);

  const state: PullState = refreshing ? 'refreshing' : distance >= PULL_THRESHOLD ? 'armed' : distance > 0 ? 'pulling' : 'idle';
  return { distance, state };
}
