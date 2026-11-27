import { useState } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { AppError, MessageDetail, Settings } from '../../api/types';
import { Icon } from '../../ui/Icon';

/**
 * A message that asks for a read receipt. With "Ask each time" a bar asks
 * whether to send one; "Always send" leaves it to the backend (see below);
 * "Never send" does nothing, so the sender is never told.
 */
export function ReadReceiptBar({ message, mode }: { message: MessageDetail; mode: Settings['readReceipts'] }) {
  const api = useMailApi();
  const asked = message.readReceipt ?? null;
  const [answered, setAnswered] = useState<null | boolean>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  const open = asked !== null && !asked.answered && answered === null;

  const answer = (send: boolean) => {
    setBusy(true);
    setError(null);
    api.answerReadReceipt(message.id, send).then(
      () => {
        setBusy(false);
        setAnswered(send);
      },
      (e: unknown) => {
        setBusy(false);
        setError(toAppError(e));
      },
    );
  };
  // "Always send": the backend sends the receipt itself when the message is
  // opened. It leaves one unanswered only when it won't send by itself (the
  // reply would go to a different domain than the sender's), so ask then.
  const asking = mode === 'ask' || mode === 'always';

  if (asking && open) {
    return (
      <div className="receipt-bar" role="group" aria-label="Read receipt">
        <Icon name="envelopeOpen" />
        <span>{asked.requestedBy} asked to know when you read this.</span>
        <span className="receipt-bar__actions">
          <button type="button" className="btn btn--secondary btn--small" disabled={busy} onClick={() => { answer(true); }}>
            Send receipt
          </button>
          <button type="button" className="btn btn--ghost btn--small" disabled={busy} onClick={() => { answer(false); }}>
            Don’t send
          </button>
        </span>
        {error && (
          <span className="form-error" role="alert">
            {error.message}
          </span>
        )}
      </div>
    );
  }
  if (asking && answered !== null) {
    return (
      <p className="receipt-bar receipt-bar--done" role="status">
        {answered ? 'Read receipt sent.' : 'No read receipt was sent.'}
      </p>
    );
  }
  return null;
}
