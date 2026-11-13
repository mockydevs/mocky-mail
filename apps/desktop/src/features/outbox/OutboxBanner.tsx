import type { OutboxStatus } from '../../api/types';
import { Icon } from '../../ui/Icon';
import { outboxCopy } from './outboxCopy';

interface Props {
  status: OutboxStatus;
  accountLabel: string;
  onSendNow: () => void;
  onEdit: () => void;
  onCancel: () => void;
}

export function OutboxBanner({ status, accountLabel, onSendNow, onEdit, onCancel }: Props) {
  const copy = outboxCopy(status, accountLabel);
  return (
    <section className={`banner banner--${copy.tone}`} role="status" aria-label="Delivery status">
      <div className="banner__main">
        <Icon name={copy.tone === 'warn' ? 'clock' : 'alert'} size={20} />
        <div className="banner__copy">
          <p className="banner__title">{copy.title}</p>
          <p className="banner__text">{copy.text}</p>
          {copy.progress !== null && (
            <progress className="banner__progress" max={100} value={copy.progress} aria-label="Upload progress" />
          )}
        </div>
      </div>
      <div className="banner__actions">
        {copy.canSendNow && (
          <button type="button" className="btn btn--primary btn--small" onClick={onSendNow}>
            Send now
          </button>
        )}
        {status.kind !== 'sending' && (
          <button type="button" className="btn btn--secondary btn--small" onClick={onEdit}>
            Edit message
          </button>
        )}
        {copy.canCancel && (
          <button type="button" className="btn btn--link btn--small" onClick={onCancel}>
            {status.kind === 'failed' ? 'Delete message' : 'Cancel sending'}
          </button>
        )}
      </div>
    </section>
  );
}
