import { MOCK_ACCOUNTS, MOCK_MESSAGES } from '../../api/mock/mockData';
import type { Account, MessageDetail } from '../../api/types';
import {
  bodyTextOf,
  draftFromMessage,
  formatMailbox,
  forwardDraft,
  htmlToText,
  quote,
  replyDraft,
  replySubject,
  toolbarActions,
} from './seed';

function byId(id: string): MessageDetail {
  const m = MOCK_MESSAGES.find((x) => x.id === id);
  if (!m) throw new Error(`missing ${id}`);
  return m;
}
const [work, personal] = MOCK_ACCOUNTS as [Account, Account];

describe('addresses and subjects', () => {
  it('formats mailboxes safely', () => {
    expect(formatMailbox({ name: 'Wanjiru Kamau', address: 'w@x.example' })).toBe('Wanjiru Kamau <w@x.example>');
    expect(formatMailbox({ name: null, address: 'w@x.example' })).toBe('w@x.example');
    expect(formatMailbox({ name: 'Evil "<a@b>"', address: 'w@x.example' })).toBe('Evil a@b <w@x.example>');
  });

  it('adds Re: once', () => {
    expect(replySubject('Lunch')).toBe('Re: Lunch');
    expect(replySubject('RE: Lunch')).toBe('RE: Lunch');
  });
});

describe('replyDraft', () => {
  it('replies to the sender with threading and a quote', () => {
    const draft = replyDraft(byId('m1'), work, false, 'Africa/Nairobi');
    expect(draft.to).toBe('Wanjiru Kamau <wanjiru@fitout.example>');
    expect(draft.cc).toBe('');
    expect(draft.subject).toBe('Re: Revised quotation for the Westlands office fit-out');
    expect(draft.inReplyTo).toBe('<m1@mock.example>');
    expect(draft.references).toEqual(['<q-2040@fitout.example>', '<m1@mock.example>']);
    expect(draft.replyToMessageId).toBe('m1');
    expect(draft.bodyText).toContain('On Fri 13 Nov 2026, 09:42, Wanjiru Kamau wrote:');
    expect(draft.bodyText).toContain('> Thanks,\n> Wanjiru');
  });

  it('reply all keeps everyone except me', () => {
    const draft = replyDraft(byId('m1'), work, true);
    expect(draft.to).toBe('Wanjiru Kamau <wanjiru@fitout.example>');
    expect(draft.cc).toBe('Amina Hassan <amina@studio.example>');
    expect(draft.to + draft.cc).not.toContain('you@work.example');
  });

  it('honours Reply-To', () => {
    expect(replyDraft(byId('m3'), personal, false).to).toBe('Brian (phone) <brian.m@personal.example>');
  });

  it('replying to my own sent mail goes to its recipients', () => {
    expect(replyDraft(byId('s1'), work, false).to).toBe('Peter Mwangi <peter@client.example>');
  });

  it('quotes HTML-only messages as text', () => {
    const draft = replyDraft(byId('m2'), work, false);
    expect(draft.bodyText).toContain('> The nightly build of main finished without errors.');
    expect(draft.bodyText).not.toContain('<p>');
  });
});

describe('forward and edit', () => {
  it('forwards with a header block and no recipients', () => {
    const draft = forwardDraft(byId('m1'), 'Africa/Nairobi');
    expect(draft.to).toBe('');
    expect(draft.subject).toBe('Fwd: Revised quotation for the Westlands office fit-out');
    expect(draft.bodyText).toContain('From: Wanjiru Kamau <wanjiru@fitout.example>');
    expect(draft.bodyText).toContain('Cc: Amina Hassan <amina@studio.example>, you@work.example');
    expect(draft.replyToMessageId).toBeNull();
  });

  it('reopens a draft, replacing the saved copy and keeping its threading', () => {
    const files = [{ path: '/tmp/Menu.pdf', fileName: 'Menu.pdf', size: 1 }];
    const draft = draftFromMessage(byId('d1'), files);
    expect(draft.to).toBe('Brian Otieno <brian@personal.example>');
    expect(draft.bodyText).toBe('Friday works. 1pm at');
    expect(draft.draftMessageId).toBe('<d1@mock.example>');
    expect(draft.inReplyTo).toBe('<lunch-1@personal.example>');
    expect(draft.references).toEqual(['<lunch-1@personal.example>']);
    expect(draft.attachments).toEqual(files);
  });

  it('carries forwarded attachments and never a draft id', () => {
    const files = [{ path: '/tmp/a.pdf', fileName: 'a.pdf', size: 1 }];
    const draft = forwardDraft(byId('m1'), 'UTC', files);
    expect(draft.attachments).toEqual(files);
    expect(draft.draftMessageId).toBeNull();
  });
});

describe('text helpers', () => {
  it('turns HTML into readable text without running anything', () => {
    expect(htmlToText('<p>One</p><p>Two<br>Three</p><script>alert(1)</script>')).toBe('One\nTwo\nThree');
    expect(bodyTextOf({ text: null, html: null })).toBe('');
  });

  it('quotes nested quotes without a space', () => {
    expect(quote('a\n> b')).toBe('> a\n>> b');
  });
});

describe('toolbarActions', () => {
  it('fits the folder role', () => {
    expect(toolbarActions('inbox')).toEqual(['reply', 'replyAll', 'forward', 'archive', 'delete', 'flag', 'read']);
    expect(toolbarActions('trash')).toEqual(['reply', 'replyAll', 'forward', 'deleteForever', 'flag', 'read']);
    expect(toolbarActions('drafts')).toEqual(['edit', 'delete']);
  });
});
