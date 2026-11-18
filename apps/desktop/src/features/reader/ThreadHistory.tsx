import { useEffect, useState } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { MessageDetail, MessageListItem } from '../../api/types';
import { formatListTime, formatLongDate, nowSeconds } from '../../lib/format';
import type { Theme } from '../../theme/useTheme';
import { Avatar } from '../../ui/Avatar';
import { SandboxedHtml } from './SandboxedHtml';

interface Props {
  /** The message open in the reader; its earlier conversation is shown. */
  messageId: string;
  theme: Theme;
}

/**
 * The earlier messages of the open message's conversation (sent ones
 * included), oldest first, folded to one line each. Tapping one unfolds it
 * here.
 */
export function ThreadHistory({ messageId, theme }: Props) {
  const api = useMailApi();
  const [earlier, setEarlier] = useState<MessageListItem[]>([]);
  const [open, setOpen] = useState<Record<string, MessageDetail | 'loading' | { error: string }>>({});

  useEffect(() => {
    let live = true;
    api.getThread(messageId).then(
      (thread) => {
        if (!live) return;
        const at = thread.findIndex((m) => m.id === messageId);
        // Only what came before the open message; later replies are rows in the list.
        setEarlier(at > 0 ? thread.slice(0, at) : []);
      },
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, [api, messageId]);

  if (earlier.length === 0) return null;

  const toggle = (id: string) => {
    if (open[id] !== undefined) {
      setOpen((o) => Object.fromEntries(Object.entries(o).filter(([key]) => key !== id)));
      return;
    }
    setOpen((o) => ({ ...o, [id]: 'loading' }));
    api.getMessage(id, false).then(
      (detail) => {
        setOpen((o) => ({ ...o, [id]: detail }));
      },
      (e: unknown) => {
        setOpen((o) => ({ ...o, [id]: { error: toAppError(e).message } }));
      },
    );
  };

  const now = nowSeconds();
  return (
    <section className="thread" aria-label={`Earlier in this conversation, ${String(earlier.length)} messages`}>
      {earlier.map((m) => {
        const state = open[m.id];
        const sender = m.from?.name ?? m.from?.address ?? 'Unknown sender';
        return (
          <article key={m.id} className="thread__item">
            <button
              type="button"
              className="thread__head"
              aria-expanded={state !== undefined}
              onClick={() => {
                toggle(m.id);
              }}
            >
              <Avatar name={m.from?.name ?? ''} address={m.from?.address ?? ''} size={28} />
              <span className="thread__who">{sender}</span>
              <span className="thread__preview">{state === undefined ? m.preview : ''}</span>
              <time className="thread__when" title={formatLongDate(m.date)}>
                {formatListTime(m.date, now)}
              </time>
            </button>
            {state === 'loading' && (
              <p className="muted thread__body" role="status">
                Loading…
              </p>
            )}
            {state !== undefined && state !== 'loading' && 'error' in state && (
              <p className="form-error thread__body" role="alert">
                {state.error}
              </p>
            )}
            {state !== undefined && state !== 'loading' && !('error' in state) && (
              <div className="thread__body">
                {state.html !== null ? (
                  <SandboxedHtml title={`Message: ${state.subject}`} html={state.html} theme={theme} fitContent />
                ) : (
                  <div className="thread__text">{state.text}</div>
                )}
              </div>
            )}
          </article>
        );
      })}
    </section>
  );
}
