import { readStoredTheme, resolveTheme, THEME_STORAGE_KEY, writeStoredTheme } from './theme';

function memoryStorage(initial: Record<string, string> = {}) {
  const data = new Map(Object.entries(initial));
  return {
    getItem: (k: string) => data.get(k) ?? null,
    setItem: (k: string, v: string) => {
      data.set(k, v);
    },
  };
}

const throwing = {
  getItem: () => {
    throw new Error('SecurityError');
  },
  setItem: () => {
    throw new Error('QuotaExceededError');
  },
};

describe('theme storage', () => {
  it('reads only valid values', () => {
    expect(readStoredTheme(memoryStorage({ [THEME_STORAGE_KEY]: 'dark' }))).toBe('dark');
    expect(readStoredTheme(memoryStorage({ [THEME_STORAGE_KEY]: 'neon' }))).toBeNull();
    expect(readStoredTheme(null)).toBeNull();
  });

  it('never throws when storage is blocked', () => {
    expect(readStoredTheme(throwing)).toBeNull();
    expect(writeStoredTheme(throwing, 'dark')).toBe(false);
    expect(writeStoredTheme(null, 'dark')).toBe(false);
  });

  it('round-trips a saved choice', () => {
    const storage = memoryStorage();
    expect(writeStoredTheme(storage, 'light')).toBe(true);
    expect(readStoredTheme(storage)).toBe('light');
  });
});

describe('resolveTheme', () => {
  it('prefers the saved choice over the system', () => {
    expect(resolveTheme('light', true)).toBe('light');
    expect(resolveTheme(null, true)).toBe('dark');
    expect(resolveTheme(null, false)).toBe('light');
  });
});
