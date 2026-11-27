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
    suggestRecipients: (query) => call('suggest_recipients', { query }),
    addAccount: (input) => call('add_account', { input }),
    addOAuthAccount: (input) => call('add_oauth_account', { input }),
    updateAccount: (id, patch) => call('update_account', { id, patch }),
    reconnectAccount: (id) => call('reconnect_account', { id }),
    removeAccount: (id) => call('remove_account', { id }),
    listAccounts: () => call('list_accounts'),
    listFolders: (accountId) => call('list_folders', { accountId }),
    createFolder: (accountId, name, parentId) => call('create_folder', { accountId, name, parentId }),
    renameFolder: (folderId, name) => call('rename_folder', { folderId, name }),
    deleteFolder: (folderId) => call('delete_folder', { folderId }),
    listMessages: ({ threads, ...query }) => call('list_messages', { query, threads: threads ?? false }),
    getThread: (id) => call('get_thread', { id }),
    previewBody: (body) => call('preview_body', { body }),
    takeMailto: () => call('take_mailto'),
    parseMailto: (url) => call('parse_mailto', { url }),
    setFocusRule: (sender, focus, wholeDomain) => call('set_focus_rule', { sender, focus, wholeDomain }),
    parseSearch: (query) => call('parse_search', { query }),
    listRules: () => call('list_rules'),
    saveRule: (rule) => call('save_rule', { rule }),
    deleteRule: (id) => call('delete_rule', { id }),
    reorderRules: (ids) => call('reorder_rules', { ids }),
    runRules: (ruleIds, folderId) => call('run_rules', { ruleIds, folderId }),
    previewRule: (rule, folderId) => call('preview_rule', { rule, folderId }),
    listImageSenders: () => call('list_image_senders'),
    trustImageSender: (sender, wholeDomain) => call('trust_image_sender', { sender, wholeDomain }),
    untrustImageSender: (sender) => call('untrust_image_sender', { sender }),
    getMessage: (id, allowRemote) => call('get_message', { id, allowRemote }),
    setFlags: (ids, change) => call('set_flags', { ids, ...change }),
    moveMessages: (ids, target) => call('move_messages', { ids, target }),
    deleteForever: (ids) => call('delete_forever', { ids }),
    copyMessages: (ids, target) => call('copy_messages', { ids, target }),
    markJunk: (ids, junk) => call('mark_junk', { ids, junk }),
    listCategories: () => call('list_categories'),
    saveCategory: (category, previousName) => call('save_category', { category, previousName }),
    deleteCategory: (name) => call('delete_category', { name }),
    setCategories: (ids, add, remove) => call('set_categories', { ids, add, remove }),
    setFolderFavorite: (folderId, favorite) => call('set_folder_favorite', { folderId, favorite }),
    reorderAccounts: (ids) => call('reorder_accounts', { ids }),
    openMessageWindow: (id) => call('open_message_window', { id }),
    saveAttachment: (messageId, index) => call('save_attachment', { messageId, index }),
    saveMessage: (messageId) => call('save_message', { messageId }),
    messageSource: (messageId) => call('message_source', { messageId }),
    printHtml: (html, title) => call('print_html', { html, title }),
    previewAttachment: (messageId, index) => call('preview_attachment', { messageId, index }),
    openAttachment: (messageId, index) => call('open_attachment', { messageId, index }),
    pickAttachments: () => call('pick_attachments'),
    checkDraft: (draft) => call('check_draft', { draft }),
    sendMessage: (draft) => call('send_message', { draft }),
    saveDraft: (draft) => call('save_draft', { draft }),
    attachmentsForDraft: (messageId) => call('attachments_for_draft', { messageId }),
    listOutbox: () => call('list_outbox'),
    outboxSendNow: (id) => call('outbox_send_now', { id }),
    outboxCancel: (id) => call('outbox_cancel', { id }),
    outboxEdit: (id) => call('outbox_edit', { id }),
    syncNow: (accountId) => call('sync_now', { accountId }),
    checkForUpdate: () => call('check_for_update'),
    installUpdate: () => call('install_update'),
    listContacts: (query) => call('list_contacts', { query }),
    getContact: (id) => call('get_contact', { id }),
    saveContact: (contact) => call('save_contact', { contact }),
    deleteContact: (id) => call('delete_contact', { id }),
    syncContacts: () => call('sync_contacts'),
    listCalendars: () => call('list_calendars'),
    listEvents: (from, to) => call('list_events', { from, to }),
    getEvent: (id) => call('get_event', { id }),
    saveEvent: (event) => call('save_event', { event }),
    deleteEvent: (id, occurrence) => call('delete_event', { id, occurrence }),
    syncCalendars: () => call('sync_calendars'),
    answerInvitation: (id, answer, notify) => call('answer_invitation', { id, answer, notify }),
    removeCancelledEvent: (id) => call('remove_cancelled_event', { id }),
    applyInvitationReply: (id) => call('apply_invitation_reply', { id }),
    getOutOfOffice: (accountId) => call('get_out_of_office', { accountId }),
    setOutOfOffice: (accountId, settings) => call('set_out_of_office', { accountId, settings }),
    getSignature: (accountId, reply) => call('get_signature', { accountId, reply: reply ?? false }),
    listSignatures: (accountId) => call('list_signatures', { accountId }),
    saveSignature: (accountId, id, name, html) => call('save_signature', { accountId, id, name, html }),
    deleteSignature: (accountId, id) => call('delete_signature', { accountId, id }),
    setSignatureDefaults: (accountId, defaultNew, defaultReply) =>
      call('set_signature_defaults', { accountId, defaultNew, defaultReply }),
    setSignatureImage: (accountId, id) => call('set_signature_image', { accountId, id }),
    setSignatureImageWidth: (accountId, id, width) => call('set_signature_image_width', { accountId, id, width }),
    removeSignatureImage: (accountId, id) => call('remove_signature_image', { accountId, id }),
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
