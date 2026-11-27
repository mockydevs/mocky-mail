import type { SecurityKind } from '../../api/types';
import { Modal } from '../../ui/Modal';
import { kindNoun } from '../security/securityText';

interface Props {
  /** Recipients there is no key or certificate for. */
  missing: readonly string[];
  kind: SecurityKind;
  /** Taking them out would leave nobody to send to. */
  everyone: boolean;
  onRemove: () => void;
  onSendUnencrypted: () => void;
  onCancel: () => void;
}

/** Send with Encrypt on, but some recipients couldn't read an encrypted message. */
export function MissingKeysDialog({ missing, kind, everyone, onRemove, onSendUnencrypted, onCancel }: Props) {
  const noun = kindNoun(kind);
  const list = missing.join(', ');
  const one = missing.length === 1;
  return (
    <Modal
      title="Can’t encrypt to everyone"
      description={`There’s no ${noun} for ${list}, so ${one ? 'they' : 'those recipients'} couldn’t read an encrypted message.`}
      size="small"
      onClose={onCancel}
    >
      {everyone && (
        <p className="form-hint">Nobody you’re writing to can get encrypted mail yet, so there’s no one left to remove.</p>
      )}
      <div className="modal__actions modal__actions--stack">
        <button type="button" className="btn btn--secondary" onClick={onCancel}>
          Cancel
        </button>
        <button type="button" className="btn btn--secondary" onClick={onSendUnencrypted}>
          Send without encryption
        </button>
        {!everyone && (
          <button type="button" className="btn btn--primary" onClick={onRemove}>
            {one ? `Remove ${list} and send encrypted` : 'Remove them and send encrypted'}
          </button>
        )}
      </div>
    </Modal>
  );
}
