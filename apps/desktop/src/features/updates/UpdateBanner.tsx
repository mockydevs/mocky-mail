import { useEffect, useState } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { AppError, UpdateInfo } from '../../api/types';
import { Icon } from '../../ui/Icon';

/** First check shortly after start, so it doesn't compete with the first sync. */
export const FIRST_CHECK_MS = 15_000;
/** Then every few hours, for copies left open for days. */
export const CHECK_EVERY_MS = 6 * 60 * 60 * 1000;
const DISMISSED_KEY = 'mocky.update.dismissed';

function dismissedVersion(): string | null {
  try {
    return window.localStorage.getItem(DISMISSED_KEY);
  } catch {
    return null;
  }
}

function dismiss(version: string) {
  try {
    window.localStorage.setItem(DISMISSED_KEY, version);
  } catch {
    // Without storage it simply asks again next time.
  }
}

/**
 * Offers a newer version of the app. "Later" hides it until a newer one
 * still comes out. Failed checks stay quiet: being offline isn't news.
 */
export function UpdateBanner() {
  const api = useMailApi();
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  useEffect(() => {
    let active = true;
    const check = () => {
      api.checkForUpdate().then(
        (found) => {
          if (active) setUpdate(found && found.version !== dismissedVersion() ? found : null);
        },
        () => undefined,
      );
    };
    const first = window.setTimeout(check, FIRST_CHECK_MS);
    const every = window.setInterval(check, CHECK_EVERY_MS);
    return () => {
      active = false;
      window.clearTimeout(first);
      window.clearInterval(every);
    };
  }, [api]);

  if (!update) return null;

  const start = () => {
    setError(null);
    if (update.how === 'download') {
      api.openExternal(update.page).catch((e: unknown) => {
        setError(toAppError(e));
      });
      return;
    }
    setBusy(true);
    // On success the app closes (Windows) or restarts (Linux).
    api.installUpdate().catch((e: unknown) => {
      setError(toAppError(e));
      setBusy(false);
    });
  };

  return (
    <div className="update-bar" role="status">
      <Icon name="download" />
      <div className="update-bar__text">
        <p className="update-bar__title">Mocky Mail {update.version} is available</p>
        {update.notes && <p className="update-bar__notes">{update.notes}</p>}
        {busy && <p className="update-bar__notes">Downloading and checking the update…</p>}
        {error && (
          <p className="update-bar__error" role="alert">
            {error.message}
          </p>
        )}
      </div>
      <div className="update-bar__actions">
        <button type="button" className="btn btn--primary btn--small" disabled={busy} onClick={start}>
          {update.how === 'install' ? 'Update now' : 'Download'}
        </button>
        <button
          type="button"
          className="btn btn--ghost btn--small"
          disabled={busy}
          onClick={() => {
            dismiss(update.version);
            setUpdate(null);
          }}
        >
          Later
        </button>
      </div>
    </div>
  );
}
