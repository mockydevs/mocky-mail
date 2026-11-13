import type { OutboxStatus } from '../../api/types';
import { formatBytes } from '../../lib/format';

export type Tone = 'warn' | 'info' | 'error';

export interface OutboxCopy {
  tone: Tone;
  /** Short label for the message list. */
  pill: string;
  title: string;
  text: string;
  /** Progress from 0 to 100 while sending. */
  progress: number | null;
  canSendNow: boolean;
  canCancel: boolean;
}

function clampPercent(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(100, Math.max(0, Math.round(value)));
}

/** Everything the list pill and the reading-pane banner say about a queued message. */
export function outboxCopy(status: OutboxStatus, accountLabel: string): OutboxCopy {
  switch (status.kind) {
    case 'retrying': {
      const seconds = Math.max(0, Math.round(status.retryInSeconds));
      return {
        tone: 'warn',
        pill: seconds > 0 ? `Retrying in ${seconds} s` : 'Waiting to send',
        title: 'Waiting for a better connection',
        text:
          seconds > 0
            ? `The connection dropped while sending. Mocky Mail will try again in ${seconds} seconds. The message is safe on this computer.`
            : 'This message is saved on this computer and will send as soon as the connection allows.',
        progress: null,
        canSendNow: true,
        canCancel: true,
      };
    }
    case 'sending': {
      const progress = clampPercent(status.progress);
      return {
        tone: 'info',
        pill: `Sending ${progress}%`,
        title: `Sending, ${progress}% uploaded`,
        text: `Sending through the ${accountLabel} account. You can keep working, this happens in the background.`,
        progress,
        canSendNow: false,
        canCancel: false,
      };
    }
    case 'failed':
      if (status.reason === 'too_large') {
        return {
          tone: 'error',
          pill: 'Too large to send',
          title: 'Couldn’t send: attachments are too large',
          text: `This message is about ${formatBytes(status.estimatedBytes)} once encoded. The ${accountLabel} mail server accepts up to ${formatBytes(status.limitBytes)}. Remove an attachment and send again.`,
          progress: null,
          canSendNow: false,
          canCancel: true,
        };
      }
      return {
        tone: 'error',
        pill: status.reason === 'rejected' ? 'Rejected' : 'Couldn’t send',
        title: status.reason === 'rejected' ? 'The server refused this message' : 'Couldn’t send after several tries',
        text: status.detail,
        progress: null,
        canSendNow: true,
        canCancel: true,
      };
  }
}
