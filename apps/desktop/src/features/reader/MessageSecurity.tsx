import { useId, useState } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { appError, toAppError } from '../../api/errors';
import type { MessageDetail, SignatureCheck } from '../../api/types';
import { formatLongDate } from '../../lib/format';
import { Icon, type IconName } from '../../ui/Icon';
import { formatFingerprint, kindLabel, kindName, sameFingerprint } from '../security/securityText';
import { TrustKeyDialog } from '../security/TrustKeyDialog';

interface Props {
  message: MessageDetail;
  /** Something changed that the message's check depends on (a key was trusted): load it again. */
  onChanged: () => void;
  /** Open Settings › Security, to add the key a locked message needs. */
  onOpenSecurity?: (() => void) | undefined;
}

/** The signature bar and the encryption note, above the message. */
export function MessageSecurity({ message, onChanged, onOpenSecurity }: Props) {
  return (
    <>
      {message.signature && <SignatureBar signature={message.signature} onChanged={onChanged} />}
      {message.encryption && <EncryptionNote encryption={message.encryption} onOpenSecurity={onOpenSecurity} />}
    </>
  );
}

const TONE: Record<SignatureCheck['result'], string> = {
  verified: 'ok',
  unverifiedSigner: 'warn',
  changed: 'bad',
  unknown: 'neutral',
};

const ICON: Record<SignatureCheck['result'], IconName> = {
  verified: 'seal',
  unverifiedSigner: 'sealAlert',
  changed: 'sealAlert',
  unknown: 'sealUnknown',
};

/** "Wanjiru Kamau <wanjiru@fitout.example>", or whichever of the two is known. */
function signerText(signer: SignatureCheck['signer']): string | null {
  if (!signer) return null;
  if (signer.name && signer.address) return `${signer.name} <${signer.address}>`;
  return signer.name ?? signer.address;
}

function headline(signature: SignatureCheck): string {
  switch (signature.result) {
    case 'verified': {
      const who = signerText(signature.signer);
      return who ? `Signed by ${who}` : 'Signed';
    }
    case 'unverifiedSigner':
      return 'Signed, but the signer isn’t verified';
    case 'changed':
      return 'Changed after it was signed — don’t trust it';
    case 'unknown':
      return 'Signature can’t be checked';
  }
}

function SignatureBar({ signature, onChanged }: { signature: SignatureCheck; onChanged: () => void }) {
  const api = useMailApi();
  const id = useId();
  const [open, setOpen] = useState(false);
  const [trusting, setTrusting] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { signer, result } = signature;
  const fingerprint = signer?.fingerprint ?? null;
  const canTrust = signature.kind === 'openpgp' && result === 'unverifiedSigner' && fingerprint !== null;
  const who = signerText(signer);

  const trust = () => {
    if (fingerprint === null) return;
    setBusy(true);
    setError(null);
    api
      .listKeys()
      .then((keys) => {
        const found = keys.find((k) => !k.isOwn && k.kind === 'openpgp' && sameFingerprint(k.fingerprint, fingerprint));
        if (!found) throw appError('not_found', 'This key isn’t on this device any more, so it can’t be trusted here.');
        return api.setKeyTrusted(found.id, true);
      })
      .then(
        () => {
          setBusy(false);
          setTrusting(false);
          onChanged();
        },
        (e: unknown) => {
          setBusy(false);
          setError(toAppError(e).message);
        },
      );
  };

  return (
    <div className={`sig-bar sig-bar--${TONE[result]}`} role="group" aria-label="Signature">
      <div className="sig-bar__main">
        <Icon name={ICON[result]} />
        <div className="sig-bar__text">
          <p className="sig-bar__title">{headline(signature)}</p>
          {result === 'unknown' && <p className="sig-bar__detail">{signature.detail}</p>}
          {signature.fromMismatch && (
            <p className="sig-bar__mismatch">
              <strong>The signer isn’t the sender shown.</strong>
              {signer?.address ? ` It was signed by ${signer.address}.` : ''}
            </p>
          )}
        </div>
        <span className="sig-bar__actions">
          {canTrust && (
            <button
              type="button"
              className="btn btn--secondary btn--small"
              onClick={() => {
                setError(null);
                setTrusting(true);
              }}
            >
              Trust this key
            </button>
          )}
          <button
            type="button"
            className="btn btn--ghost btn--small"
            aria-expanded={open}
            aria-controls={`${id}-details`}
            onClick={() => {
              setOpen((on) => !on);
            }}
          >
            Details
          </button>
        </span>
      </div>
      {open && (
        <dl id={`${id}-details`} className="sig-bar__details" aria-label="Signature details">
          <dt>Signed by</dt>
          <dd>{who ?? 'Not known'}</dd>
          {signer?.issuer && (
            <>
              <dt>Issued by</dt>
              <dd>{signer.issuer}</dd>
            </>
          )}
          {fingerprint !== null && (
            <>
              <dt>Fingerprint</dt>
              <dd className="fingerprint">{formatFingerprint(fingerprint)}</dd>
            </>
          )}
          {signature.signedAt !== null && (
            <>
              <dt>Signed at</dt>
              <dd>{formatLongDate(signature.signedAt)}</dd>
            </>
          )}
          <dt>Kind</dt>
          <dd>{kindName(signature.kind)}</dd>
          {result !== 'unknown' && (
            <>
              <dt>What this means</dt>
              <dd>{signature.detail}</dd>
            </>
          )}
        </dl>
      )}
      {trusting && fingerprint !== null && (
        <TrustKeyDialog
          owner={who ?? 'the sender'}
          fingerprint={fingerprint}
          busy={busy}
          error={error}
          onConfirm={trust}
          onCancel={() => {
            setTrusting(false);
          }}
        />
      )}
    </div>
  );
}

function EncryptionNote({
  encryption,
  onOpenSecurity,
}: {
  encryption: NonNullable<MessageDetail['encryption']>;
  onOpenSecurity?: (() => void) | undefined;
}) {
  if (encryption.state === 'decrypted') {
    return (
      <p className="lock-note" role="note">
        <Icon name="lock" size={16} />
        <span>Encrypted — decrypted on this device</span>
      </p>
    );
  }
  const noun = kindLabel(encryption.kind);
  return (
    <div className="lock-bar" role="group" aria-label="Encrypted message">
      <Icon name="lock" />
      <div className="lock-bar__text">
        <p className="lock-bar__title">This message is encrypted, and this device can’t open it</p>
        {encryption.detail && <p>{encryption.detail}</p>}
        <p>
          To read it, add the {noun} it was encrypted to in{' '}
          {onOpenSecurity ? (
            <button type="button" className="btn btn--link btn--inline" onClick={onOpenSecurity}>
              Settings › Security
            </button>
          ) : (
            'Settings › Security'
          )}
          .
        </p>
      </div>
    </div>
  );
}
