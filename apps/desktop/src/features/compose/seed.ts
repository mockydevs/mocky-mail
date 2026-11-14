import type { Account, Draft, FolderRole, Mailbox, MessageDetail } from '../../api/types';
import { formatLongDate } from '../../lib/format';

/** "Wanjiru Kamau <wanjiru@fitout.example>" or the bare address. */
export function formatMailbox(mailbox: Mailbox): string {
  const name = mailbox.name?.replace(/["<>]/g, '').trim();
  return name ? `${name} <${mailbox.address}>` : mailbox.address;
}

export function formatMailboxes(list: readonly Mailbox[]): string {
  return list.map(formatMailbox).join(', ');
}

export function blankDraft(accountId: string): Draft {
  return {
    accountId,
    to: '',
    cc: '',
    bcc: '',
    subject: '',
    bodyText: '',
    inReplyTo: null,
    references: [],
    attachments: [],
    replyToMessageId: null,
    draftMessageId: null,
  };
}

function prefixed(prefix: 'Re' | 'Fwd', subject: string): string {
  const pattern = prefix === 'Re' ? /^re:/i : /^(fwd?|fw):/i;
  const trimmed = subject.trim();
  return pattern.test(trimmed) ? trimmed : `${prefix}: ${trimmed}`;
}

export const replySubject = (subject: string): string => prefixed('Re', subject);
export const forwardSubject = (subject: string): string => prefixed('Fwd', subject);

/**
 * Plain text of a sanitized HTML body. DOMParser builds an inert document:
 * no scripts run and nothing loads.
 */
export function htmlToText(html: string): string {
  const doc = new DOMParser().parseFromString(html, 'text/html');
  for (const hidden of doc.querySelectorAll('script, style, template')) hidden.remove();
  for (const block of doc.body.querySelectorAll('p, div, br, li, tr, h1, h2, h3, h4')) {
    block.append(doc.createTextNode('\n'));
  }
  return doc.body.textContent
    .replace(/[ \t]+\n/g, '\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

export function bodyTextOf(message: Pick<MessageDetail, 'text' | 'html'>): string {
  if (message.text !== null) return message.text;
  return message.html !== null ? htmlToText(message.html) : '';
}

export function quote(text: string): string {
  return text
    .split('\n')
    .map((line) => (line.startsWith('>') ? `>${line}` : `> ${line}`).trimEnd())
    .join('\n');
}

function sameAddress(a: string, b: string): boolean {
  return a.trim().toLowerCase() === b.trim().toLowerCase();
}

function dedupe(list: readonly Mailbox[], exclude: readonly string[]): Mailbox[] {
  const seen = new Set(exclude.map((a) => a.toLowerCase()));
  const out: Mailbox[] = [];
  for (const mailbox of list) {
    const key = mailbox.address.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(mailbox);
  }
  return out;
}

function threading(message: MessageDetail): Pick<Draft, 'inReplyTo' | 'references'> {
  const references = [...message.references];
  if (message.messageIdHeader && !references.includes(message.messageIdHeader)) {
    references.push(message.messageIdHeader);
  }
  return { inReplyTo: message.messageIdHeader, references };
}

/**
 * Reply or reply all. Replies go to Reply-To when the sender set one. Your
 * own address is never added. Replying to your own sent mail goes to its
 * original recipients.
 */
export function replyDraft(message: MessageDetail, account: Account, all: boolean, timeZone?: string): Draft {
  const own = account.email;
  const fromMe = message.from !== null && sameAddress(message.from.address, own);
  let primary: Mailbox[];
  if (fromMe) primary = message.to;
  else if (message.replyTo.length > 0) primary = message.replyTo;
  else primary = message.from ? [message.from] : [];
  const to = dedupe(all && !fromMe ? [...primary, ...message.to] : primary, [own]);
  const cc = all ? dedupe(message.cc, [own, ...to.map((m) => m.address)]) : [];
  const sender = message.from ? message.from.name ?? message.from.address : 'someone';
  const header = `On ${formatLongDate(message.date, timeZone)}, ${sender} wrote:`;
  return {
    ...blankDraft(message.accountId),
    ...threading(message),
    to: formatMailboxes(to),
    cc: formatMailboxes(cc),
    subject: replySubject(message.subject),
    bodyText: `\n\n${header}\n${quote(bodyTextOf(message))}`,
    replyToMessageId: message.id,
  };
}

export function forwardDraft(message: MessageDetail, timeZone?: string): Draft {
  const header = [
    '---------- Forwarded message ----------',
    `From: ${message.from ? formatMailbox(message.from) : ''}`,
    `Date: ${formatLongDate(message.date, timeZone)}`,
    `Subject: ${message.subject}`,
    `To: ${formatMailboxes(message.to)}`,
    ...(message.cc.length > 0 ? [`Cc: ${formatMailboxes(message.cc)}`] : []),
  ].join('\n');
  return {
    ...blankDraft(message.accountId),
    subject: forwardSubject(message.subject),
    bodyText: `\n\n${header}\n\n${bodyTextOf(message)}`,
    references: threading(message).references,
  };
}

/** Continue editing a message from Drafts. */
export function draftFromMessage(message: MessageDetail): Draft {
  return {
    ...blankDraft(message.accountId),
    to: formatMailboxes(message.to),
    cc: formatMailboxes(message.cc),
    subject: message.subject,
    bodyText: bodyTextOf(message),
    references: message.references,
  };
}

export type ToolbarAction = 'reply' | 'replyAll' | 'forward' | 'edit' | 'archive' | 'delete' | 'deleteForever' | 'flag' | 'read';

/** Which reading-pane actions apply to a message in a folder with this role. */
export function toolbarActions(role: FolderRole | null): ToolbarAction[] {
  if (role === 'drafts') return ['edit', 'delete'];
  const actions: ToolbarAction[] = ['reply', 'replyAll', 'forward'];
  if (role !== 'archive' && role !== 'trash' && role !== 'junk') actions.push('archive');
  actions.push(role === 'trash' || role === 'junk' ? 'deleteForever' : 'delete');
  actions.push('flag', 'read');
  return actions;
}
