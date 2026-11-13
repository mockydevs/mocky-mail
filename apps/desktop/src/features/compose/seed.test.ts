import { MOCK_MESSAGES } from '../../api/mockData';
import type { MessageDetail } from '../../api/types';
import { editSeed, forwardSeed, replySeed, replySubject, toolbarActions } from './seed';

function byId(id: string): MessageDetail {
  const m = MOCK_MESSAGES.find((x) => x.id === id);
  if (!m) throw new Error(`missing ${id}`);
  return m;
}

describe('replySubject', () => {
  it('adds Re: once', () => {
    expect(replySubject('Lunch on Friday?')).toBe('Re: Lunch on Friday?');
    expect(replySubject('RE: Lunch')).toBe('RE: Lunch');
  });
});

describe('seeds', () => {
  it('replies to the sender from the same account', () => {
    expect(replySeed(byId('m3'))).toEqual({
      accountId: 'personal',
      to: 'brian@personal.example',
      subject: 'Re: Lunch on Friday?',
      body: '',
    });
  });

  it('forwards with the original text and header', () => {
    const seed = forwardSeed(byId('m6'), 'Africa/Nairobi');
    expect(seed.to).toBe('');
    expect(seed.subject).toBe('Fwd: Invoice INV-0142 paid');
    expect(seed.body).toContain('From: Peter Mwangi <peter@client.example>');
    expect(seed.body).toContain('Date: Wed 11 Nov, 15:47');
    expect(seed.body).toContain('Payment for INV-0142 has gone through.');
  });

  it('does not double the Fwd prefix', () => {
    expect(forwardSeed({ ...byId('m6'), subject: 'Fw: x' }).subject).toBe('Fw: x');
  });

  it('marks edits of queued mail as replacing the original', () => {
    expect(editSeed(byId('o3')).replacesOutboxId).toBe('o3');
    expect(editSeed(byId('d1'))).not.toHaveProperty('replacesOutboxId');
  });
});

describe('toolbarActions', () => {
  it('fits the folder and direction', () => {
    expect(toolbarActions({ folder: 'inbox', direction: 'incoming' })).toEqual([
      'reply',
      'forward',
      'archive',
      'delete',
      'flag',
    ]);
    expect(toolbarActions({ folder: 'sent', direction: 'outgoing' })).toEqual(['forward', 'archive', 'delete', 'flag']);
    expect(toolbarActions({ folder: 'trash', direction: 'incoming' })).toEqual(['reply', 'forward', 'flag']);
    expect(toolbarActions({ folder: 'drafts', direction: 'outgoing' })).toEqual(['edit', 'delete']);
    expect(toolbarActions({ folder: 'outbox', direction: 'outgoing' })).toEqual([]);
  });
});
