import { useState, type Ref } from 'react';

import type { Theme } from '../theme/useTheme';
import { Icon, Logo } from '../ui/Icon';
import type { SyncSummary } from './sync';

interface Props {
  search: string;
  /** Lets the "/" shortcut focus the search box. */
  searchField: Ref<HTMLInputElement>;
  sync: SyncSummary;
  theme: Theme;
  onSearch: (value: string) => void;
  onSync: () => Promise<void>;
  onToggleTheme: () => void;
  onHelp: () => void;
  /** Phones: opens the folder menu. Desktop has the sidebar instead. */
  onMenu?: (() => void) | undefined;
}

export function TopBar({ searchField, ...props }: Props) {
  const [syncing, setSyncing] = useState(false);
  const themeLabel = props.theme === 'dark' ? 'Switch to light mode' : 'Switch to dark mode';

  const sync = () => {
    setSyncing(true);
    props
      .onSync()
      .catch(() => undefined)
      .finally(() => {
        setSyncing(false);
      });
  };

  return (
    <header className="topbar">
      {props.onMenu && (
        <button type="button" className="icon-btn topbar__menu" aria-label="Folders and accounts" onClick={props.onMenu}>
          <Icon name="menu" />
        </button>
      )}
      <div className="brand">
        <Logo />
        <span className="brand__name">Mocky Mail</span>
      </div>
      <div className="search" role="search">
        <Icon name="search" />
        <label htmlFor="search" className="visually-hidden">
          Search mail
        </label>
        <input
          id="search"
          ref={searchField}
          type="search"
          className="search__input"
          // Phones (which get the menu button) have room for two words only.
          placeholder={props.onMenu ? 'Search mail' : 'Search mail, people, attachments'}
          value={props.search}
          onChange={(e) => {
            props.onSearch(e.target.value);
          }}
        />
        <kbd className="search__key" aria-hidden="true">
          /
        </kbd>
      </div>
      <div className="topbar__actions">
        {props.sync.pill && (
          <span className={`conn conn--${props.sync.tone}`} role="status" title={props.sync.status}>
            <Icon name="signal" size={16} />
            {props.sync.pill}
          </span>
        )}
        <button
          type="button"
          className="icon-btn"
          aria-label={syncing ? 'Syncing' : 'Check for new mail'}
          aria-busy={syncing}
          disabled={syncing}
          onClick={sync}
        >
          <Icon name="sync" />
        </button>
        <button type="button" className="icon-btn" aria-label={themeLabel} title={themeLabel} onClick={props.onToggleTheme}>
          <Icon name={props.theme === 'dark' ? 'sun' : 'moon'} />
        </button>
        <button type="button" className="icon-btn topbar__help" aria-label="Keyboard shortcuts" title="Keyboard shortcuts (?)" onClick={props.onHelp}>
          <Icon name="keyboard" />
        </button>
      </div>
    </header>
  );
}
