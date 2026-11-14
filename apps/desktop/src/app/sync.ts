import type { Account, SyncStatus } from '../api/types';
import { formatClock } from '../lib/format';

export interface SyncSummary {
  /** Short pill text, or null when everything is fine. */
  pill: string | null;
  tone: 'ok' | 'info' | 'warn' | 'error';
  status: string;
  lastSync: string | null;
}

/** One line for the top bar and status bar from every account's sync state. */
export function syncSummary(
  statuses: readonly SyncStatus[],
  accounts: readonly Account[],
  timeZone?: string,
): SyncSummary {
  const known = statuses.filter((s) => accounts.some((a) => a.id === s.accountId));
  const latest = known.reduce<number | null>(
    (max, s) => (s.lastSyncedAt !== null && (max === null || s.lastSyncedAt > max) ? s.lastSyncedAt : max),
    null,
  );
  const lastSync = latest === null ? null : `Last synced ${formatClock(latest, timeZone)}`;
  const nameOf = (id: string) => accounts.find((a) => a.id === id)?.email ?? 'an account';

  const failing = known.find((s) => s.state === 'error');
  if (failing) {
    return {
      pill: 'Sync problem',
      tone: 'error',
      status: `Could not sync ${nameOf(failing.accountId)}${failing.message ? `: ${failing.message}` : '.'}`,
      lastSync,
    };
  }
  if (known.some((s) => s.state === 'syncing')) {
    return { pill: 'Syncing', tone: 'info', status: 'Checking for new mail.', lastSync };
  }
  if (known.length > 0 && known.length >= accounts.length && known.every((s) => s.state === 'offline')) {
    return {
      pill: 'Offline',
      tone: 'warn',
      status: 'Offline. Working from mail saved on this computer; new mail waits in the Outbox.',
      lastSync,
    };
  }
  if (known.some((s) => s.state === 'offline')) {
    const offline = known.filter((s) => s.state === 'offline').map((s) => nameOf(s.accountId));
    return { pill: 'Partly offline', tone: 'warn', status: `Cannot reach ${offline.join(', ')}.`, lastSync };
  }
  return { pill: null, tone: 'ok', status: 'Up to date.', lastSync };
}
