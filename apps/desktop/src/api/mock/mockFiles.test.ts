import type { ImportProgress } from '../types';
import { createMockMailApi } from './mockMailApi';

describe('mock backend: 0.9.0 files', () => {
  it('reports progress, then what an import brought in', async () => {
    const api = createMockMailApi();
    const steps: ImportProgress[] = [];
    api.on('mail://import-progress', (p) => steps.push(p));
    const report = await api.importMailFiles('work:Clients');
    expect(report).toMatchObject({ imported: 3, skipped: 1, folderId: 'work:Clients' });
    expect(steps[0]).toEqual({ phase: 'Reading files', done: 0, total: 0 });
    expect(steps.at(-1)).toEqual({ phase: 'Uploading messages', done: 3, total: 3 });
    const page = await api.listMessages({
      accountId: 'work',
      folderId: 'work:Clients',
      role: null,
      filter: 'all',
      search: null,
      offset: 0,
      limit: 50,
    });
    expect(page.items.map((m) => m.subject)).toContain('Holiday plans');
  });

  it('returns null when a picker is cancelled, and checks the target first', async () => {
    const api = createMockMailApi();
    api.cancelNextPick();
    expect(await api.exportCalendar('work:personal')).toBeNull();
    await expect(api.exportMail({ ids: ['m1'] }, 'mbox')).rejects.toMatchObject({ kind: 'invalid' });
    await expect(api.importMailFiles('nowhere')).rejects.toMatchObject({ kind: 'not_found' });
    expect(await api.exportMail({ ids: ['m1', 'm2'] }, 'eml')).toBe('/home/you/Documents/Messages (2).zip');
  });

  it('imports an Outlook file into a new folder tree with counts by kind', async () => {
    const api = createMockMailApi();
    const report = await api.importPst('personal');
    expect(report?.counts).toEqual({ messages: 4, contacts: 1, events: 1, tasks: 2, notes: 1 });
    const folders = await api.listFolders('personal');
    expect(folders.filter((f) => f.path.startsWith('Imported from Outlook')).map((f) => f.path)).toEqual([
      'Imported from Outlook',
      'Imported from Outlook.Inbox',
      'Imported from Outlook.Sent Items',
    ]);
  });

  it('hands over opened files once, and reads an opened .eml without filing it', async () => {
    const api = createMockMailApi({ openedFiles: [{ kind: 'eml', path: '/tmp/a.eml', name: 'a.eml' }] });
    expect(await api.takeOpenedFiles()).toHaveLength(1);
    expect(await api.takeOpenedFiles()).toEqual([]);
    const message = await api.readMessageFile('/tmp/a.eml');
    expect(message.subject).toBe('Signed drawings for the Kilimani house');
    await expect(api.readMessageFile('/tmp/broken.eml')).rejects.toMatchObject({ kind: 'invalid' });
    await api.importMessageFile('/tmp/a.eml', 'work:inbox');
    expect((await api.listFolders('work')).find((f) => f.id === 'work:inbox')?.total).toBe(5);
  });

  it('adds an opened calendar or contacts file', async () => {
    const api = createMockMailApi();
    expect(await api.importOpenedFile('/tmp/x.ics', { calendarId: 'work:personal' })).toMatchObject({ imported: 2 });
    expect(await api.importOpenedFile('/tmp/x.vcf', { accountId: null })).toMatchObject({ imported: 2 });
    expect((await api.listContacts('Faith'))[0]?.accountId).toBeNull();
    await expect(api.importOpenedFile('/tmp/x.ics', { calendarId: 'gone' })).rejects.toMatchObject({ kind: 'not_found' });
  });
});
