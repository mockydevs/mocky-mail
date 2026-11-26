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

type Insets = Partial<Record<'top' | 'right' | 'bottom' | 'left', string>>;

/**
 * Android: the app draws under the status and navigation bars, and the
 * WebView reports env(safe-area-inset-*) as 0, so MainActivity passes the
 * bars' sizes in. They become --safe-top etc. (tokens.css defaults them to
 * env(), which is right on other systems).
 */
function applyInsets(insets: Insets) {
  const style = document.documentElement.style;
  for (const side of ['top', 'right', 'bottom', 'left'] as const) {
    const value = insets[side];
    if (value && /^\d+(\.\d+)?px$/.test(value)) style.setProperty(`--safe-${side}`, value);
  }
}

if (typeof window !== 'undefined') {
  const host = window as { MockyInsets?: { get: () => string }; __mockyInsets?: (insets: Insets) => void };
  host.__mockyInsets = applyInsets;
  try {
    const now = host.MockyInsets?.get();
    if (now) applyInsets(JSON.parse(now) as Insets);
  } catch {
    // Not on Android, or no sizes yet: the env() defaults stay.
  }
}

/** Android: colour the phone's status and navigation bars. No-op elsewhere. */
export function setSystemBars(dark: boolean, background: string) {
  const bars = (window as { MockySystemBars?: { set: (dark: boolean, background: string) => void } }).MockySystemBars;
  bars?.set(dark, background);
}

/** Android: match the phone's bars to the app's current theme. */
export function matchSystemBars() {
  const root = document.documentElement;
  const style = getComputedStyle(root);
  // The welcome screen's blue or navy brand panel runs up under the status
  // bar, so the clock and icons go light there in both themes.
  if (document.querySelector('.welcome')) {
    setSystemBars(true, style.getPropertyValue('--pane-bg').trim());
    return;
  }
  setSystemBars(root.dataset['theme'] === 'dark', style.getPropertyValue('--list').trim());
}
