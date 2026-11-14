import { MOCK_ACCOUNTS } from '../api/mock/mockData';
import type { SyncStatus } from '../api/types';
import { syncSummary } from './sync';

const s = (accountId: string, state: SyncStatus['state'], lastSyncedAt: number | null = null, message: string | null = null): SyncStatus => ({
  accountId, state, lastSyncedAt, message,
});

describe('syncSummary', () => {
  it('is quiet when all is well and reports the latest sync', () => {
    const out = syncSummary([s('work', 'idle', 100), s('personal', 'idle', 7_200)], MOCK_ACCOUNTS, 'UTC');
    expect(out.pill).toBeNull();
    expect(out.lastSync).toBe('Last synced 02:00');
  });

  it('puts errors first, then syncing, then offline', () => {
    expect(syncSummary([s('work', 'syncing'), s('personal', 'error', null, 'login failed')], MOCK_ACCOUNTS).status).toBe(
      'Could not sync you@personal.example: login failed',
    );
    expect(syncSummary([s('work', 'syncing'), s('personal', 'offline')], MOCK_ACCOUNTS).pill).toBe('Syncing');
    expect(syncSummary([s('work', 'offline'), s('personal', 'offline')], MOCK_ACCOUNTS).pill).toBe('Offline');
    expect(syncSummary([s('work', 'offline'), s('personal', 'idle')], MOCK_ACCOUNTS).status).toBe('Cannot reach you@work.example.');
  });

  it('ignores status for removed accounts', () => {
    expect(syncSummary([s('gone', 'error')], MOCK_ACCOUNTS).pill).toBeNull();
  });
});
