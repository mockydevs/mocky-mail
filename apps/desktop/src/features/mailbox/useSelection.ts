import { useState } from 'react';

import type { MessageListItem } from '../../api/types';

const NONE: ReadonlySet<string> = new Set();

/** Row ids from `from` to `to` inclusive, in list order; just `to` when `from` is not listed. */
export function rangeOf(rows: readonly MessageListItem[], from: string | null, to: string): string[] {
  const end = rows.findIndex((m) => m.id === to);
  if (end < 0) return [];
  const start = from === null ? -1 : rows.findIndex((m) => m.id === from);
  if (start < 0) return [to];
  const [lo, hi] = start <= end ? [start, end] : [end, start];
  return rows.slice(lo, hi + 1).map((m) => m.id);
}

/**
 * Which message is open, and which rows are ticked for a bulk action.
 *
 * `selectedId` is the message the user opened (null: nothing chosen yet).
 * `kept` is that row as it was when opened, so a filter or search does not
 * drop it from the list the moment reading it changes it.
 * `checked` holds the row ids picked with Ctrl/Cmd-click, Shift-click, the
 * row checkboxes or (phones) a long press; while any are picked, the list
 * shows the selection bar.
 */
export function useSelection() {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [kept, setKept] = useState<MessageListItem | null>(null);
  const [checked, setChecked] = useState<ReadonlySet<string>>(NONE);
  /** Where a Shift-click range starts: the last row picked or opened. */
  const [anchor, setAnchor] = useState<string | null>(null);

  /** Open a message (or none). */
  const choose = (id: string | null, item: MessageListItem | null = null) => {
    setSelectedId(id);
    setKept(item);
    if (id !== null) setAnchor(id);
  };

  /** Close the open message: nothing chosen, nothing kept. */
  const close = () => {
    setSelectedId(null);
    setKept(null);
  };

  /** Tick or untick one row. `also` joins it first (the open row, on a first Ctrl-click). */
  const toggle = (id: string, also: string | null = null) => {
    setChecked((current) => {
      const next = new Set(current);
      if (current.size === 0 && also !== null && also !== id) next.add(also);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
    setAnchor(id);
  };

  /** Tick every row from the anchor (or `fallback`) to `id`, keeping what is already ticked. */
  const extendTo = (rows: readonly MessageListItem[], id: string, fallback: string | null) => {
    const ids = rangeOf(rows, anchor ?? fallback, id);
    setChecked((current) => new Set([...current, ...ids]));
  };

  const selectAll = (rows: readonly MessageListItem[]) => {
    setChecked(new Set(rows.map((m) => m.id)));
  };

  const clearChecked = () => {
    setChecked(NONE);
  };

  return {
    selectedId,
    setSelectedId,
    kept,
    setKept,
    choose,
    close,
    checked,
    toggle,
    extendTo,
    selectAll,
    clearChecked,
  };
}

export type Selection = ReturnType<typeof useSelection>;
