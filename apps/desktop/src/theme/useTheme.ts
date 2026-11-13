import { useCallback, useEffect, useState, useSyncExternalStore } from 'react';

import { readStoredTheme, resolveTheme, safeLocalStorage, writeStoredTheme, type Theme } from './theme';

const QUERY = '(prefers-color-scheme: dark)';

function media(): MediaQueryList | null {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function' ? window.matchMedia(QUERY) : null;
}

function subscribe(onChange: () => void): () => void {
  const list = media();
  list?.addEventListener('change', onChange);
  return () => list?.removeEventListener('change', onChange);
}

const systemPrefersDark = (): boolean => media()?.matches ?? false;

/** Follow the operating system until the user picks a theme, then remember it. */
export function useTheme(): { theme: Theme; toggle: () => void } {
  const [stored, setStored] = useState<Theme | null>(() => readStoredTheme(safeLocalStorage()));
  const systemDark = useSyncExternalStore(subscribe, systemPrefersDark, () => false);
  const theme = resolveTheme(stored, systemDark);

  useEffect(() => {
    document.documentElement.dataset['theme'] = theme;
  }, [theme]);

  const toggle = useCallback(() => {
    const next: Theme = theme === 'dark' ? 'light' : 'dark';
    writeStoredTheme(safeLocalStorage(), next);
    setStored(next);
  }, [theme]);

  return { theme, toggle };
}
