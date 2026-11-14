import type { OutboxEntry } from '../../api/types';
import { formatClock } from '../../lib/format';

export type Tone = 'warn' | 'info' | 'error' | 'ok';

export interface OutboxCopy {
  tone: Tone;
  /** Short label for the list. */
  pill: string;
  title: string;
  text: string;
  /** Detail from the backend, shown small under the text. */
  detail: string | null;
  /** 0 to 100 while sending; null when not sending or unknown. */
  progress: number | null;
  canSendNow: boolean;
  /** Text on the send button when `canSendNow`. */
  sendLabel: string;
  canEdit: boolean;
  canCancel: boolean;
}

/** Entries the Outbox view shows; sent and cancelled ones are finished. */
export function isActive(entry: OutboxEntry): boolean {
  return entry.status.state === 'queued' || entry.status.state === 'sending' || entry.status.state === 'failed';
}

function percent(progress: number | null): number | null {
  if (progress === null || !Number.isFinite(progress)) return null;
  return Math.min(100, Math.max(0, Math.round(progress * 100)));
}

function when(nextAt: number, now: number, timeZone?: string): { short: string; long: string } {
  const seconds = Math.max(0, Math.round(nextAt - now));
  if (seconds < 60) return { short: `in ${String(seconds)} s`, long: `in ${String(seconds)} seconds` };
  const clock = formatClock(nextAt, timeZone);
  return { short: `at ${clock}`, long: `at ${clock}` };
}

/** What the list pill and the reading-pane banner say about an Outbox entry. */
export function outboxCopy(entry: OutboxEntry, accountLabel: string, now: number, timeZone?: string): OutboxCopy {
  const base = { detail: null, progress: null, canSendNow: false, sendLabel: 'Send now', canEdit: false, canCancel: false };
  const { status } = entry;
  switch (status.state) {
    case 'queued': {
      if (status.maybe_sent) {
        return {
          ...base,
          tone: 'warn',
          pill: 'Checking',
          title: 'Checking whether this was sent',
          text: 'Mocky Mail stopped while sending this message. It is checking your Sent folder first, so the message is never sent twice.',
          canEdit: true,
          canCancel: true,
        };
      }
      const retrying = entry.attempts > 0 && status.next_attempt_at > now;
      if (retrying) {
        const t = when(status.next_attempt_at, now, timeZone);
        return {
          ...base,
          tone: 'warn',
          pill: `Retrying ${t.short}`,
          title: 'Waiting for a better connection',
          text: `The last attempt did not go through. Mocky Mail will try again ${t.long}. The message is safe on this computer.`,
          canSendNow: true,
          canEdit: true,
          canCancel: true,
        };
      }
      return {
        ...base,
        tone: 'warn',
        pill: 'Waiting to send',
        title: 'Waiting to send',
        text: 'This message is saved on this computer and sends as soon as the connection allows.',
        canSendNow: true,
        canEdit: true,
        canCancel: true,
      };
    }
    case 'sending': {
      const progress = percent(entry.progress);
      return {
        ...base,
        tone: 'info',
        pill: progress === null ? 'Sending' : `Sending ${String(progress)}%`,
        title: progress === null ? 'Sending' : `Sending, ${String(progress)}% uploaded`,
        text: `Sending through the ${accountLabel} account. You can keep working; this happens in the background.`,
        progress,
      };
    }
    case 'failed':
      switch (status.reason) {
        case 'too_large':
          return {
            ...base,
            tone: 'error',
            pill: 'Too large to send',
            title: 'Couldn\u2019t send: the message is too large',
            text: `The ${accountLabel} mail server does not accept a message this size. Remove an attachment or share it as a link, then send again.`,
            detail: status.message,
            canEdit: true,
            canCancel: true,
          };
        case 'rejected':
          return {
            ...base,
            tone: 'error',
            pill: 'Rejected',
            title: 'The server refused this message',
            text: 'Check the addresses, then edit and send again.',
            detail: status.message,
            canSendNow: true,
            canEdit: true,
            canCancel: true,
          };
        case 'unconfirmed':
          return {
            ...base,
            tone: 'error',
            pill: 'May have been sent',
            title: 'This message may already have been sent',
            text: 'Mocky Mail couldn\u2019t confirm whether it went out. Check your Sent folder on the web or ask the recipient. Send it again only if it didn\u2019t arrive.',
            detail: status.message,
            canSendNow: true,
            sendLabel: 'Send again',
            canEdit: true,
            canCancel: true,
          };
        case 'gave_up':
          return {
            ...base,
            tone: 'error',
            pill: 'Couldn\u2019t send',
            title: 'Couldn\u2019t send after several tries',
            text: 'The connection kept failing. Try again when you are back online.',
            detail: status.message,
            canSendNow: true,
            canEdit: true,
            canCancel: true,
          };
      }
    // The inner switch is exhaustive; this is unreachable.
    // falls through
    case 'sent':
      return { ...base, tone: 'ok', pill: 'Sent', title: 'Sent', text: 'The server accepted this message.' };
    case 'cancelled':
      return { ...base, tone: 'ok', pill: 'Cancelled', title: 'Cancelled', text: 'This message will not be sent.' };
  }
}
