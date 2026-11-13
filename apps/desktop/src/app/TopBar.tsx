import { useState } from 'react';

import type { ConnectionStatus } from '../api/types';
import type { Theme } from '../theme/theme';
import { Icon, Logo } from '../ui/Icon';
import { connectionCopy } from './connection';

interface Props {
  query: string;
  connection: ConnectionStatus | null;
  theme: Theme;
  onQuery: (query: string) => void;
  onSync: () => Promise<void>;
  onToggleTheme: () => void;
}

export function TopBar({ query, connection, theme, onQuery, onSync, onToggleTheme }: Props) {
  const [syncing, setSyncing] = useState(false);
  const copy = connection ? connectionCopy(connection) : null;
  const themeLabel = theme === 'dark' ? 'Switch to light mode' : 'Switch to dark mode';

  const sync = () => {
    setSyncing(true);
    void onSync().finally(() => {
      setSyncing(false);
    });
  };

  return (
    <header className="topbar">
      <div className="brand">
        <Logo />
        <span className="brand__name">Mocky Mail</span>
      </div>
      <div className="search">
        <Icon name="search" />
        <label htmlFor="search" className="visually-hidden">
          Search mail
        </label>
        <input
          id="search"
          type="search"
          className="search__input"
          placeholder="Search mail, people, attachments"
          value={query}
          onChange={(e) => {
            onQuery(e.target.value);
          }}
        />
      </div>
      <div className="topbar__actions">
        {copy && copy.tone !== 'ok' && (
          <span className={`conn conn--${copy.tone}`} role="status">
            <Icon name="signal" size={16} />
            {copy.pill}
          </span>
        )}
        <button
          type="button"
          className="icon-btn"
          aria-label={syncing ? 'Syncing' : 'Sync now'}
          aria-busy={syncing}
          disabled={syncing}
          onClick={sync}
        >
          <Icon name="sync" />
        </button>
        <button type="button" className="icon-btn" aria-label={themeLabel} title={themeLabel} onClick={onToggleTheme}>
          <Icon name={theme === 'dark' ? 'sun' : 'moon'} />
        </button>
      </div>
    </header>
  );
}
