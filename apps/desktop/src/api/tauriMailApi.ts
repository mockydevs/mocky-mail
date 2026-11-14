import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import { toAppError } from './errors';
import type { MailApi, MailEventName, MailEvents } from './types';

/** Call a command and normalise its rejection into an AppError. */
async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error: unknown) {
    throw toAppError(error);
  }
}

/** MailApi backed by the Rust commands in src-tauri (see docs/ipc.md). */
export function createTauriMailApi(): MailApi {
  return {
    suggestServers: (email) => call('suggest_servers', { email }),
    addAccount: (input) => call('add_account', { input }),
    addOAuthAccount: (input) => call('add_oauth_account', { input }),
    updateAccount: (id, patch) => call('update_account', { id, patch }),
    removeAccount: (id) => call('remove_account', { id }),
    listAccounts: () => call('list_accounts'),
    listFolders: (accountId) => call('list_folders', { accountId }),
    listMessages: (query) => call('list_messages', { query }),
    getMessage: (id, allowRemote) => call('get_message', { id, allowRemote }),
    setFlags: (ids, change) => call('set_flags', { ids, ...change }),
    moveMessages: (ids, target) => call('move_messages', { ids, target }),
    deleteForever: (ids) => call('delete_forever', { ids }),
    saveAttachment: (messageId, index) => call('save_attachment', { messageId, index }),
    pickAttachments: () => call('pick_attachments'),
    checkDraft: (draft) => call('check_draft', { draft }),
    sendMessage: (draft) => call('send_message', { draft }),
    saveDraft: (draft) => call('save_draft', { draft }),
    listOutbox: () => call('list_outbox'),
    outboxSendNow: (id) => call('outbox_send_now', { id }),
    outboxCancel: (id) => call('outbox_cancel', { id }),
    outboxEdit: (id) => call('outbox_edit', { id }),
    syncNow: (accountId) => call('sync_now', { accountId }),
    getSettings: () => call('get_settings'),
    setSettings: (settings) => call('set_settings', { settings }),
    openExternal: (url) => call('open_external', { url }),
    on<E extends MailEventName>(event: E, handler: (payload: MailEvents[E]) => void): () => void {
      let unlisten: (() => void) | null = null;
      let stopped = false;
      void listen<MailEvents[E]>(event, (e) => {
        handler(e.payload);
      }).then((fn) => {
        if (stopped) fn();
        else unlisten = fn;
      });
      return () => {
        stopped = true;
        unlisten?.();
      };
    },
  };
}
