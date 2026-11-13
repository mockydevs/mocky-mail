import type { ConnectionStatus } from '../api/types';
import { formatClock } from '../lib/format';

export interface ConnectionCopy {
  pill: string;
  status: string;
  lastSync: string | null;
  tone: 'ok' | 'warn' | 'error';
}

export function connectionCopy(connection: ConnectionStatus, timeZone?: string): ConnectionCopy {
  const lastSync = connection.lastSyncAt ? `Last full sync ${formatClock(connection.lastSyncAt, timeZone)}` : null;
  switch (connection.state) {
    case 'online':
      return { pill: 'Online', status: 'Connected. Mail is up to date.', lastSync, tone: 'ok' };
    case 'weak':
      return {
        pill: 'Weak connection',
        status: 'Weak connection. Working from mail saved on this computer.',
        lastSync,
        tone: 'warn',
      };
    case 'offline':
      return {
        pill: 'Offline',
        status: 'Offline. New mail waits in the Outbox until you are back online.',
        lastSync,
        tone: 'error',
      };
  }
}
