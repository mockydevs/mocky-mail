import { vi } from 'vitest';

const invoke = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>();
const unlisten = vi.fn();
let listener: ((event: { payload: unknown }) => void) | null = null;
const listen = vi.fn((_name: string, cb: (event: { payload: unknown }) => void) => {
  listener = cb;
  return Promise.resolve(unlisten);
});

vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invoke(cmd, args) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: (name: string, cb: (event: { payload: unknown }) => void) => listen(name, cb),
}));

const { createTauriMailApi } = await import('./tauriMailApi');

describe('createTauriMailApi', () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(undefined);
    listener = null;
  });

  it('maps methods to commands with camelCase args from docs/ipc.md', async () => {
    const api = createTauriMailApi();
    await api.getMessage('m1', true);
    await api.setFlags(['a', 'b'], { seen: true });
    await api.moveMessages(['a'], { role: 'archive' });
    await api.saveAttachment('m1', 2);
    await api.syncNow(null);
    await api.listFolders('acc');
    await api.attachmentsForDraft('m9');
    await api.setSettings({ theme: 'dark', remoteImages: 'block', notifications: true, sound: true });
    expect(invoke.mock.calls).toEqual([
      ['get_message', { id: 'm1', allowRemote: true }],
      ['set_flags', { ids: ['a', 'b'], seen: true }],
      ['move_messages', { ids: ['a'], target: { role: 'archive' } }],
      ['save_attachment', { messageId: 'm1', index: 2 }],
      ['sync_now', { accountId: null }],
      ['list_folders', { accountId: 'acc' }],
      ['attachments_for_draft', { messageId: 'm9' }],
      ['set_settings', { settings: { theme: 'dark', remoteImages: 'block', notifications: true, sound: true } }],
    ]);
  });

  it('passes AppErrors through and hides anything else', async () => {
    const api = createTauriMailApi();
    invoke.mockRejectedValueOnce({ kind: 'auth', message: 'Wrong password' });
    await expect(api.listAccounts()).rejects.toMatchObject({ kind: 'auth', message: 'Wrong password' });
    invoke.mockRejectedValueOnce('thread panicked');
    await expect(api.listAccounts()).rejects.toMatchObject({ kind: 'internal' });
  });

  it('delivers event payloads and unsubscribes even before listen resolves', async () => {
    const api = createTauriMailApi();
    const handler = vi.fn();
    const stop = api.on('mail://new-mail', handler);
    stop();
    await Promise.resolve();
    await Promise.resolve();
    expect(unlisten).toHaveBeenCalledTimes(1);

    const handler2 = vi.fn();
    api.on('mail://changed', handler2);
    await Promise.resolve();
    listener?.({ payload: { accountId: 'a', folderIds: ['f'] } });
    expect(handler2).toHaveBeenCalledWith({ accountId: 'a', folderIds: ['f'] });
  });
});
