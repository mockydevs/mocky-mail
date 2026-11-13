import type { ConnectionStatus, FolderCounts } from '../api/types';
import { connectionCopy } from './connection';

export function StatusBar({
  connection,
  counts,
  accountCount,
}: {
  connection: ConnectionStatus | null;
  counts: FolderCounts | null;
  accountCount: number;
}) {
  const copy = connection ? connectionCopy(connection) : null;
  return (
    <footer className="statusbar">
      {copy && (
        <span className="statusbar__item">
          <span className={`dot dot--small dot--conn-${copy.tone}`} aria-hidden="true" />
          {copy.status}
        </span>
      )}
      {copy?.lastSync && <span className="statusbar__item">{copy.lastSync}</span>}
      <span className="statusbar__spacer" />
      {counts && counts.outbox > 0 && <span className="statusbar__item">{counts.outbox} waiting in Outbox</span>}
      <span className="statusbar__item">
        {accountCount} {accountCount === 1 ? 'account' : 'accounts'}
      </span>
    </footer>
  );
}
