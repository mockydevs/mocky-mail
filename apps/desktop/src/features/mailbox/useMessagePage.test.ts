import { act, renderHook, waitFor } from '@testing-library/react';
import { vi } from 'vitest';

import type { MailApi, MessageListItem, MessagePage } from '../../api/types';
import { PAGE_SIZE, useMessagePage, type PageQuery } from './useMessagePage';

const QUERY: PageQuery = { accountId: null, folderId: null, role: 'inbox', filter: 'all', search: null };

/** A folder of `count` messages served the way the backend does: 200 rows a call at most. */
function folder(count: number) {
  const all = Array.from({ length: count }, (_, i) => ({ id: `m${String(i)}` }) as MessageListItem);
  const listMessages = vi.fn(
    ({ offset, limit }: { offset: number; limit: number }): Promise<MessagePage> =>
      Promise.resolve({ items: all.slice(offset, offset + Math.min(limit, 200)), total: count }),
  );
  return { api: { listMessages } as unknown as MailApi, listMessages };
}

describe('useMessagePage', () => {
  it('keeps every loaded message when a long list is refreshed', async () => {
    const { api, listMessages } = folder(450);
    const { result, rerender } = renderHook(({ revision }) => useMessagePage(api, QUERY, revision), {
      initialProps: { revision: 0 },
    });
    await waitFor(() => {
      expect(result.current.items).toHaveLength(PAGE_SIZE);
    });
    // Scroll down past what one backend call can return.
    for (let loaded = PAGE_SIZE; loaded < 250; loaded += PAGE_SIZE) {
      act(() => {
        result.current.loadMore();
      });
      await waitFor(() => {
        expect(result.current.items).toHaveLength(loaded + PAGE_SIZE);
      });
    }
    listMessages.mockClear();

    // New mail or an action reloads the list: the 230th message must stay.
    rerender({ revision: 1 });
    await waitFor(() => {
      expect(listMessages).toHaveBeenCalledTimes(2);
    });
    expect(listMessages).toHaveBeenNthCalledWith(1, expect.objectContaining({ offset: 0, limit: 200 }));
    expect(listMessages).toHaveBeenNthCalledWith(2, expect.objectContaining({ offset: 200, limit: 50 }));
    await waitFor(() => {
      expect(result.current.items).toHaveLength(250);
    });
    expect(result.current.items.some((m) => m.id === 'm229')).toBe(true);
    expect(result.current.hasMore).toBe(true);
  });

  it('stops at the end of a short folder', async () => {
    const { api, listMessages } = folder(7);
    const { result } = renderHook(() => useMessagePage(api, QUERY, 0));
    await waitFor(() => {
      expect(result.current.items).toHaveLength(7);
    });
    expect(listMessages).toHaveBeenCalledTimes(1);
    expect(result.current.hasMore).toBe(false);
  });
});
