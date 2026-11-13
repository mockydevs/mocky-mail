import type { MessageDetail } from '../../api/types';
import { formatLongDate } from '../../lib/format';

/** Starting values for the compose form. */
export interface ComposeSeed {
  accountId: string;
  to: string;
  subject: string;
  body: string;
  /** Outbox message this edit replaces once the new one is queued. */
  replacesOutboxId?: string;
}

function prefixed(prefix: 'Re' | 'Fwd', subject: string): string {
  const pattern = prefix === 'Re' ? /^re:/i : /^(fwd?|fw):/i;
  const trimmed = subject.trim();
  return pattern.test(trimmed) ? trimmed : `${prefix}: ${trimmed}`;
}

export function replySubject(subject: string): string {
  return prefixed('Re', subject);
}

function bodyText(message: MessageDetail): string {
  return message.body.kind === 'text' ? message.body.text : '';
}

export function blankSeed(accountId: string): ComposeSeed {
  return { accountId, to: '', subject: '', body: '' };
}

export function replySeed(message: MessageDetail): ComposeSeed {
  return {
    accountId: message.accountId,
    to: message.counterpart.address,
    subject: replySubject(message.subject),
    body: '',
  };
}

export function forwardSeed(message: MessageDetail, timeZone?: string): ComposeSeed {
  const header = [
    '---------- Forwarded message ----------',
    `From: ${message.counterpart.name} <${message.counterpart.address}>`,
    `Date: ${formatLongDate(message.date, timeZone)}`,
    `Subject: ${message.subject}`,
  ].join('\n');
  const original = bodyText(message);
  return {
    accountId: message.accountId,
    to: '',
    subject: prefixed('Fwd', message.subject),
    body: `\n\n${header}\n\n${original}`.trimEnd(),
  };
}

/** Reopen a queued or draft message for editing. */
export function editSeed(message: MessageDetail): ComposeSeed {
  const seed: ComposeSeed = {
    accountId: message.accountId,
    to: message.counterpart.address,
    subject: message.subject,
    body: bodyText(message),
  };
  return message.folder === 'outbox' ? { ...seed, replacesOutboxId: message.id } : seed;
}

export type ToolbarAction = 'reply' | 'forward' | 'edit' | 'archive' | 'delete' | 'flag';

/** Which reading-pane actions make sense for this message. */
export function toolbarActions(message: Pick<MessageDetail, 'folder' | 'direction'>): ToolbarAction[] {
  if (message.folder === 'outbox') return [];
  if (message.folder === 'drafts') return ['edit', 'delete'];
  const actions: ToolbarAction[] = message.direction === 'incoming' ? ['reply', 'forward'] : ['forward'];
  if (message.folder !== 'archive' && message.folder !== 'trash') actions.push('archive');
  if (message.folder !== 'trash') actions.push('delete');
  actions.push('flag');
  return actions;
}
