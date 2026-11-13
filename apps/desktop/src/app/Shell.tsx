import { useCallback, useState } from 'react';

import { useMailApi } from '../api/MailApiContext';
import type { FolderId } from '../api/types';
import { ComposeView } from '../features/compose/ComposeView';
import { blankSeed, type ComposeSeed } from '../features/compose/seed';
import { MessageList } from '../features/mailbox/MessageList';
import { filterMessages, isFiltered, NO_FILTERS, resolveSelection, type ListFilters } from '../features/mailbox/selectors';
import { Sidebar } from '../features/mailbox/Sidebar';
import { ReadingPane } from '../features/reader/ReadingPane';
import { useLoad } from '../lib/useLoad';
import { useTheme } from '../theme/useTheme';
import { StatusBar } from './StatusBar';
import { TopBar } from './TopBar';

export function Shell({ now = () => new Date() }: { now?: () => Date }) {
  const api = useMailApi();
  const { theme, toggle } = useTheme();
  const [folder, setFolder] = useState<FolderId>('inbox');
  const [filters, setFilters] = useState<ListFilters>(NO_FILTERS);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  const [compose, setCompose] = useState<ComposeSeed | null>(null);

  const refresh = useCallback(() => {
    setRevision((r) => r + 1);
  }, []);

  const accounts = useLoad('accounts', 0, () => api.listAccounts());
  const labels = useLoad('labels', 0, () => api.listLabels());
  const counts = useLoad('counts', revision, () => api.getCounts());
  const connection = useLoad('connection', revision, () => api.getConnection());
  const list = useLoad(`folder:${folder}`, revision, () => api.listFolder(folder));

  const accountList = accounts.status === 'ready' ? accounts.value : [];
  const visible = list.status === 'ready' ? filterMessages(list.value, filters, selectedId) : [];
  const activeId = resolveSelection(visible, selectedId);
  const accountLabel = accountList.find((a) => a.id === filters.accountId)?.label ?? null;
  const connectionValue = connection.status === 'ready' ? connection.value : null;
  const countsValue = counts.status === 'ready' ? counts.value : null;

  const toggleFilter = (key: 'accountId' | 'labelId', id: string) => {
    setFilters((f) => ({ ...f, [key]: f[key] === id ? null : id }));
    setSelectedId(null);
  };

  return (
    <div className="app">
      <TopBar
        query={filters.query}
        connection={connectionValue}
        theme={theme}
        onQuery={(query) => {
          setFilters((f) => ({ ...f, query }));
        }}
        onSync={() => api.syncNow().then(refresh)}
        onToggleTheme={toggle}
      />
      <div className="workspace">
        <Sidebar
          folder={folder}
          accountId={filters.accountId}
          labelId={filters.labelId}
          accounts={accountList}
          labels={labels.status === 'ready' ? labels.value : []}
          counts={countsValue}
          onFolder={(next) => {
            setFolder(next);
            setFilters((f) => ({ ...f, read: 'all' }));
            setSelectedId(null);
            setCompose(null);
          }}
          onAccount={(id) => {
            toggleFilter('accountId', id);
          }}
          onLabel={(id) => {
            toggleFilter('labelId', id);
          }}
          onCompose={() => {
            setCompose(blankSeed(filters.accountId ?? accountList[0]?.id ?? ''));
          }}
        />
        <main className="panes">
          {compose ? (
            <ComposeView
              seed={compose}
              accounts={accountList}
              offlineNote={connectionValue !== null && connectionValue.state !== 'online'}
              onClose={() => {
                setCompose(null);
              }}
              onQueued={() => {
                setCompose(null);
                refresh();
              }}
            />
          ) : (
            <>
              <MessageList
                folder={folder}
                loaded={list}
                visible={visible}
                filtered={isFiltered(filters)}
                selectedId={activeId}
                read={filters.read}
                accounts={accountList}
                accountLabel={accountLabel}
                now={now()}
                onSelect={setSelectedId}
                onRead={(read) => {
                  setFilters((f) => ({ ...f, read }));
                  setSelectedId(null);
                }}
                onRetry={refresh}
              />
              <ReadingPane
                messageId={activeId}
                markRead={activeId !== null && activeId === selectedId}
                revision={revision}
                accounts={accountList}
                onChanged={refresh}
                onCompose={setCompose}
              />
            </>
          )}
        </main>
      </div>
      <StatusBar connection={connectionValue} counts={countsValue} accountCount={accountList.length} />
    </div>
  );
}
