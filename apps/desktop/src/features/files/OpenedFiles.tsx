import { useEffect, useState } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { Account, AppError, CalendarInfo, Draft, Folder, MessageDetail, OpenedFile } from '../../api/types';
import type { Notice } from '../../app/useToasts';
import { usePhoneBack } from '../../app/usePhone';
import { formatBytes, formatLongDate } from '../../lib/format';
import type { Theme } from '../../theme/useTheme';
import { Icon } from '../../ui/Icon';
import { Modal } from '../../ui/Modal';
import { forwardDraft, formatMailboxes, replyDraft, type ComposeKind } from '../compose/seed';
import { FolderPicker } from '../mailbox/FolderPicker';
import { folderLabel } from '../mailbox/selectors';
import { linkify } from '../reader/linkify';
import { SandboxedHtml } from '../reader/SandboxedHtml';
import { AddressBookOptions, bookFromValue, bookValue } from './AddressBookPicker';
import { useFileTasks } from './FileTasks';
import { addressBookName } from './tasks';
import { fileErrorCopy } from './fileErrors';

/** How the shell starts compose (see Shell's `compose`). */
export type StartCompose = (draft: Draft, title: string, kind: ComposeKind, attachmentsFrom: string | null) => void;

interface Props {
  accounts: readonly Account[];
  folders: Readonly<Record<string, Folder[]>>;
  theme: Theme;
  workOffline?: boolean;
  onCompose: StartCompose;
  onNotice: (notice: Notice) => void;
  onOpenFolder: (folderId: string) => void;
}

/**
 * Files the app was asked to open: double-clicked, "Open with", or shared to
 * it on Android. Those it started with come from takeOpenedFiles, later ones
 * as mail://open-file. They are shown one at a time: an email in a viewer,
 * a calendar or contacts file in a dialog that adds it.
 */
export function OpenedFiles({ accounts, folders, theme, workOffline = false, onCompose, onNotice, onOpenFolder }: Props) {
  const api = useMailApi();
  const [queue, setQueue] = useState<OpenedFile[]>([]);

  useEffect(() => {
    let live = true;
    const add = (files: OpenedFile[]) => {
      setQueue((q) => [...q, ...files.filter((f) => !q.some((x) => x.path === f.path))]);
    };
    api.takeOpenedFiles().then(
      (files) => {
        if (live) add(files);
      },
      () => undefined,
    );
    const off = api.on('mail://open-file', ({ file }) => {
      add([file]);
    });
    return () => {
      live = false;
      off();
    };
  }, [api]);

  const [file] = queue;
  if (!file || accounts.length === 0) return null;
  const done = () => {
    setQueue((q) => q.filter((f) => f.path !== file.path));
  };

  switch (file.kind) {
    case 'eml':
      return (
        <MessageFileViewer
          key={file.path}
          file={file}
          accounts={accounts}
          folders={folders}
          theme={theme}
          workOffline={workOffline}
          onClose={done}
          onCompose={(...args) => {
            done();
            onCompose(...args);
          }}
          onAdded={(folder) => {
            done();
            onNotice({
              tone: 'info',
              title: `Added to ${folderLabel(folder)}`,
              text: file.name,
              action: {
                label: 'Open folder',
                run: () => {
                  onOpenFolder(folder.id);
                },
              },
            });
          }}
        />
      );
    case 'ics':
      return <CalendarFileDialog key={file.path} file={file} accounts={accounts} onClose={done} />;
    case 'vcf':
      return <ContactsFileDialog key={file.path} file={file} accounts={accounts} onClose={done} />;
  }
}

/** The account a reply goes out from: the one the message was sent to, else the first. */
function accountFor(message: MessageDetail, accounts: readonly Account[]): Account | undefined {
  const to = [...message.to, ...message.cc].map((m) => m.address.toLowerCase());
  return accounts.find((a) => to.includes(a.email.toLowerCase())) ?? accounts[0];
}

type Loading = { status: 'loading' } | { status: 'ready'; message: MessageDetail } | { status: 'error'; error: AppError };

/** An opened .eml file: read it, reply or forward, add it to a folder, save its attachments. */
function MessageFileViewer({
  file,
  accounts,
  folders,
  theme,
  workOffline,
  onClose,
  onCompose,
  onAdded,
}: {
  file: OpenedFile;
  accounts: readonly Account[];
  folders: Readonly<Record<string, Folder[]>>;
  theme: Theme;
  workOffline: boolean;
  onClose: () => void;
  onCompose: StartCompose;
  onAdded: (folder: Folder) => void;
}) {
  const api = useMailApi();
  const [loaded, setLoaded] = useState<Loading>({ status: 'loading' });
  const [picking, setPicking] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  /** Saving an attachment straight from the file was refused: the buttons go. */
  const [noSaving, setNoSaving] = useState(false);
  usePhoneBack(1, () => {
    if (picking) setPicking(false);
    else onClose();
  });

  useEffect(() => {
    let live = true;
    api.readMessageFile(file.path).then(
      (message) => {
        if (live) setLoaded({ status: 'ready', message });
      },
      (e: unknown) => {
        if (live) setLoaded({ status: 'error', error: toAppError(e) });
      },
    );
    return () => {
      live = false;
    };
  }, [api, file.path]);

  if (loaded.status !== 'ready') {
    const copy = loaded.status === 'error' ? fileErrorCopy(loaded.error) : null;
    return (
      <Modal title={file.name} onClose={onClose} size="small">
        {copy ? (
          <div className="notice notice--error" role="alert">
            <p className="notice__title">This email couldn&apos;t be opened</p>
            <p>{copy.text}</p>
          </div>
        ) : (
          <p className="muted" role="status">
            Opening {file.name}…
          </p>
        )}
        <div className="modal__actions">
          <button type="button" className="btn btn--secondary" onClick={onClose}>
            Close
          </button>
        </div>
      </Modal>
    );
  }

  const { message } = loaded;
  const account = accountFor(message, accounts);
  const sender = message.from;

  const reply = (all: boolean) => {
    if (!account) return;
    const mine = { ...message, accountId: account.id };
    // The file isn't in a folder, so there is no stored original to mark answered.
    onCompose({ ...replyDraft(mine, account, all), replyToMessageId: null }, all ? 'Reply all' : 'Reply', 'reply', null);
  };
  const forward = () => {
    if (!account) return;
    const files = message.attachments.length > 0 && !noSaving ? message.id : null;
    onCompose(forwardDraft({ ...message, accountId: account.id }), 'Forward', 'forward', files);
  };
  const save = (index: number) => {
    setError(null);
    setSaved(null);
    api.saveAttachment(message.id, index).then(
      (path) => {
        if (path !== null) setSaved(`Saved: ${path}`);
      },
      (e: unknown) => {
        const failure = toAppError(e);
        // The backend only saves attachments of stored mail: say how instead.
        if (failure.kind === 'not_found' || failure.kind === 'invalid') setNoSaving(true);
        else setError(failure);
      },
    );
  };
  const addTo = (folderId: string) => {
    setPicking(false);
    const folder = Object.values(folders)
      .flat()
      .find((f) => f.id === folderId);
    if (!folder) return;
    setBusy(true);
    setError(null);
    api.importMessageFile(file.path, folderId).then(
      () => {
        setBusy(false);
        onAdded(folder);
      },
      (e: unknown) => {
        setBusy(false);
        setError(toAppError(e));
      },
    );
  };
  const copy = error ? fileErrorCopy(error, workOffline) : null;

  return (
    <>
      <Modal title={message.subject || '(no subject)'} onClose={onClose} size="large">
        <div className="file-message" aria-label={`Email file ${file.name}`} role="group">
          <p className="form-hint">From the file {file.name}. It isn&apos;t in any of your folders.</p>
          <div className="file-message__people">
            <p className="reader__name">
              {sender ? (
                <>
                  {sender.name ?? sender.address} {sender.name && <span className="muted">&lt;{sender.address}&gt;</span>}
                </>
              ) : (
                'Unknown sender'
              )}
            </p>
            <p className="reader__meta">To {formatMailboxes(message.to) || 'no recipients'}</p>
            {message.cc.length > 0 && <p className="reader__meta">Cc {formatMailboxes(message.cc)}</p>}
            <p className="reader__meta">{formatLongDate(message.date)}</p>
          </div>

          <div className="file-message__actions" role="toolbar" aria-label="Email file actions">
            <button
              type="button"
              className="btn btn--secondary btn--small"
              disabled={!account}
              onClick={() => {
                reply(false);
              }}
            >
              <Icon name="reply" />
              Reply
            </button>
            <button
              type="button"
              className="btn btn--secondary btn--small"
              disabled={!account}
              onClick={() => {
                reply(true);
              }}
            >
              <Icon name="replyAll" />
              Reply all
            </button>
            <button type="button" className="btn btn--secondary btn--small" disabled={!account} onClick={forward}>
              <Icon name="forward" />
              Forward
            </button>
            <button
              type="button"
              className="btn btn--primary btn--small"
              disabled={busy}
              aria-busy={busy}
              onClick={() => {
                setPicking(true);
              }}
            >
              <Icon name="folder" />
              Add to folder…
            </button>
          </div>

          {copy && (
            <div className="notice notice--error" role="alert">
              <p className="notice__title">{copy.title}</p>
              <p>{copy.text}</p>
            </div>
          )}
          {saved && (
            <p className="form-hint" role="status">
              {saved}
            </p>
          )}

          {message.attachments.length > 0 && (
            <>
              <ul className="attachments" aria-label="Attachments">
                {message.attachments.map((a) => (
                  <li key={a.index} className="attachment">
                    <span className="attachment__text">
                      <span className="attachment__name">{a.fileName}</span>
                      <span className="muted small">{formatBytes(a.size)}</span>
                    </span>
                    {!noSaving && (
                      <button
                        type="button"
                        className="icon-btn"
                        aria-label={`Save ${a.fileName}`}
                        onClick={() => {
                          save(a.index);
                        }}
                      >
                        <Icon name="download" />
                      </button>
                    )}
                  </li>
                ))}
              </ul>
              {noSaving && (
                <p className="form-hint" role="status">
                  To save the attachments, add the email to a folder first and open it there.
                </p>
              )}
            </>
          )}

          {message.html !== null ? (
            <div className="file-message__frame">
              <SandboxedHtml title={`Message: ${message.subject}`} html={message.html} theme={theme} />
            </div>
          ) : (
            <div className="reader__text file-message__text">
              {linkify(message.text ?? '').map((part, i) =>
                part.kind === 'text' ? (
                  <span key={i}>{part.text}</span>
                ) : (
                  <a
                    key={i}
                    href={part.url}
                    title={part.url}
                    onClick={(e) => {
                      e.preventDefault();
                      api.openExternal(part.url).catch((err: unknown) => {
                        setError(toAppError(err));
                      });
                    }}
                  >
                    {part.text}
                  </a>
                ),
              )}
            </div>
          )}
        </div>
        <div className="modal__actions">
          <button type="button" className="btn btn--secondary" onClick={onClose}>
            Close
          </button>
        </div>
      </Modal>
      {picking && (
        <FolderPicker
          mode="pick"
          accountIds={accounts.map((a) => a.id)}
          accounts={accounts}
          folders={folders}
          title="Add the email to…"
          description="A copy of the email goes into this folder, on the mail server too."
          onClose={() => {
            setPicking(false);
          }}
          onPick={(target) => {
            if ('folderId' in target) addTo(target.folderId);
          }}
        />
      )}
    </>
  );
}

/** An opened .ics file: add its events to a calendar. */
function CalendarFileDialog({ file, accounts, onClose }: { file: OpenedFile; accounts: readonly Account[]; onClose: () => void }) {
  const api = useMailApi();
  const files = useFileTasks();
  usePhoneBack(1, onClose);
  const [calendars, setCalendars] = useState<CalendarInfo[] | null>(null);
  const [choice, setChoice] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    api.listCalendars().then(
      (list) => {
        // Only calendars events can be added to.
        if (live) setCalendars(list.filter((c) => c.writable && accounts.some((a) => a.id === c.accountId)));
      },
      () => {
        if (live) setCalendars([]);
      },
    );
    return () => {
      live = false;
    };
  }, [api, accounts]);
  const calendar = calendars?.find((c) => c.id === choice) ?? calendars?.[0];
  const label = (c: CalendarInfo) => `${c.name} (${accounts.find((a) => a.id === c.accountId)?.email ?? ''})`;

  return (
    <Modal
      title={`Add ${file.name} to calendar`}
      description="The events in this file are added to the calendar you choose, and sync to its server."
      onClose={onClose}
      size="small"
    >
      {calendars === null ? (
        <p className="muted" role="status">
          Looking for calendars…
        </p>
      ) : calendar ? (
        <label className="form-field">
          <span className="form-field__label">Calendar</span>
          <select
            className="field"
            value={calendar.id}
            onChange={(e) => {
              setChoice(e.target.value);
            }}
          >
            {calendars.map((c) => (
              <option key={c.id} value={c.id}>
                {label(c)}
              </option>
            ))}
          </select>
        </label>
      ) : (
        <p className="notice notice--error" role="alert">
          None of your accounts has a calendar to add events to.
        </p>
      )}
      <div className="modal__actions">
        <button type="button" className="btn btn--secondary" onClick={onClose}>
          Cancel
        </button>
        <button
          type="button"
          className="btn btn--primary"
          disabled={!calendar}
          onClick={() => {
            if (!calendar) return;
            onClose();
            files.run({
              title: `Adding ${file.name} to ${calendar.name}`,
              doneTitle: 'Events added',
              noun: 'event',
              start: () => api.importOpenedFile(file.path, { calendarId: calendar.id }),
            });
          }}
        >
          Add to calendar
        </button>
      </div>
    </Modal>
  );
}

/** An opened .vcf file: add its contacts to an account's address book or this device. */
function ContactsFileDialog({ file, accounts, onClose }: { file: OpenedFile; accounts: readonly Account[]; onClose: () => void }) {
  const api = useMailApi();
  const files = useFileTasks();
  usePhoneBack(1, onClose);
  const [book, setBook] = useState(bookValue(accounts[0]?.id ?? null));
  return (
    <Modal
      title={`Add contacts from ${file.name}`}
      description="Contacts on an account sync to its server; on this device they stay here."
      onClose={onClose}
      size="small"
    >
      <label className="form-field">
        <span className="form-field__label">Add them to</span>
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
      <div className="modal__actions">
        <button type="button" className="btn btn--secondary" onClick={onClose}>
          Cancel
        </button>
        <button
          type="button"
          className="btn btn--primary"
          onClick={() => {
            const accountId = bookFromValue(book);
            onClose();
            files.run({
              title: `Adding contacts to ${addressBookName(accounts, accountId)}`,
              doneTitle: 'Contacts added',
              noun: 'contact',
              start: () => api.importOpenedFile(file.path, { accountId }),
            });
          }}
        >
          Add contacts
        </button>
      </div>
    </Modal>
  );
}
