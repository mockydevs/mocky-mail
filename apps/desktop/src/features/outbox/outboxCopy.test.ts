import { outboxCopy } from './outboxCopy';

const MB = 1024 * 1024;

describe('outboxCopy', () => {
  it('describes a retry with a countdown', () => {
    const copy = outboxCopy({ kind: 'retrying', retryInSeconds: 12 }, 'Work');
    expect(copy.tone).toBe('warn');
    expect(copy.pill).toBe('Retrying in 12 s');
    expect(copy.text).toContain('12 seconds');
    expect(copy.canSendNow).toBe(true);
  });

  it('describes a message waiting with no countdown', () => {
    expect(outboxCopy({ kind: 'retrying', retryInSeconds: 0 }, 'Work').pill).toBe('Waiting to send');
  });

  it('clamps sending progress and blocks cancel mid-send', () => {
    const copy = outboxCopy({ kind: 'sending', progress: 140 }, 'Work');
    expect(copy.progress).toBe(100);
    expect(copy.canCancel).toBe(false);
    expect(outboxCopy({ kind: 'sending', progress: Number.NaN }, 'Work').progress).toBe(0);
  });

  it('explains a too-large message with real sizes', () => {
    const copy = outboxCopy(
      { kind: 'failed', reason: 'too_large', estimatedBytes: 28 * MB, limitBytes: 25 * MB },
      'Work',
    );
    expect(copy.tone).toBe('error');
    expect(copy.text).toContain('28 MB');
    expect(copy.text).toContain('25 MB');
    expect(copy.canSendNow).toBe(false);
  });

  it('passes server detail through for rejections', () => {
    const copy = outboxCopy({ kind: 'failed', reason: 'rejected', detail: '550 No such user' }, 'Work');
    expect(copy.pill).toBe('Rejected');
    expect(copy.text).toBe('550 No such user');
  });
});
