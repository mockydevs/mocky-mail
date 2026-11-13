import { connectionCopy } from './connection';

describe('connectionCopy', () => {
  it('describes each state', () => {
    expect(connectionCopy({ state: 'online', lastSyncAt: null }).tone).toBe('ok');
    expect(connectionCopy({ state: 'weak', lastSyncAt: null }).pill).toBe('Weak connection');
    expect(connectionCopy({ state: 'offline', lastSyncAt: null }).tone).toBe('error');
  });

  it('shows the last sync time when known', () => {
    expect(connectionCopy({ state: 'weak', lastSyncAt: '2026-11-13T07:02:00Z' }, 'Africa/Nairobi').lastSync).toBe(
      'Last full sync 10:02',
    );
    expect(connectionCopy({ state: 'weak', lastSyncAt: null }).lastSync).toBeNull();
  });
});
