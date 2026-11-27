import { Modal } from '../../ui/Modal';
import { formatFingerprint } from './securityText';

interface Props {
  /** Who the key says it belongs to: a name, an address, or both. */
  owner: string;
  fingerprint: string;
  busy?: boolean;
  error?: string | null;
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * Before saying another person's OpenPGP key really is theirs: compare its
 * fingerprint with the one they give you some other way. Trusting a key
 * makes the messages it signed show as verified.
 */
export function TrustKeyDialog({ owner, fingerprint, busy = false, error = null, onConfirm, onCancel }: Props) {
  return (
    <Modal
      title="Trust this key?"
      description={`Only do this if you’re sure the key belongs to ${owner}. Ask them for their key’s fingerprint in person, on a call, or some other way you know is really them, and check it matches this one exactly.`}
      size="small"
      onClose={onCancel}
    >
      <p className="fingerprint" aria-label="Fingerprint">
        {formatFingerprint(fingerprint)}
      </p>
      <p className="form-hint">
        Once you trust it, messages signed with this key show as verified. Someone else could send a key with the same
        name and address, so the fingerprint is what counts.
      </p>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      <div className="modal__actions">
        <button type="button" className="btn btn--secondary" onClick={onCancel}>
          Cancel
        </button>
        <button type="button" className="btn btn--primary" disabled={busy} onClick={onConfirm}>
          Trust this key
        </button>
      </div>
    </Modal>
  );
}
