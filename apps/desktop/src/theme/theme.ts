export type Theme = 'light' | 'dark';

export const THEME_STORAGE_KEY = 'mocky-mail.theme';

type StorageLike = Pick<Storage, 'getItem' | 'setItem'>;

/** The saved override, or null. Never throws: storage can be disabled or full. */
export function readStoredTheme(storage: StorageLike | null): Theme | null {
  try {
    const value = storage?.getItem(THEME_STORAGE_KEY);
    return value === 'light' || value === 'dark' ? value : null;
  } catch {
    return null;
  }
}

/** Save the override. Returns false when storage refused it. */
export function writeStoredTheme(storage: StorageLike | null, theme: Theme): boolean {
  try {
    if (!storage) return false;
    storage.setItem(THEME_STORAGE_KEY, theme);
    return true;
  } catch {
    return false;
  }
}

/** A saved choice wins; otherwise follow the operating system. */
export function resolveTheme(stored: Theme | null, systemPrefersDark: boolean): Theme {
  return stored ?? (systemPrefersDark ? 'dark' : 'light');
}

export function safeLocalStorage(): Storage | null {
  try {
    return typeof window === 'undefined' ? null : window.localStorage;
  } catch {
    return null;
  }
}
