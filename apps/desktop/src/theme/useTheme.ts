import { useEffect, useSyncExternalStore } from 'react';

import type { ThemeSetting } from '../api/types';

export type Theme = 'light' | 'dark';

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

export function resolveTheme(setting: ThemeSetting, systemDark: boolean): Theme {
  if (setting === 'system') return systemDark ? 'dark' : 'light';
  return setting;
}

/** Apply the theme from Settings, following the system when it says "system". */
export function useTheme(setting: ThemeSetting): Theme {
  const systemDark = useSyncExternalStore(subscribe, systemPrefersDark, () => false);
  const theme = resolveTheme(setting, systemDark);
  useEffect(() => {
    document.documentElement.dataset['theme'] = theme;
    matchSystemBars();
  }, [theme]);
  return theme;
}

/** Android: colour the phone's status and navigation bars. No-op elsewhere. */
export function setSystemBars(dark: boolean, background: string) {
  const bars = (window as { MockySystemBars?: { set: (dark: boolean, background: string) => void } }).MockySystemBars;
  bars?.set(dark, background);
}

/** Android: match the phone's bars to the app's current theme. */
export function matchSystemBars() {
  const root = document.documentElement;
  setSystemBars(root.dataset['theme'] === 'dark', getComputedStyle(root).getPropertyValue('--list').trim());
}
