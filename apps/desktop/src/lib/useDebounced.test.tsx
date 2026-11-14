import { act, renderHook } from '@testing-library/react';
import { vi } from 'vitest';

import { useDebounced } from './useDebounced';

describe('useDebounced', () => {
  it('only settles after the value stops changing', () => {
    vi.useFakeTimers();
    try {
      const { result, rerender } = renderHook(({ v }) => useDebounced(v, 300), { initialProps: { v: 'a' } });
      rerender({ v: 'ab' });
      act(() => {
        vi.advanceTimersByTime(200);
      });
      rerender({ v: 'abc' });
      act(() => {
        vi.advanceTimersByTime(200);
      });
      expect(result.current).toBe('a');
      act(() => {
        vi.advanceTimersByTime(100);
      });
      expect(result.current).toBe('abc');
    } finally {
      vi.useRealTimers();
    }
  });
});
