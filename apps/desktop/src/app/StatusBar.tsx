import type { SyncSummary } from './sync';

interface Props {
  sync: SyncSummary;
  outbox: number;
  accountCount: number;
  /** Work Offline is on. */
  workOffline?: boolean;
  onWorkOffline?: (on: boolean) => void;
}

export function StatusBar({ sync, outbox, accountCount, workOffline = false, onWorkOffline }: Props) {
  return (
    <footer className="statusbar">
      <span className="statusbar__item">
        <span className={`dot dot--small dot--conn-${sync.tone}`} aria-hidden="true" />
        {sync.status}
      </span>
      {sync.lastSync && <span className="statusbar__item">{sync.lastSync}</span>}
      <span className="statusbar__spacer" />
      {outbox > 0 && <span className="statusbar__item">{outbox} waiting in Outbox</span>}
      {onWorkOffline && (
        <button
          type="button"
          className={`statusbar__toggle${workOffline ? ' statusbar__toggle--on' : ''}`}
          aria-pressed={workOffline}
          title={workOffline ? 'Back online: check for mail and send again' : 'Stop checking for mail and sending until you switch this off'}
          onClick={() => {
            onWorkOffline(!workOffline);
          }}
        >
          Work offline
        </button>
      )}
      <span className="statusbar__item">
        {accountCount} {accountCount === 1 ? 'account' : 'accounts'}
      </span>
    </footer>
  );
}
