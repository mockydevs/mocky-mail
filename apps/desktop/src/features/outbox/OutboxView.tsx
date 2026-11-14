import { useState } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { Account, AppError, Draft, OutboxEntry } from '../../api/types';
import { formatListTime } from '../../lib/format';
import { Avatar } from '../../ui/Avatar';
import { Icon } from '../../ui/Icon';
import { formatMailboxes } from '../compose/seed';
import { emptyCopy } from '../mailbox/selectors';
import { isActive, outboxCopy } from './outboxCopy';

interface Props {
  entries: readonly OutboxEntry[] | null;
  accounts: readonly Account[];
  now: number;
  onEdit: (draft: Draft) => void;
}

export function OutboxView({ entries, accounts, now, onEdit }: Props) {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const active = (entries ?? []).filter(isActive).sort((a, b) => b.createdAt - a.createdAt);
  const selected = active.find((e) => e.id === selectedId) ?? active[0] ?? null;
  const empty = emptyCopy('outbox', false);
  const labelOf = (id: string) => accounts.find((a) => a.id === id)?.displayName ?? 'this';

  return (
    <>
      <section className="list" aria-labelledby="list-title">
        <div className="list__head">
          <div className="list__heading">
            <h1 id="list-title" className="list__title">
              Outbox
            </h1>
            <span className="muted small">
              {active.length} {active.length === 1 ? 'message' : 'messages'} waiting
            </span>
          </div>
        </div>
        <p className="list__note">
          <Icon name="clock" />
          Messages here are saved on this computer and send automatically when the connection is good.
        </p>
        <div className="list__scroll">
          {entries === null && (
            <div aria-busy="true" aria-label="Loading messages" className="row row--skeleton">
              <div className="skeleton skeleton--avatar" />
            </div>
          )}
          {entries !== null && active.length === 0 && (
            <div className="empty">
              <Icon name="outbox" size={40} />
              <p className="empty__title">{empty.title}</p>
              <p className="empty__text">{empty.text}</p>
            </div>
          )}
          {active.length > 0 && (
            <ul className="rows" aria-label="Outbox messages">
              {active.map((entry) => {
                const copy = outboxCopy(entry, labelOf(entry.accountId), now);
                const first = entry.to[0];
                return (
                  <li key={entry.id}>
                    <button
                      type="button"
                      className="row"
                      aria-current={entry.id === selected?.id ? 'true' : undefined}
                      onClick={() => {
                        setSelectedId(entry.id);
                      }}
                    >
                      <Avatar name={first?.name ?? first?.address ?? '?'} address={first?.address ?? ''} />
                      <span className="row__text">
                        <span className="row__top">
                          <span className="row__who">To {entry.to.map((t) => t.name ?? t.address).join(', ')}</span>
                          <span className="row__time">{formatListTime(entry.createdAt, now)}</span>
                        </span>
                        <span className="row__subject">{entry.subject || '(no subject)'}</span>
                        <span className="row__preview">{entry.preview}</span>
                        <span className={`pill pill--${copy.tone}`}>{copy.pill}</span>
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      </section>
      {selected ? (
        <OutboxDetail
          key={selected.id}
          entry={selected}
          accountLabel={labelOf(selected.accountId)}
          now={now}
          onEdit={onEdit}
        />
      ) : (
        <article className="reader reader--empty" aria-label="Reading pane">
          <Icon name="mail" size={48} />
          <p className="empty__title">No message selected</p>
        </article>
      )}
    </>
  );
}

function OutboxDetail({
  entry,
  accountLabel,
  now,
  onEdit,
}: {
  entry: OutboxEntry;
  accountLabel: string;
  now: number;
  onEdit: (draft: Draft) => void;
}) {
  const api = useMailApi();
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  const copy = outboxCopy(entry, accountLabel, now);

  const run = (work: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    work()
      .catch((e: unknown) => {
        setError(toAppError(e));
      })
      .finally(() => {
        setBusy(false);
      });
  };

  return (
    <article className="reader" aria-label="Reading pane">
      <div className="reader__scroll">
        <section className={`banner banner--${copy.tone}`} role="status" aria-label="Delivery status">
          <div className="banner__main">
            <Icon name={copy.tone === 'warn' ? 'clock' : 'alert'} size={20} />
            <div className="banner__copy">
              <p className="banner__title">{copy.title}</p>
              <p className="banner__text">{copy.text}</p>
              {copy.detail && <p className="banner__detail">{copy.detail}</p>}
              {copy.progress !== null && (
                <progress className="banner__progress" max={100} value={copy.progress} aria-label="Upload progress" />
              )}
            </div>
          </div>
          {(copy.canSendNow || copy.canEdit || copy.canCancel) && (
            <div className="banner__actions">
              {copy.canSendNow && (
                <button
                  type="button"
                  className="btn btn--primary btn--small"
                  disabled={busy}
                  onClick={() => {
                    run(() => api.outboxSendNow(entry.id));
                  }}
                >
                  Send now
                </button>
              )}
              {copy.canEdit && (
                <button
                  type="button"
                  className="btn btn--secondary btn--small"
                  disabled={busy}
                  onClick={() => {
                    run(() => api.outboxEdit(entry.id).then(onEdit));
                  }}
                >
                  Edit message
                </button>
              )}
              {copy.canCancel && (
                <button
                  type="button"
                  className="btn btn--link btn--small"
                  disabled={busy}
                  onClick={() => {
                    run(() => api.outboxCancel(entry.id));
                  }}
                >
                  {entry.status.state === 'failed' ? 'Delete message' : 'Cancel sending'}
                </button>
              )}
            </div>
          )}
        </section>
        {error && (
          <p className="form-error" role="alert">
            {error.message}
          </p>
        )}
        <h2 className="reader__subject">{entry.subject || '(no subject)'}</h2>
        <p className="reader__meta">To {formatMailboxes(entry.to)}</p>
        <p className="reader__meta">
          Queued {formatListTime(entry.createdAt, now)}, {entry.attempts} {entry.attempts === 1 ? 'attempt' : 'attempts'}
        </p>
        <div className="reader__body">
          <div className="reader__text">{entry.preview}</div>
        </div>
      </div>
    </article>
  );
}
