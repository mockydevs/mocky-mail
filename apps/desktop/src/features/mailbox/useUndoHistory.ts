import { useCallback, useRef } from 'react';

/** One change the user made that can be taken back and made again. */
export interface UndoEntry {
  /** What was done, for the toast: "Archived", "Flagged 3 messages". */
  label: string;
  undo: () => void;
  redo: () => void;
}

/** How many changes Ctrl+Z can go back through. */
export const UNDO_LIMIT = 20;

/**
 * The last few changes (read, flag, pin, categories, importance, moves,
 * junk), newest last. Doing something new forgets what was undone.
 */
export function useUndoHistory(limit = UNDO_LIMIT) {
  const done = useRef<UndoEntry[]>([]);
  const undone = useRef<UndoEntry[]>([]);

  const push = useCallback(
    (entry: UndoEntry) => {
      done.current = [...done.current, entry].slice(-limit);
      undone.current = [];
    },
    [limit],
  );

  /** Undo the newest change; returns it, or null when there is nothing to undo. */
  const undo = useCallback((): UndoEntry | null => {
    const entry = done.current[done.current.length - 1];
    if (!entry) return null;
    done.current = done.current.slice(0, -1);
    undone.current = [...undone.current, entry];
    entry.undo();
    return entry;
  }, []);

  const redo = useCallback((): UndoEntry | null => {
    const entry = undone.current[undone.current.length - 1];
    if (!entry) return null;
    undone.current = undone.current.slice(0, -1);
    done.current = [...done.current, entry];
    entry.redo();
    return entry;
  }, []);

  /** Undo one particular change (a toast's Undo), wherever it is in the history. */
  const undoEntry = useCallback((entry: UndoEntry): void => {
    if (!done.current.includes(entry)) return;
    done.current = done.current.filter((e) => e !== entry);
    undone.current = [...undone.current, entry];
    entry.undo();
  }, []);

  return { push, undo, redo, undoEntry };
}

export type UndoHistory = ReturnType<typeof useUndoHistory>;
