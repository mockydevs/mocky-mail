import { useCallback, useEffect, useRef, useState } from 'react';

import { toAppError } from '../../api/errors';
import type { AppError, ListQuery, MailApi, MessageListItem } from '../../api/types';

export const PAGE_SIZE = 50;
const MAX_LIMIT = 200;

export type PageQuery = Omit<ListQuery, 'offset' | 'limit'>;

export interface MessagePageState {
  status: 'loading' | 'ready' | 'error';
  items: MessageListItem[];
  total: number;
  /** Increases each time fresh data arrives. */
  version: number;
  loadingMore: boolean;
  error: AppError | null;
  hasMore: boolean;
  loadMore: () => void;
}

interface Data {
  key: string;
  items: MessageListItem[];
  total: number;
  version: number;
  error: AppError | null;
}

/**
 * Pages of messages for `query`, newest first. A new query starts over and
 * drops late results from the old one. A new `revision` (after mail://changed
 * or an action) reloads as many messages as are already loaded, so the list
 * does not jump back to the top. `loadMore` fetches the next page once.
 */
export function useMessagePage(api: MailApi, query: PageQuery | null, revision: number): MessagePageState {
  const key = query ? JSON.stringify(query) : '';
  const [data, setData] = useState<Data | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const loadedRef = useRef(0);
  const requestRef = useRef(0);
  const queryRef = useRef(query);

  useEffect(() => {
    queryRef.current = query;
  });

  const loadedCount = data?.key === key ? data.items.length : 0;
  useEffect(() => {
    loadedRef.current = loadedCount;
  });

  useEffect(() => {
    const current = queryRef.current;
    if (!current) return;
    const request = ++requestRef.current;
    const limit = Math.min(MAX_LIMIT, Math.max(PAGE_SIZE, loadedRef.current));
    api.listMessages({ ...current, offset: 0, limit }).then(
      (page) => {
        if (request !== requestRef.current) return;
        setData((prev) => ({
          key,
          items: page.items,
          total: page.total,
          version: (prev?.version ?? 0) + 1,
          error: null,
        }));
      },
      (error: unknown) => {
        if (request !== requestRef.current) return;
        setData((prev) => ({
          key,
          items: prev?.key === key ? prev.items : [],
          total: prev?.key === key ? prev.total : 0,
          version: (prev?.version ?? 0) + 1,
          error: toAppError(error),
        }));
      },
    );
  }, [api, key, revision]);

  const current = data?.key === key ? data : null;
  const hasMore = current !== null && current.items.length < current.total;

  const loadMore = useCallback(() => {
    const q = queryRef.current;
    if (!q || !current || loadingMore || current.items.length >= current.total) return;
    const request = requestRef.current;
    setLoadingMore(true);
    api
      .listMessages({ ...q, offset: current.items.length, limit: PAGE_SIZE })
      .then(
        (page) => {
          if (request !== requestRef.current) return;
          setData((prev) => {
            if (prev?.key !== key) return prev;
            const known = new Set(prev.items.map((m) => m.id));
            return {
              ...prev,
              items: [...prev.items, ...page.items.filter((m) => !known.has(m.id))],
              total: page.total,
              version: prev.version + 1,
            };
          });
        },
        (error: unknown) => {
          if (request !== requestRef.current) return;
          setData((prev) => (prev?.key === key ? { ...prev, error: toAppError(error) } : prev));
        },
      )
      .finally(() => {
        setLoadingMore(false);
      });
  }, [api, current, key, loadingMore]);

  return {
    status: current === null ? 'loading' : current.error && current.items.length === 0 ? 'error' : 'ready',
    items: current?.items ?? [],
    total: current?.total ?? 0,
    version: data?.version ?? 0,
    loadingMore,
    error: current?.error ?? null,
    hasMore,
    loadMore,
  };
}
