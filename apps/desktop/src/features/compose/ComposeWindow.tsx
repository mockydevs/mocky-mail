import { useEffect, useState } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { AppError, Draft, MailApi, Settings } from '../../api/types';
import { useLoad } from '../../lib/useLoad';
import { useTheme } from '../../theme/useTheme';
import { useSendStatus } from '../outbox/useSendStatus';
import { ComposeView } from './ComposeView';

/** The key a compose window was opened with (`?compose=<key>`), or null. */
export function composeWindowKey(search: string): string | null {
  const key = new URLSearchParams(search).get('compose');
  return key && key.trim() !== '' ? key : null;
}

/**
 * A window's draft can be taken once. React may run the loading effect
 * twice (StrictMode), so both runs share the one answer.
 */
const taken = new WeakMap<MailApi, Map<string, Promise<Draft | null>>>();
function takeOnce(api: MailApi, key: string): Promise<Draft | null> {
  const byKey = taken.get(api) ?? new Map<string, Promise<Draft | null>>();
  taken.set(api, byKey);
  let draft = byKey.get(key);
  if (!draft) {
    draft = api.takeComposeDraft(key);
    byKey.set(key, draft);
  }
  return draft;
}

function closeWindow() {
  window.close();
}

/**
 * Compose in a window of its own (computers, Pop out): only the message,
 * filling the window. Sending or closing closes the window; while a sent
 * message can still be undone, the window says so and offers Undo.
 */
export function ComposeWindow({ composeKey }: { composeKey: string }) {
  const api = useMailApi();
  const load = useLoad(`compose-window:${composeKey}`, 0, () => takeOnce(api, composeKey));
  const accountsLoad = useLoad('accounts', 0, () => api.listAccounts());
  const accounts = accountsLoad.status === 'ready' ? accountsLoad.value : [];
  const [settings, setSettings] = useState<Settings | null>(null);
  useTheme(settings?.theme ?? 'system');
  /** After Undo: the message back out of the Outbox, held only here. */
  const [again, setAgain] = useState<Draft | null>(null);
  const [sent, setSent] = useState(false);
  /** Undo pressed: the message is on its way back, so the window stays. */
  const [undoing, setUndoing] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const sending = useSendStatus(api);
  const phase = sending.status?.phase;

  useEffect(() => {
    api.getSettings().then(setSettings, () => undefined);
  }, [api]);

  const draft = again ?? (load.status === 'ready' ? load.value : null);
  useEffect(() => {
    document.title = draft ? draft.subject || 'New message' : 'New message';
  }, [draft]);

  // Once it can no longer be undone (or it is on its way), the window's work is done.
  useEffect(() => {
    if (sent && !undoing && phase !== 'scheduled' && phase !== 'failed') closeWindow();
  }, [sent, undoing, phase]);

  if (load.status === 'loading' || accountsLoad.status === 'loading') {
    return (
      <div className="splash" aria-busy="true">
        Opening the message
      </div>
    );
  }
  if (!draft) {
    return (
      <div className="splash" role="alert">
        <p className="empty__title">This message is not here any more</p>
        <p className="empty__text">
          {load.status === 'error' ? load.message : 'It may be open in another window. Saved drafts are in Drafts.'}
        </p>
        <button type="button" className="btn btn--primary" onClick={closeWindow}>
          Close window
        </button>
      </div>
    );
  }

  if (sent) {
    const status = sending.status;
    return (
      <div className="splash" role="status">
        {status?.phase === 'failed' ? (
          <>
            <p className="empty__title">Not sent</p>
            <p className="empty__text">{status.message} The message is in the Outbox.</p>
          </>
        ) : (
          <>
            <p className="empty__title">Sending…</p>
            {status?.phase === 'scheduled' && (
              <p className="empty__text">You have {status.secs} seconds to undo.</p>
            )}
          </>
        )}
        {error && <p className="form-error">{error.message}</p>}
        <div className="splash__actions">
          {status?.phase === 'scheduled' && (
            <button
              type="button"
              className="btn btn--secondary"
              onClick={() => {
                const id = status.id;
                setUndoing(true);
                sending.clear();
                api.outboxEdit(id).then(
                  (back) => {
                    setAgain(back);
                    setSent(false);
                    setUndoing(false);
                  },
                  (e: unknown) => {
                    setUndoing(false);
                    setError(toAppError(e));
                  },
                );
              }}
            >
              Undo
            </button>
          )}
          <button type="button" className="btn btn--primary" onClick={closeWindow}>
            Close window
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="app app--window">
      <main className="panes panes--window">
        <ComposeView
          key={again ? 'again' : 'first'}
          initial={draft}
          // It already has its signature from where it was started.
          kind="draft"
          // It exists only here: closing asks first.
          unsaved
          title={draft.replyToMessageId ? 'Reply' : 'New message'}
          accounts={accounts}
          offlineNote={settings?.workOffline ?? false}
          standalone
          onClose={closeWindow}
          onSent={(entry) => {
            setError(null);
            sending.track(entry);
            setSent(true);
          }}
        />
      </main>
    </div>
  );
}
