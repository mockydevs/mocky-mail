import { useState } from 'react';

import type { MailApi, OutboxEntry } from '../../api/types';
import { useMailEvent } from '../../lib/useEvent';

/** What the user is told about a message they just sent. */
export type SendStatus =
  | { phase: 'sending' }
  | { phase: 'sent' }
  /** Not out yet (offline, or the server asked to wait); it retries by itself. */
  | { phase: 'waiting' }
  | { phase: 'failed'; message: string };

/** Turn an Outbox entry into the outcome the user cares about. */
export function sendStatusOf(entry: OutboxEntry): SendStatus {
  const { status } = entry;
  switch (status.state) {
    case 'sent':
      return { phase: 'sent' };
    case 'failed':
      return { phase: 'failed', message: status.message };
    case 'cancelled':
      return { phase: 'failed', message: 'Sending was cancelled.' };
    case 'sending':
      return { phase: 'sending' };
    case 'queued':
      // A first attempt is on its way; after a failed attempt it waits to retry.
      return entry.attempts > 0 || status.maybe_sent ? { phase: 'waiting' } : { phase: 'sending' };
  }
}

/**
 * Follow one message from Send until it is actually out (or not), using the
 * Outbox's live updates. `track` starts following the entry `sendMessage`
 * returned; the status then moves on by itself.
 */
export function useSendStatus(api: MailApi): {
  status: SendStatus | null;
  track: (entry: OutboxEntry) => void;
  clear: () => void;
} {
  const [tracked, setTracked] = useState<{ id: string; lastState: OutboxEntry['status']['state'] } | null>(null);
  const [status, setStatus] = useState<SendStatus | null>(null);

  useMailEvent(api, 'mail://outbox-changed', ({ entries }) => {
    if (tracked === null) return;
    const entry = entries.find((e) => e.id === tracked.id);
    if (!entry) {
      // Finished entries leave the Outbox. Gone while sending means it went
      // out; gone while waiting means the user cancelled or edited it.
      const wentOut = tracked.lastState === 'sending' || tracked.lastState === 'sent';
      setStatus(wentOut ? { phase: 'sent' } : null);
      setTracked(null);
      return;
    }
    const next = sendStatusOf(entry);
    setStatus(next);
    setTracked(next.phase === 'sent' || next.phase === 'failed' ? null : { id: entry.id, lastState: entry.status.state });
  });

  return {
    status,
    track: (entry) => {
      const first = sendStatusOf(entry);
      setStatus(first);
      setTracked(first.phase === 'sent' || first.phase === 'failed' ? null : { id: entry.id, lastState: entry.status.state });
    },
    clear: () => {
      setTracked(null);
      setStatus(null);
    },
  };
}
