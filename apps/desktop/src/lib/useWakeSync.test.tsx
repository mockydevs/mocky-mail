import { renderHook } from '@testing-library/react';
import { vi } from 'vitest';

import { useWakeSync, WAKE_SYNC_GAP_MS } from './useWakeSync';

describe('useWakeSync', () => {
  it('checks for mail when the app comes back, at most once per gap', () => {
    let clock = 1_000_000;
    const syncNow = vi.fn();
    const now = () => clock;
    renderHook(() => {
      useWakeSync(syncNow, now);
    });

    // Focus right after opening: nothing yet.
    window.dispatchEvent(new Event('focus'));
    expect(syncNow).not.toHaveBeenCalled();

    // Back after a while (laptop woke, phone resumed): check at once.
    clock += WAKE_SYNC_GAP_MS + 1;
    window.dispatchEvent(new Event('focus'));
    expect(syncNow).toHaveBeenCalledTimes(1);

    // Focus and network events together don't pile up.
    window.dispatchEvent(new Event('online'));
    document.dispatchEvent(new Event('visibilitychange'));
    expect(syncNow).toHaveBeenCalledTimes(1);

    // The network coming back later also checks.
    clock += WAKE_SYNC_GAP_MS + 1;
    window.dispatchEvent(new Event('online'));
    expect(syncNow).toHaveBeenCalledTimes(2);
  });
});
