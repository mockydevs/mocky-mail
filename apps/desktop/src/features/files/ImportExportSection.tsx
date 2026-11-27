import { useId, useState } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import type { Account, Folder } from '../../api/types';
import { useLoad } from '../../lib/useLoad';
import { FolderPicker } from '../mailbox/FolderPicker';
import { accountLabel, AddressBookOptions, bookFromValue, bookValue } from './AddressBookPicker';
import { useFileTasks } from './FileTasks';
import {
  exportCalendarTask,
  exportContactsTask,
  exportFolderTask,
  importCalendarTask,
  importContactsTask,
  importMailTask,
  importPstTask,
} from './tasks';

/**
 * Settings › Import and export: mail (.eml, .mbox), calendars (.ics),
 * contacts (.vcf) and an Outlook data file (.pst). Each button opens the
 * system's file picker; progress and the result show in a dialog.
 */
export function ImportExportSection({
  accounts,
  folders,
}: {
  accounts: readonly Account[];
  folders: Readonly<Record<string, Folder[]>>;
}) {
  const id = useId();
  const api = useMailApi();
  const files = useFileTasks();
  const calendarsLoad = useLoad('import-calendars', 0, () => api.listCalendars());
  const calendars = (calendarsLoad.status === 'ready' ? calendarsLoad.value : []).filter((c) =>
    accounts.some((a) => a.id === c.accountId),
  );
  const [picking, setPicking] = useState<'import' | 'export' | null>(null);
  const [calendarId, setCalendarId] = useState<string | null>(null);
  const calendar = calendars.find((c) => c.id === calendarId) ?? calendars[0];
  const [book, setBook] = useState(bookValue(accounts[0]?.id ?? null));
  const [pstAccount, setPstAccount] = useState(accounts[0]?.id ?? '');
  const pst = accounts.find((a) => a.id === pstAccount) ?? accounts[0];
  const allFolders = Object.values(folders).flat();
  const calendarLabel = (calendarAccount: string, name: string) => {
    const email = accounts.find((a) => a.id === calendarAccount)?.email ?? '';
    return `${name} (${email})`;
  };

  return (
    <section className="settings__group files-section" aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`} className="settings__legend">
        Import and export
      </h2>

      <h3 className="files-section__head">Mail</h3>
      <p className="form-hint">Bring in .eml and .mbox files, or save a folder as one .mbox file.</p>
      <div className="files-section__row">
        <button
          type="button"
          className="btn btn--secondary btn--small"
          onClick={() => {
            setPicking('import');
          }}
        >
          Import messages…
        </button>
        <button
          type="button"
          className="btn btn--secondary btn--small"
          onClick={() => {
            setPicking('export');
          }}
        >
          Export a folder as .mbox…
        </button>
      </div>

      <h3 className="files-section__head">Calendar</h3>
      {calendar ? (
        <>
          <label className="form-field">
            <span className="form-field__label">Calendar</span>
            <select
              className="field"
              value={calendar.id}
              onChange={(e) => {
                setCalendarId(e.target.value);
              }}
            >
              {calendars.map((c) => (
                <option key={c.id} value={c.id}>
                  {calendarLabel(c.accountId, c.name)}
                </option>
              ))}
            </select>
          </label>
          <div className="files-section__row">
            <button
              type="button"
              className="btn btn--secondary btn--small"
              disabled={!calendar.writable}
              title={calendar.writable ? undefined : `${calendar.name} is read-only.`}
              onClick={() => {
                files.run(importCalendarTask(api, calendar));
              }}
            >
              Import .ics…
            </button>
            <button
              type="button"
              className="btn btn--secondary btn--small"
              onClick={() => {
                files.run(exportCalendarTask(api, calendar));
              }}
            >
              Export calendar…
            </button>
          </div>
        </>
      ) : (
        <p className="form-hint">
          {calendarsLoad.status === 'loading' ? 'Looking for calendars…' : 'None of your accounts has a calendar yet.'}
        </p>
      )}

      <h3 className="files-section__head">Contacts</h3>
      <label className="form-field">
        <span className="form-field__label">Address book</span>
        <select
          className="field"
          value={book}
          onChange={(e) => {
            setBook(e.target.value);
          }}
        >
          <AddressBookOptions accounts={accounts} />
        </select>
      </label>
      <div className="files-section__row">
        <button
          type="button"
          className="btn btn--secondary btn--small"
          onClick={() => {
            files.run(importContactsTask(api, accounts, bookFromValue(book)));
          }}
        >
          Import contacts (.vcf)…
        </button>
        <button
          type="button"
          className="btn btn--secondary btn--small"
          onClick={() => {
            files.run(exportContactsTask(api, accounts, bookFromValue(book)));
          }}
        >
          Export these contacts…
        </button>
        <button
          type="button"
          className="btn btn--link btn--small"
          onClick={() => {
            files.run(exportContactsTask(api, accounts, '*'));
          }}
        >
          Export all contacts…
        </button>
      </div>

      <h3 className="files-section__head">Outlook data file (.pst)</h3>
      {pst && (
        <>
          <label className="form-field">
            <span className="form-field__label">Import into account</span>
            <select
              className="field"
              value={pst.id}
              onChange={(e) => {
                setPstAccount(e.target.value);
              }}
            >
              {accounts.map((a) => (
                <option key={a.id} value={a.id}>
                  {accountLabel(a)}
                </option>
              ))}
            </select>
          </label>
          <p className="form-hint" id={`${id}-pst`}>
            Mail goes to a new folder, Imported from Outlook, on that account; contacts and calendar go to the
            account&apos;s address book and calendar. A big file can take a long time, and you can keep using Mocky Mail
            meanwhile.
          </p>
          <div className="files-section__row">
            <button
              type="button"
              className="btn btn--primary btn--small"
              aria-describedby={`${id}-pst`}
              disabled={files.busy}
              onClick={() => {
                files.run(importPstTask(api, pst));
              }}
            >
              Start import…
            </button>
          </div>
        </>
      )}

      {picking && (
        <FolderPicker
          mode="pick"
          accountIds={accounts.map((a) => a.id)}
          accounts={accounts}
          folders={folders}
          title={picking === 'import' ? 'Import messages into…' : 'Export which folder?'}
          description={
            picking === 'import'
              ? 'Choose the folder the messages go to. You pick the .eml or .mbox files next.'
              : 'The folder is saved as one .mbox file. You pick where it goes next.'
          }
          onClose={() => {
            setPicking(null);
          }}
          onPick={(target) => {
            const mode = picking;
            setPicking(null);
            const folder = 'folderId' in target ? allFolders.find((f) => f.id === target.folderId) : undefined;
            if (!folder) return;
            files.run(mode === 'import' ? importMailTask(api, folder) : exportFolderTask(api, folder));
          }}
        />
      )}
    </section>
  );
}
