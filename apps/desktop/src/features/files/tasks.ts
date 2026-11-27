import type { Account, CalendarInfo, Folder, MailApi } from '../../api/types';
import { folderLabel } from '../mailbox/selectors';
import type { FileTask } from './FileTasks';

/** "Import messages into this folder…": pick .eml and .mbox files, upload them there. */
export function importMailTask(api: MailApi, folder: Folder, onDone?: () => void): FileTask {
  return {
    title: `Importing messages into ${folderLabel(folder)}`,
    doneTitle: 'Messages imported',
    noun: 'message',
    start: () => api.importMailFiles(folder.id),
    ...(onDone ? { onDone } : {}),
  };
}

/** "Export folder as .mbox…". */
export function exportFolderTask(api: MailApi, folder: Folder): FileTask {
  return {
    title: `Saving ${folderLabel(folder)} as .mbox`,
    failedTitle: 'Export failed',
    noun: 'message',
    start: () => api.exportMail({ folderId: folder.id }, 'mbox'),
  };
}

/** "Save as .eml (zip)…" for ticked messages. */
export function exportMessagesTask(api: MailApi, ids: string[]): FileTask {
  return {
    title: ids.length === 1 ? 'Saving the message' : `Saving ${String(ids.length)} messages`,
    failedTitle: 'Export failed',
    noun: 'message',
    start: () => api.exportMail({ ids }, 'eml'),
  };
}

export function importCalendarTask(api: MailApi, calendar: CalendarInfo, onDone?: () => void): FileTask {
  return {
    title: `Importing events into ${calendar.name}`,
    doneTitle: 'Events imported',
    noun: 'event',
    start: () => api.importCalendarFile(calendar.id),
    ...(onDone ? { onDone } : {}),
  };
}

export function exportCalendarTask(api: MailApi, calendar: CalendarInfo): FileTask {
  return {
    title: `Saving ${calendar.name}`,
    failedTitle: 'Export failed',
    noun: 'event',
    start: () => api.exportCalendar(calendar.id),
  };
}

/** Where contacts go or come from: an account's address book, or this device (null). */
export function addressBookName(accounts: readonly Account[], accountId: string | null): string {
  if (accountId === null) return 'this device';
  const account = accounts.find((a) => a.id === accountId);
  return account ? account.email : 'the account';
}

export function importContactsTask(
  api: MailApi,
  accounts: readonly Account[],
  accountId: string | null,
  onDone?: () => void,
): FileTask {
  return {
    title: `Importing contacts to ${addressBookName(accounts, accountId)}`,
    doneTitle: 'Contacts imported',
    noun: 'contact',
    start: () => api.importContactsFile(accountId),
    ...(onDone ? { onDone } : {}),
  };
}

/** `'*'`: everyone's contacts in one file. */
export function exportContactsTask(api: MailApi, accounts: readonly Account[], accountId: string | null): FileTask {
  return {
    title: accountId === '*' ? 'Saving all contacts' : `Saving the contacts on ${addressBookName(accounts, accountId)}`,
    failedTitle: 'Export failed',
    noun: 'contact',
    start: () => api.exportContacts(accountId),
  };
}

export function importPstTask(api: MailApi, account: Account, onDone?: () => void): FileTask {
  return {
    title: `Importing from Outlook into ${account.email}`,
    doneTitle: 'Outlook import finished',
    failedTitle: 'Outlook import failed',
    noun: 'item',
    long: true,
    start: () => api.importPst(account.id),
    ...(onDone ? { onDone } : {}),
  };
}
