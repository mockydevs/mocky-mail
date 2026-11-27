import type { Account, Draft, DraftAttachment, FolderRole, Mailbox, MessageDetail } from '../../api/types';
import { formatLongDate } from '../../lib/format';
import { quotedOriginal, signatureBlock, SIGNATURE_SEPARATOR } from './richText';

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
    // The original stays as HTML, quoted under where the user types.
    bodyHtml: `<p></p>${quotedOriginal([header], message, true)}`,
    replyToMessageId: message.id,
  };
}

/**
 * Forward with the original text. `attachments` are copies from
 * attachments_for_draft; the caller loads them because that can fail.
 */
export function forwardDraft(message: MessageDetail, timeZone?: string, attachments: DraftAttachment[] = []): Draft {
  const lines = [
    '---------- Forwarded message ----------',
    `From: ${message.from ? formatMailbox(message.from) : ''}`,
    `Date: ${formatLongDate(message.date, timeZone)}`,
    `Subject: ${message.subject}`,
    `To: ${formatMailboxes(message.to)}`,
    ...(message.cc.length > 0 ? [`Cc: ${formatMailboxes(message.cc)}`] : []),
  ];
  const header = lines.join('\n');
  return {
    ...blankDraft(message.accountId),
    subject: forwardSubject(message.subject),
    bodyText: `\n\n${header}\n\n${bodyTextOf(message)}`,
    bodyHtml: `<p></p>${quotedOriginal(lines, message, false)}`,
    references: threading(message).references,
    attachments,
  };
}

/**
 * Continue editing a message from Drafts. Saving or sending replaces that
 * saved copy, so the server keeps only one.
 */
export function draftFromMessage(message: MessageDetail, attachments: DraftAttachment[] = []): Draft {
  return {
    ...blankDraft(message.accountId),
    to: formatMailboxes(message.to),
    cc: formatMailboxes(message.cc),
    // Only a draft still has its Bcc; received mail never carries one.
    bcc: formatMailboxes(message.bcc ?? []),
    subject: message.subject,
    bodyText: bodyTextOf(message),
    // Written in the rich-text editor: carry on with its HTML.
    bodyHtml: message.html,
    inReplyTo: message.inReplyTo,
    references: message.references,
    attachments,
    draftMessageId: message.messageIdHeader,
  };
}

/**
 * What compose was opened for. It is said outright, not guessed from the
 * draft: a forward of a message without attachments looks like a new message.
 */
export type ComposeKind = 'new' | 'reply' | 'forward' | 'draft';

/**
 * Which of the account's default signatures a message starts with. A draft
 * (or a message taken back from the Outbox) already has whatever it was
 * written with, so it gets none.
 */
export function signatureFor(kind: ComposeKind): 'new' | 'reply' | null {
  if (kind === 'draft') return null;
  return kind === 'new' ? 'new' : 'reply';
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

export { SIGNATURE_SEPARATOR, signatureBlock };

/**
 * Add a signature under where the user types: after the (empty) top of a new
 * message, above the quoted original in a reply or forward.
 */
export function withSignature(body: string, text: string | null): string {
  if (!text) return body;
  const block = signatureBlock(text);
  if (body.includes(block)) return body;
  if (body === '') return `\n\n${block}`;
  return `\n\n${block}${body.startsWith('\n') ? body : `\n\n${body}`}`;
}

/**
 * Swap the signature when the From account changes. A block the user edited
 * is left alone (it is no longer the old signature), and no second one is added.
 */
export function swapSignature(body: string, previous: string | null, next: string | null): string {
  if (previous === next) return body;
  if (previous) {
    const old = signatureBlock(previous);
    if (!body.includes(old)) return body;
    if (next) return body.replace(old, signatureBlock(next));
    const at = body.indexOf(old);
    const before = body.slice(0, at).replace(/\n\n$/, '');
    return before + body.slice(at + old.length);
  }
  return withSignature(body, next);
}
