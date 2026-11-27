import { useEffect, useId, useState } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { AppError, ImageSender } from '../../api/types';

/** Settings › Images: the senders (and domains) whose pictures always load, each with Remove. */
export function TrustedSenders() {
  const api = useMailApi();
  const id = useId();
  const [senders, setSenders] = useState<ImageSender[] | null>(null);
  const [error, setError] = useState<AppError | null>(null);

  useEffect(() => {
    api.listImageSenders().then(setSenders, (e: unknown) => {
      setError(toAppError(e));
      setSenders([]);
    });
  }, [api]);

  return (
    <div className="settings__sub trusted">
      <h3 id={`${id}-title`} className="form-field__label">
        Trusted senders
      </h3>
      {senders === null ? null : senders.length === 0 ? (
        <p className="form-hint">
          None yet. When pictures are blocked in a message, choose “Always show from” to add its sender here.
        </p>
      ) : (
        <ul className="trusted__list" aria-labelledby={`${id}-title`}>
          {senders.map((s) => (
            <li key={s.sender} className="trusted__row">
              <span className="trusted__who">
                {s.wholeDomain ? `Anyone at ${s.sender}` : s.sender}
              </span>
              <button
                type="button"
                className="btn btn--link btn--danger-text"
                aria-label={`Stop trusting ${s.sender}`}
                onClick={() => {
                  setError(null);
                  api.untrustImageSender(s.sender).then(setSenders, (e: unknown) => {
                    setError(toAppError(e));
                  });
                }}
              >
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
    </div>
  );
}
