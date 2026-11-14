import type { SyncSummary } from './sync';

export function StatusBar({ sync, outbox, accountCount }: { sync: SyncSummary; outbox: number; accountCount: number }) {
  return (
    <footer className="statusbar">
      <span className="statusbar__item">
        <span className={`dot dot--small dot--conn-${sync.tone}`} aria-hidden="true" />
        {sync.status}
      </span>
      {sync.lastSync && <span className="statusbar__item">{sync.lastSync}</span>}
      <span className="statusbar__spacer" />
      {outbox > 0 && <span className="statusbar__item">{outbox} waiting in Outbox</span>}
      <span className="statusbar__item">
        {accountCount} {accountCount === 1 ? 'account' : 'accounts'}
      </span>
    </footer>
  );
}
