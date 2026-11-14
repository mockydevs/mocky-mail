import { useCallback, useState } from 'react';

import { readCollapsed, safeLocalStorage, writeCollapsed } from './folderTree';

/** Which folder-tree nodes are collapsed, remembered on this computer. */
export function useCollapsedFolders(): { isCollapsed: (key: string) => boolean; toggle: (key: string) => void } {
  const [collapsed, setCollapsed] = useState<Set<string>>(() => readCollapsed(safeLocalStorage()));
  const toggle = useCallback((key: string) => {
    setCollapsed((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      writeCollapsed(safeLocalStorage(), next);
      return next;
    });
  }, []);
  return { isCollapsed: (key) => collapsed.has(key), toggle };
}
