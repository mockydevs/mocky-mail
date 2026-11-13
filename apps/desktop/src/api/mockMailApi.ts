import { MOCK_ACCOUNTS, MOCK_CONNECTION, MOCK_LABELS, MOCK_MESSAGES } from './mockData';
import type { Draft, FolderCounts, FolderId, MailApi, MessageDetail, MessageSummary } from './types';

function summaryOf(message: MessageDetail): MessageSummary {
  const summary: MessageSummary & Partial<Pick<MessageDetail, 'body' | 'attachments'>> = { ...message };
  delete summary.body;
  delete summary.attachments;
  return summary;
}

function previewOf(body: string): string {
  return body.split(/\s+/).filter(Boolean).join(' ').slice(0, 160);
}

export interface MockOptions {
  messages?: readonly MessageDetail[];
  now?: () => Date;
}

/**
 * In-memory MailApi for tests and the browser preview. Each call returns a
 * fresh, isolated mailbox so tests never leak state into each other.
 */
export function createMockMailApi(options: MockOptions = {}): MailApi {
  let messages: MessageDetail[] = (options.messages ?? MOCK_MESSAGES).map((m) => ({ ...m }));
  const now = options.now ?? (() => new Date());
  let nextId = 1;

  const update = (id: string, change: (m: MessageDetail) => MessageDetail | null): void => {
    messages = messages.flatMap((m) => {
      if (m.id !== id) return [m];
      const next = change(m);
      return next ? [next] : [];
    });
  };

  const counts = (): FolderCounts => {
    const unread = messages.filter((m) => m.folder === 'inbox' && m.unread);
    const unreadByAccount: Record<string, number> = {};
    for (const account of MOCK_ACCOUNTS) {
      unreadByAccount[account.id] = unread.filter((m) => m.accountId === account.id).length;
    }
    return {
      inboxUnread: unread.length,
      unreadByAccount,
      outbox: messages.filter((m) => m.folder === 'outbox').length,
      drafts: messages.filter((m) => m.folder === 'drafts').length,
    };
  };

  return {
    listAccounts: () => Promise.resolve(MOCK_ACCOUNTS),
    listLabels: () => Promise.resolve(MOCK_LABELS),
    listFolder: (folder: FolderId) =>
      Promise.resolve(
        messages
          .filter((m) => m.folder === folder)
          .sort((a, b) => b.date.localeCompare(a.date))
          .map(summaryOf),
      ),
    getMessage: (id) => Promise.resolve(messages.find((m) => m.id === id) ?? null),
    getCounts: () => Promise.resolve(counts()),
    getConnection: () => Promise.resolve(MOCK_CONNECTION),
    syncNow: () => Promise.resolve(),
    setFlagged: (id, flagged) => {
      update(id, (m) => ({ ...m, flagged }));
      return Promise.resolve();
    },
    markRead: (id) => {
      update(id, (m) => ({ ...m, unread: false }));
      return Promise.resolve();
    },
    moveToFolder: (id, folder) => {
      update(id, (m) => ({ ...m, folder }));
      return Promise.resolve();
    },
    queueMessage: (draft: Draft) => {
      const first = draft.to[0] ?? '';
      messages = [
        ...messages,
        {
          id: `q${String(nextId++)}`,
          folder: 'outbox',
          accountId: draft.accountId,
          direction: 'outgoing',
          counterpart: { name: first, address: first },
          subject: draft.subject,
          preview: previewOf(draft.body),
          date: now().toISOString(),
          unread: false,
          flagged: false,
          hasAttachments: false,
          labelIds: [],
          outbox: { kind: 'retrying', retryInSeconds: 0 },
          body: { kind: 'text', text: draft.body },
          attachments: [],
        },
      ];
      return Promise.resolve();
    },
    outbox: {
      sendNow: (id) => {
        update(id, (m) => ({ ...m, outbox: { kind: 'sending', progress: 0 } }));
        return Promise.resolve();
      },
      cancel: (id) => {
        update(id, (m) => (m.outbox?.kind === 'sending' ? m : null));
        return Promise.resolve();
      },
    },
  };
}
