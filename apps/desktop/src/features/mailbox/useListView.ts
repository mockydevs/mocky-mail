import { useState } from 'react';

import type { Focus, ListSort, ReadFilter } from '../../api/types';
import { useDebounced } from '../../lib/useDebounced';
import { safeLocalStorage } from './folderTree';
import { INBOX_VIEW, type MailView } from './selectors';

export const SEARCH_DEBOUNCE_MS = 300;

/** How rows are put under headings: by day (as before), by sender, or by importance. */
export type GroupBy = 'date' | 'from' | 'importance';

export interface ListOrder {
  sort: ListSort;
  /** Newest, Z–A or largest first. */
  descending: boolean;
  groupBy: GroupBy;
}

export const DEFAULT_ORDER: ListOrder = { sort: 'date', descending: true, groupBy: 'date' };

export const ORDER_STORAGE_KEY = 'mocky-mail.list.order';

const SORTS: readonly ListSort[] = ['date', 'from', 'subject', 'size'];
const GROUPS: readonly GroupBy[] = ['date', 'from', 'importance'];

/** The remembered sort and grouping. Never throws: storage can be blocked or corrupt. */
export function readOrder(storage: Pick<Storage, 'getItem'> | null): ListOrder {
  try {
    const raw = storage?.getItem(ORDER_STORAGE_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : null;
    if (typeof parsed !== 'object' || parsed === null) return DEFAULT_ORDER;
    const value = parsed as Partial<Record<keyof ListOrder, unknown>>;
    return {
      sort: SORTS.find((s) => s === value.sort) ?? DEFAULT_ORDER.sort,
      descending: typeof value.descending === 'boolean' ? value.descending : DEFAULT_ORDER.descending,
      groupBy: GROUPS.find((g) => g === value.groupBy) ?? DEFAULT_ORDER.groupBy,
    };
  } catch {
    return DEFAULT_ORDER;
  }
}

function writeOrder(order: ListOrder): void {
  try {
    safeLocalStorage()?.setItem(ORDER_STORAGE_KEY, JSON.stringify(order));
  } catch {
    // Remembering the order is a convenience; the list works without it.
  }
}

/**
 * What the message list shows: the folder or unified view, the read filter,
 * the search being typed (and its debounced value), the Focused Inbox half,
 * and the order rows come in, which is remembered between sessions.
 */
export function useListView() {
  const [view, setView] = useState<MailView>(INBOX_VIEW);
  const [filter, setFilter] = useState<ReadFilter>('all');
  const [searchInput, setSearchInput] = useState('');
  const search = useDebounced(searchInput, SEARCH_DEBOUNCE_MS);
  const [focusTab, setFocusTab] = useState<Focus>('focused');
  const [order, setOrderState] = useState<ListOrder>(() => readOrder(safeLocalStorage()));

  const setOrder = (next: ListOrder) => {
    setOrderState(next);
    writeOrder(next);
  };

  /** Open another view: back to its first half and every message. */
  const goTo = (next: MailView) => {
    setView(next);
    setFocusTab('focused');
    setFilter('all');
  };

  return {
    view,
    setView,
    filter,
    setFilter,
    searchInput,
    setSearchInput,
    search,
    focusTab,
    setFocusTab,
    order,
    setOrder,
    goTo,
  };
}

export type ListView = ReturnType<typeof useListView>;
