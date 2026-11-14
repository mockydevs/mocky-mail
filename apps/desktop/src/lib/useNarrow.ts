import { useEffect, useState } from 'react';

/** Phone layout: one column. Keep in step with the `max-width: 760px` block in app.css. */
export const NARROW_QUERY = '(max-width: 760px)';

function query(): MediaQueryList | null {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function' ? window.matchMedia(NARROW_QUERY) : null;
}

/** True while the window is phone-sized. */
export function useNarrow(): boolean {
  const [narrow, setNarrow] = useState(() => query()?.matches ?? false);
  useEffect(() => {
    const list = query();
    if (!list) return;
    const update = () => {
      setNarrow(list.matches);
    };
    update();
    list.addEventListener('change', update);
    return () => {
      list.removeEventListener('change', update);
    };
  }, []);
  return narrow;
}
