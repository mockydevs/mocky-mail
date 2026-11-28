import { useEffect, useState } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { Account, AddressBookStatus, CalendarStatus } from '../../api/types';
import { Icon } from '../../ui/Icon';
import { AdminConsentPanel } from '../accounts/AdminConsent';

type Phase = { kind: 'idle' } | { kind: 'busy' } | { kind: 'done' } | { kind: 'failed'; message: string; adminConsentUrl?: string | undefined };

/** The name a provider goes by in the card. */
function providerName(account: Account): string {
  return account.auth === 'google' ? 'Google' : 'Microsoft';
}

/**
 * Gmail and Microsoft accounts added before calendars and contacts were
 * asked for: one button signs in again (in the browser) for calendar,
 * contacts, tasks and automatic replies. A failure keeps the card, with why.
 */
export function ConnectCard({ account, onConnected }: { account: Account; onConnected?: () => void }) {
  const api = useMailApi();
  const [phase, setPhase] = useState<Phase>({ kind: 'idle' });
  const name = providerName(account);
  const connect = () => {
    setPhase({ kind: 'busy' });
    api.connectCalendarAndContacts(account.id).then(
      () => {
        setPhase({ kind: 'done' });
        onConnected?.();
      },
      (e: unknown) => {
        const failure = toAppError(e);
        setPhase({ kind: 'failed', message: failure.message, adminConsentUrl: failure.adminConsentUrl });
      },
    );
  };
  return (
    <section className={`connect-card${phase.kind === 'done' ? ' connect-card--done' : ''}`} aria-label={`Connect ${account.email}`}>
      <Icon name={phase.kind === 'done' ? 'check' : 'calendar'} />
      <div className="connect-card__text">
        <h3 className="connect-card__title">
          {phase.kind === 'done' ? `${name} calendar, contacts and tasks connected` : `Connect your ${name} calendar, contacts and tasks`}
        </h3>
        <p className="muted small">
          {phase.kind === 'done'
            ? `${account.email} is syncing them now.`
            : phase.kind === 'busy'
              ? `Finish signing in to ${account.email} in your browser…`
              : `${account.email} only shares its mail with Mocky Mail so far. Sign in once more to add the rest.`}
        </p>
        {phase.kind === 'failed' && phase.adminConsentUrl && (
          <AdminConsentPanel url={phase.adminConsentUrl} onRetry={connect} />
        )}
        {phase.kind === 'failed' && !phase.adminConsentUrl && (
          <p className="form-error" role="alert">
            {phase.message}
          </p>
        )}
      </div>
      {phase.kind !== 'done' && !(phase.kind === 'failed' && phase.adminConsentUrl) && (
        <button type="button" className="btn btn--primary btn--small" disabled={phase.kind === 'busy'} onClick={connect}>
          {phase.kind === 'busy' ? 'Connecting…' : phase.kind === 'failed' ? 'Try again' : 'Connect'}
        </button>
      )}
    </section>
  );
}

/** A card for each account the statuses say can be connected. */
export function ConnectCards({
  accounts,
  statuses,
  onConnected,
}: {
  accounts: readonly Account[];
  statuses: readonly (CalendarStatus | AddressBookStatus)[];
  onConnected?: () => void;
}) {
  // Keep a card on screen after its account connects (to say so), until the screen closes.
  const [shown, setShown] = useState<string[]>([]);
  const connectable = statuses.filter((s) => s.connectable).map((s) => s.accountId);
  const ids = [...new Set([...shown, ...connectable])];
  if (connectable.some((a) => !shown.includes(a))) setShown(ids);
  const list = accounts.filter((a) => ids.includes(a.id) && (a.auth === 'google' || a.auth === 'microsoft'));
  if (list.length === 0) return null;
  return (
    <div className="connect-cards">
      {list.map((a) => (
        <ConnectCard key={a.id} account={a} {...(onConnected ? { onConnected } : {})} />
      ))}
    </div>
  );
}

/**
 * Which accounts can be connected, for screens that don't sync calendars
 * themselves (Tasks, automatic replies): asks once, then follows the
 * calendar and contacts events.
 */
export function useConnectable(): (CalendarStatus | AddressBookStatus)[] {
  const api = useMailApi();
  const [statuses, setStatuses] = useState<(CalendarStatus | AddressBookStatus)[]>([]);
  useEffect(() => {
    let live = true;
    const offCal = api.on('mail://calendar-changed', ({ calendars }) => {
      if (calendars.length > 0) setStatuses(calendars);
    });
    const offBooks = api.on('mail://contacts-changed', ({ books }) => {
      if (books.length > 0) setStatuses((prev) => merge(prev, books));
    });
    api.syncCalendars().then(
      (list) => {
        if (live) setStatuses((prev) => merge(prev, list));
      },
      () => undefined,
    );
    return () => {
      live = false;
      offCal();
      offBooks();
    };
  }, [api]);
  return statuses;
}

/** One status per account: connectable if either says so. */
function merge(
  a: readonly (CalendarStatus | AddressBookStatus)[],
  b: readonly (CalendarStatus | AddressBookStatus)[],
): (CalendarStatus | AddressBookStatus)[] {
  const out = new Map(a.map((s) => [s.accountId, s]));
  for (const s of b) {
    const old = out.get(s.accountId);
    out.set(s.accountId, old && old.connectable && s.connectable === undefined ? old : s);
  }
  return [...out.values()];
}
