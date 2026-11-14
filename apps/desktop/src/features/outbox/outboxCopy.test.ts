import type { OutboxEntry, OutboxState } from '../../api/types';
import { isActive, outboxCopy } from './outboxCopy';

const NOW = 1_794_552_900;

function entry(status: OutboxState, extra: Partial<OutboxEntry> = {}): OutboxEntry {
  return {
    id: 'o', accountId: 'work', to: [], subject: 's', preview: 'p', createdAt: NOW,
    attempts: 0, status, progress: null, ...extra,
  };
}

describe('outboxCopy', () => {
  it('waits to send a fresh message', () => {
    const copy = outboxCopy(entry({ state: 'queued', next_attempt_at: NOW, maybe_sent: false }), 'Work', NOW);
    expect(copy.pill).toBe('Waiting to send');
    expect(copy.canSendNow && copy.canEdit && copy.canCancel).toBe(true);
  });

  it('counts down a retry, then shows a clock time for long waits', () => {
    const soon = entry({ state: 'queued', next_attempt_at: NOW + 12, maybe_sent: false }, { attempts: 2 });
    expect(outboxCopy(soon, 'Work', NOW).pill).toBe('Retrying in 12 s');
    const later = entry({ state: 'queued', next_attempt_at: NOW + 600, maybe_sent: false }, { attempts: 5 });
    expect(outboxCopy(later, 'Work', NOW, 'UTC').pill).toMatch(/^Retrying at \d\d:\d\d$/);
  });

  it('never offers Send now while checking for a duplicate', () => {
    const copy = outboxCopy(entry({ state: 'queued', next_attempt_at: NOW, maybe_sent: true }), 'Work', NOW);
    expect(copy.pill).toBe('Checking');
    expect(copy.canSendNow).toBe(false);
  });

  it('shows sending progress from a 0..1 fraction and allows no changes', () => {
    const copy = outboxCopy(entry({ state: 'sending', started_at: NOW }, { progress: 0.64 }), 'Work', NOW);
    expect(copy.pill).toBe('Sending 64%');
    expect(copy.progress).toBe(64);
    expect(copy.canCancel || copy.canEdit || copy.canSendNow).toBe(false);
    expect(outboxCopy(entry({ state: 'sending', started_at: NOW }, { progress: 7 }), 'Work', NOW).progress).toBe(100);
    expect(outboxCopy(entry({ state: 'sending', started_at: NOW }), 'Work', NOW).pill).toBe('Sending');
  });

  it('explains each failure and only retries when it can help', () => {
    const big = outboxCopy(entry({ state: 'failed', reason: 'too_large', message: 'about 28 MB' }), 'Work', NOW);
    expect(big.canSendNow).toBe(false);
    expect(big.detail).toBe('about 28 MB');
    const rejected = outboxCopy(entry({ state: 'failed', reason: 'rejected', message: '550 no such user' }), 'Work', NOW);
    expect(rejected.pill).toBe('Rejected');
    expect(rejected.canSendNow).toBe(true);
    expect(outboxCopy(entry({ state: 'failed', reason: 'gave_up', message: 'timeout' }), 'Work', NOW).tone).toBe('error');
  });

  it('treats sent and cancelled as finished', () => {
    expect(isActive(entry({ state: 'sent', at: NOW }))).toBe(false);
    expect(isActive(entry({ state: 'cancelled' }))).toBe(false);
    expect(isActive(entry({ state: 'sending', started_at: NOW }))).toBe(true);
    expect(outboxCopy(entry({ state: 'sent', at: NOW }), 'Work', NOW).pill).toBe('Sent');
  });
});
