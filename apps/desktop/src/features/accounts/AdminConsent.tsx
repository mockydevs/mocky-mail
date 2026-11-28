import { useId, useState } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { copyText } from '../../lib/clipboard';
import { Icon } from '../../ui/Icon';

/** What an administrator needs to know, ready to paste into an email or chat. */
export function adminMessage(url: string): string {
  return [
    'Hello,',
    '',
    'I would like to use Mocky Mail, an email app, with my work Microsoft 365 account. Our organisation lets only an administrator approve new apps, so Microsoft asks for your approval first.',
    '',
    'Could you approve Mocky Mail for our organisation? Open this page while signed in as an administrator, check the permissions it lists (mail, calendars, contacts and tasks) and choose Accept:',
    '',
    url,
    '',
    'Approving it lets people here sign in to Mocky Mail with their own work accounts. It does not give Mocky Mail access to anyone’s mail until they sign in themselves.',
    '',
    'Thank you.',
  ].join('\n');
}

type Copied = null | 'copied' | 'failed';

/**
 * Microsoft 365 refused the sign-in because only an administrator may approve
 * new apps in this organisation. An administrator can open the approval page
 * here; anyone else copies a message for theirs, then tries again.
 */
export function AdminConsentPanel({
  url,
  onRetry,
  busy = false,
}: {
  url: string;
  onRetry: () => void;
  busy?: boolean;
}) {
  const api = useMailApi();
  const id = useId();
  const [copied, setCopied] = useState<Copied>(null);
  const [openError, setOpenError] = useState(false);
  const message = adminMessage(url);

  const open = () => {
    setOpenError(false);
    api.openExternal(url).catch(() => {
      setOpenError(true);
    });
  };

  const copy = () => {
    void copyText(message).then((ok) => {
      setCopied(ok ? 'copied' : 'failed');
    });
  };

  return (
    <section className="notice notice--warn admin-consent" role="alert" aria-labelledby={`${id}-title`}>
      <p id={`${id}-title`} className="notice__title">
        Your organisation needs an administrator to approve Mocky Mail.
      </p>
      <p>
        Microsoft 365 at your work or school lets only an administrator allow new apps. If that’s you, open the
        approval page and accept. If not, send your administrator a message asking them to. Once it’s approved, try
        again.
      </p>
      <div className="admin-consent__actions">
        <button type="button" className="btn btn--secondary btn--small" onClick={open}>
          <Icon name="external" size={16} />
          Open the approval page
        </button>
        <button type="button" className="btn btn--secondary btn--small" onClick={copy}>
          <Icon name="copy" size={16} />
          Copy a message for your administrator
        </button>
        <button type="button" className="btn btn--primary btn--small" disabled={busy} aria-busy={busy || undefined} onClick={onRetry}>
          {busy ? 'Trying again…' : 'Try again'}
        </button>
      </div>
      {copied === 'copied' && (
        <p className="notice__hint" role="status">
          Message copied. Paste it into an email or chat to your administrator.
        </p>
      )}
      {copied === 'failed' && (
        <label className="form-field">
          <span className="notice__hint">Couldn’t copy. Select the message and copy it yourself:</span>
          <textarea className="field code-box" readOnly rows={8} value={message} />
        </label>
      )}
      {openError && (
        <p className="notice__hint" role="status">
          Couldn’t open the browser. The approval page is: <code className="admin-consent__url">{url}</code>
        </p>
      )}
    </section>
  );
}
