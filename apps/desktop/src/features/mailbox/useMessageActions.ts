import { useEffect, useRef, useState } from 'react';

import { toAppError } from '../../api/errors';
import type { AppError, FolderRole, Importance, MailApi, MessageListItem, MoveTarget } from '../../api/types';
import { nextOverride, type Override, type OverridePatch } from './selectors';
import { useUndoHistory, type UndoEntry } from './useUndoHistory';

/** The messages a list row stands for: the whole conversation when grouped. */
export function idsOf(item: MessageListItem | undefined, fallback = ''): string[] {
  if (!item) return fallback ? [fallback] : [];
  return item.threadIds && item.threadIds.length > 0 ? item.threadIds : [item.id];
}

function messageIds(rows: readonly MessageListItem[]): string[] {
  return [...new Set(rows.flatMap((row) => idsOf(row)))];
}

/** Rows that share a value, in first-seen order. */
function groupBy<V>(rows: readonly MessageListItem[], value: (row: MessageListItem) => V): [V, MessageListItem[]][] {
  const groups = new Map<string, [V, MessageListItem[]]>();
  for (const row of rows) {
    const v = value(row);
    const key = JSON.stringify(v);
    const group = groups.get(key);
    if (group) group[1].push(row);
    else groups.set(key, [v, [row]]);
  }
  return [...groups.values()];
}

/** "Archived", or "Archived 3 messages". */
export function countLabel(verb: string, rows: readonly unknown[]): string {
  return rows.length === 1 ? verb : `${verb} ${String(rows.length)} messages`;
}

export interface FlagValue {
  flagged: boolean;
  /** Due date to set (null: none); missing leaves it as it is. */
  due?: number | null;
}

interface Deps {
  api: MailApi;
  /** The list's page version, which settled changes are measured against. */
  version: number;
  refresh: () => void;
  report: (error: AppError, what: string) => void;
  /** The role of a folder, to put junk back where it came from. */
  roleOfFolder: (folderId: string) => FolderRole | null;
}

/**
 * Changes to messages: shown at once through overrides on the list rows,
 * sent to the backend, and taken back (with an explanation) if it refuses.
 * Most of them also go into the undo history for Ctrl+Z and Ctrl+Y.
 */
export function useMessageActions({ api, version, refresh, report, roleOfFolder }: Deps) {
  const [overrides, setOverrides] = useState<Record<string, Override>>({});
  const versionRef = useRef(version);
  useEffect(() => {
    versionRef.current = version;
  });
  const history = useUndoHistory();

  /** Apply changes to rows at once; undo them and explain if the backend refuses. */
  const optimisticMany = (
    changes: readonly { rowIds: readonly string[]; patch: OverridePatch }[],
    call: () => Promise<void>,
    what: string,
    onRollback?: () => void,
  ) => {
    const v = versionRef.current;
    const rowIds = changes.flatMap((c) => c.rowIds);
    setOverrides((o) => {
      const next = { ...o };
      for (const change of changes) {
        for (const id of change.rowIds) next[id] = nextOverride(o[id], change.patch, v);
      }
      return next;
    });
    call().then(
      () => {
        setOverrides((o) => {
          const next = { ...o };
          for (const id of rowIds) {
            const entry = next[id];
            if (entry) next[id] = { ...entry, settled: true, version: versionRef.current };
          }
          return next;
        });
        refresh();
      },
      (e: unknown) => {
        setOverrides((o) => {
          const next = { ...o };
          for (const id of rowIds) Reflect.deleteProperty(next, id);
          return next;
        });
        onRollback?.();
        report(toAppError(e), what);
      },
    );
  };

  const optimistic = (
    rowIds: readonly string[],
    patch: OverridePatch,
    call: () => Promise<void>,
    what: string,
    onRollback?: () => void,
  ) => {
    optimisticMany([{ rowIds, patch }], call, what, onRollback);
  };

  /** Do `forward` now and keep it, with `backward`, for undo. */
  const record = (label: string, forward: () => void, backward: () => void): UndoEntry => {
    const entry: UndoEntry = { label, undo: backward, redo: forward };
    forward();
    history.push(entry);
    return entry;
  };

  /**
   * Set one value on every row; undo puts each row's own earlier value back.
   * `patch` is what the row shows meanwhile, `call` what the backend does.
   */
  const setValue = <V>(
    rows: readonly MessageListItem[],
    label: string,
    what: string,
    next: V,
    previous: (row: MessageListItem) => V,
    patch: (value: V) => OverridePatch,
    call: (ids: string[], value: V) => Promise<void>,
  ): UndoEntry => {
    const apply = (groups: [V, MessageListItem[]][]) => {
      for (const [value, group] of groups) {
        optimistic(
          group.map((r) => r.id),
          patch(value),
          () => call(messageIds(group), value),
          what,
        );
      }
    };
    const before = groupBy(rows, previous);
    return record(
      label,
      () => {
        apply([[next, [...rows]]]);
      },
      () => {
        apply(before);
      },
    );
  };

  const setRead = (rows: readonly MessageListItem[], read: boolean) =>
    setValue(
      rows,
      countLabel(read ? 'Marked as read' : 'Marked as unread', rows),
      'Could not change read state',
      read,
      (row) => !row.unread,
      (value) => ({ unread: !value }),
      (ids, value) => api.setFlags(ids, { seen: value }),
    );

  const setFlag = (rows: readonly MessageListItem[], flag: FlagValue) =>
    setValue<FlagValue>(
      rows,
      countLabel(flag.flagged ? 'Flagged' : 'Flag cleared', rows),
      'Could not flag',
      flag,
      (row) => ({ flagged: row.flagged, due: row.flagDue }),
      (value) => ({
        flagged: value.flagged,
        ...(value.due !== undefined ? { flagDue: value.due } : value.flagged ? {} : { flagDue: null }),
      }),
      (ids, value) =>
        api.setFlags(
          ids,
          value.flagged && value.due !== undefined ? { flagged: true, flagDue: value.due } : { flagged: value.flagged },
        ),
    );

  const setPinned = (rows: readonly MessageListItem[], pinned: boolean) =>
    setValue(
      rows,
      countLabel(pinned ? 'Pinned' : 'Unpinned', rows),
      pinned ? 'Could not pin' : 'Could not unpin',
      pinned,
      (row) => row.pinned,
      (value) => ({ pinned: value }),
      (ids, value) => api.setFlags(ids, { pinned: value }),
    );

  const setImportance = (rows: readonly MessageListItem[], importance: Importance) =>
    setValue(
      rows,
      countLabel(importance === 'normal' ? 'Importance cleared' : `Marked as ${importance} importance`, rows),
      'Could not change importance',
      importance,
      (row) => row.importance,
      (value) => ({ importance: value }),
      (ids, value) => api.setFlags(ids, { importance: value }),
    );

  /** Add a category to every row, or take it off every row. */
  const setCategory = (rows: readonly MessageListItem[], name: string, add: boolean) => {
    const has = (row: MessageListItem) => row.categories.some((c) => c.toLowerCase() === name.toLowerCase());
    const changing = rows.filter((row) => has(row) !== add);
    const apply = (adding: boolean) => {
      const changes = groupBy(changing, (row) => row.categories).map(([categories, group]) => ({
        rowIds: group.map((r) => r.id),
        patch: {
          categories: adding
            ? [...categories.filter((c) => c.toLowerCase() !== name.toLowerCase()), name]
            : categories.filter((c) => c.toLowerCase() !== name.toLowerCase()),
        },
      }));
      if (changes.length === 0) return;
      optimisticMany(
        changes,
        () => api.setCategories(messageIds(changing), adding ? [name] : [], adding ? [] : [name]),
        'Could not change categories',
      );
    };
    return record(
      `${add ? 'Added to' : 'Removed from'} ${name}`,
      () => {
        apply(add);
      },
      () => {
        apply(!add);
      },
    );
  };

  /**
   * Move rows away (archive, delete to Trash, a folder). Undo moves each back
   * to the folder it was in.
   */
  const move = (rows: readonly MessageListItem[], target: MoveTarget, label: string, what: string, onRollback?: () => void) =>
    record(
      label,
      () => {
        optimistic(
          rows.map((r) => r.id),
          { removed: true },
          () => api.moveMessages(messageIds(rows), target),
          what,
          onRollback,
        );
      },
      () => {
        for (const [folderId, group] of groupBy(rows, (row) => row.folderId)) {
          optimistic(
            group.map((r) => r.id),
            { removed: false },
            () => api.moveMessages(messageIds(group), { folderId }),
            'Could not undo',
          );
        }
      },
    );

  /** Junk or not junk. Undo marks them back and returns each to its folder. */
  const markJunk = (rows: readonly MessageListItem[], junk: boolean, onRollback?: () => void) =>
    record(
      countLabel(junk ? 'Moved to Junk' : 'Moved to Inbox', rows),
      () => {
        optimistic(
          rows.map((r) => r.id),
          { removed: true },
          () => api.markJunk(messageIds(rows), junk),
          junk ? 'Could not move to Junk' : 'Could not move out of Junk',
          onRollback,
        );
      },
      () => {
        for (const [folderId, group] of groupBy(rows, (row) => row.folderId)) {
          const ids = messageIds(group);
          // Marking them back puts them in the Inbox (or Junk); from there,
          // on to the folder they came from if that was somewhere else.
          const lands: FolderRole = junk ? 'inbox' : 'junk';
          optimistic(
            group.map((r) => r.id),
            { removed: false },
            () =>
              api.markJunk(ids, !junk).then(() =>
                roleOfFolder(folderId) === lands ? undefined : api.moveMessages(ids, { folderId }),
              ),
            'Could not undo',
          );
        }
      },
    );

  /** Copies stay where they are put; there is nothing to undo in the list. */
  const copy = (rows: readonly MessageListItem[], target: MoveTarget) => {
    api.copyMessages(messageIds(rows), target).then(refresh, (e: unknown) => {
      report(toAppError(e), 'Could not copy');
    });
  };

  /** Not undoable, which is why the user is asked first. */
  const deleteForever = (rows: readonly MessageListItem[], onRollback?: () => void) => {
    optimistic(
      rows.map((r) => r.id),
      { removed: true },
      () => api.deleteForever(messageIds(rows)),
      'Could not delete',
      onRollback,
    );
  };

  return {
    overrides,
    optimistic,
    setRead,
    setFlag,
    setPinned,
    setImportance,
    setCategory,
    move,
    markJunk,
    copy,
    deleteForever,
    history,
  };
}

export type MessageActions = ReturnType<typeof useMessageActions>;
