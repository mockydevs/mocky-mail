import { useCallback, useEffect, useState } from 'react';

import { errorTitle } from '../api/errors';
import type { AppError } from '../api/types';

/** How long a confirmation toast stays up. */
export const NOTICE_MS = 4000;

export interface Notice {
  tone: 'error' | 'info';
  title: string;
  text: string;
  /** Stays up until replaced or dismissed (an action still under way). */
  sticky?: boolean;
  /** One button beside the text, such as Undo. */
  action?: { label: string; run: () => void };
}

export function errorNotice(error: AppError, what: string): Notice {
  return { tone: 'error', title: `${what}: ${errorTitle(error.kind)}`, text: error.message };
}

export interface Toasts {
  notice: Notice | null;
  /** Show a toast in place of the current one (null hides it). */
  show: (notice: Notice | null) => void;
  /** Explain an error the backend returned for `what`. */
  report: (error: AppError, what: string) => void;
  dismiss: () => void;
}

/**
 * The one toast for confirmations and errors. Confirmations close by
 * themselves; errors and sticky notices stay until dismissed or replaced.
 */
export function useToasts(): Toasts {
  const [notice, setNotice] = useState<Notice | null>(null);

  useEffect(() => {
    if (notice?.tone !== 'info' || notice.sticky) return;
    const timer = window.setTimeout(() => {
      setNotice(null);
    }, NOTICE_MS);
    return () => {
      window.clearTimeout(timer);
    };
  }, [notice]);

  const report = useCallback((error: AppError, what: string) => {
    setNotice(errorNotice(error, what));
  }, []);
  const dismiss = useCallback(() => {
    setNotice(null);
  }, []);

  return { notice, show: setNotice, report, dismiss };
}
