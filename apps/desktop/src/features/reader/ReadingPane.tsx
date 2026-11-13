import { useEffect, useState, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import type { Account, Attachment, MessageDetail } from '../../api/types';
import { formatBytes, formatLongDate } from '../../lib/format';
import { useLoad } from '../../lib/useLoad';
import { Avatar } from '../../ui/Avatar';
import { Icon } from '../../ui/Icon';
import { editSeed, forwardSeed, replySeed, replySubject, toolbarActions, type ComposeSeed } from '../compose/seed';
import { OutboxBanner } from '../outbox/OutboxBanner';
import { SandboxedHtml } from './SandboxedHtml';

interface Props {
  messageId: string | null;
  /**
   * Mark the message read when it opens. Only true when the user picked it:
   * auto-selecting the first message must not mark it read, or the Unread
   * filter would read every message in turn.
   */
  markRead: boolean;
  revision: number;
  accounts: readonly Account[];
  onChanged: () => void;
  onCompose: (seed: ComposeSeed) => void;
}

export function ReadingPane({ messageId, markRead, revision, accounts, onChanged, onCompose }: Props) {
  const api = useMailApi();
  const loaded = useLoad(`message:${messageId ?? ''}`, revision, () =>
    messageId === null ? Promise.resolve(null) : api.getMessage(messageId),
  );
  const message = loaded.status === 'ready' ? loaded.value : null;
  const unreadId = markRead && message?.unread ? message.id : null;

  useEffect(() => {
    if (unreadId === null) return;
    void api.markRead(unreadId).then(onChanged);
  }, [api, unreadId, onChanged]);

  if (messageId === null) {
    return (
      <article className="reader reader--empty" aria-label="Reading pane">
        <Icon name="mail" size={48} />
        <p className="empty__title">No message selected</p>
        <p className="empty__text">Pick a message from the list to read it here.</p>
      </article>
    );
  }
  if (loaded.status === 'error') {
    return (
      <article className="reader reader--empty" aria-label="Reading pane">
        <p className="empty__title">This message could not be opened</p>
        <p className="empty__text">{loaded.message}</p>
      </article>
    );
  }
  if (!message) {
    return (
      <article className="reader" aria-label="Reading pane" aria-busy="true">
        <div className="reader__scroll">
          <div className="skeleton skeleton--title" />
          <div className="skeleton skeleton--line" />
          <div className="skeleton skeleton--line" />
        </div>
      </article>
    );
  }

  const account = accounts.find((a) => a.id === message.accountId);
  const accountLabel = account?.label ?? message.accountId;
  const act = (run: () => Promise<void>) => () => {
    void run().then(onChanged);
  };

  return (
    <article className="reader" aria-label="Reading pane">
      <Toolbar
        message={message}
        onReply={() => {
          onCompose(replySeed(message));
        }}
        onForward={() => {
          onCompose(forwardSeed(message));
        }}
        onEdit={() => {
          onCompose(editSeed(message));
        }}
        onArchive={act(() => api.moveToFolder(message.id, 'archive'))}
        onDelete={act(() => api.moveToFolder(message.id, 'trash'))}
        onFlag={act(() => api.setFlagged(message.id, !message.flagged))}
      />
      <div className="reader__scroll">
        {message.outbox && (
          <OutboxBanner
            status={message.outbox}
            accountLabel={accountLabel}
            onSendNow={act(() => api.outbox.sendNow(message.id))}
            onEdit={() => {
              onCompose(editSeed(message));
            }}
            onCancel={act(() => api.outbox.cancel(message.id))}
          />
        )}
        <h2 className="reader__subject">{message.subject || '(no subject)'}</h2>
        <header className="reader__header">
          <Avatar name={message.counterpart.name} address={message.counterpart.address} size={44} />
          <div className="reader__people">
            <p className="reader__name">
              {message.direction === 'outgoing' ? 'To ' : ''}
              {message.counterpart.name || message.counterpart.address}{' '}
              <span className="muted">&lt;{message.counterpart.address}&gt;</span>
            </p>
            <p className="reader__meta">
              {message.direction === 'outgoing' ? 'From ' : 'To '}
              {account?.address ?? ''}
            </p>
          </div>
          <div className="reader__when">
            <span className="chip">
              <span className={`dot dot--${account?.hue ?? 'blue'}`} aria-hidden="true" />
              {accountLabel}
            </span>
            <time dateTime={message.date}>{formatLongDate(message.date)}</time>
          </div>
        </header>
        <div className="reader__body">
          {message.body.kind === 'html' ? (
            <SandboxedHtml title={`Message: ${message.subject}`} html={message.body.html} />
          ) : (
            <div className="reader__text">{message.body.text}</div>
          )}
        </div>
        {message.attachments.length > 0 && <Attachments items={message.attachments} />}
      </div>
      {message.folder === 'inbox' && (
        <QuickReply
          key={message.id}
          message={message}
          onSent={onChanged}
        />
      )}
    </article>
  );
}

interface ToolbarProps {
  message: MessageDetail;
  onReply: () => void;
  onForward: () => void;
  onEdit: () => void;
  onArchive: () => void;
  onDelete: () => void;
  onFlag: () => void;
}

function Toolbar({ message, onReply, onForward, onEdit, onArchive, onDelete, onFlag }: ToolbarProps) {
  const actions = toolbarActions(message);
  if (actions.length === 0) return null;
  return (
    <div className="toolbar" role="toolbar" aria-label="Message actions">
      {actions.includes('reply') && (
        <button type="button" className="btn btn--ghost" onClick={onReply}>
          <Icon name="reply" />
          Reply
        </button>
      )}
      {actions.includes('forward') && (
        <button type="button" className="btn btn--ghost" onClick={onForward}>
          <Icon name="forward" />
          Forward
        </button>
      )}
      {actions.includes('edit') && (
        <button type="button" className="btn btn--ghost" onClick={onEdit}>
          <Icon name="pen" />
          Continue editing
        </button>
      )}
      <span className="toolbar__divider" aria-hidden="true" />
      {actions.includes('archive') && (
        <button type="button" className="icon-btn" aria-label="Archive" onClick={onArchive}>
          <Icon name="archive" />
        </button>
      )}
      {actions.includes('delete') && (
        <button type="button" className="icon-btn" aria-label="Delete" onClick={onDelete}>
          <Icon name="trash" />
        </button>
      )}
      {actions.includes('flag') && (
        <button type="button" className="icon-btn" aria-label="Flag" aria-pressed={message.flagged} onClick={onFlag}>
          <Icon name="flag" filled={message.flagged} />
        </button>
      )}
    </div>
  );
}

function fileTone(mimeType: string): string {
  if (mimeType === 'application/pdf') return 'pdf';
  if (mimeType.startsWith('image/')) return 'image';
  return 'other';
}

function extension(name: string): string {
  const dot = name.lastIndexOf('.');
  return dot > 0 ? name.slice(dot + 1, dot + 5).toUpperCase() : 'FILE';
}

function Attachments({ items }: { items: readonly Attachment[] }) {
  return (
    <ul className="attachments" aria-label="Attachments">
      {items.map((file) => (
        <li key={file.name} className="attachment">
          <span className={`attachment__icon attachment__icon--${fileTone(file.mimeType)}`} aria-hidden="true">
            {extension(file.name)}
          </span>
          <span className="attachment__text">
            <span className="attachment__name">{file.name}</span>
            <span className="muted small">{formatBytes(file.sizeBytes)}</span>
          </span>
        </li>
      ))}
    </ul>
  );
}

function QuickReply({ message, onSent }: { message: MessageDetail; onSent: () => void }) {
  const api = useMailApi();
  const [text, setText] = useState('');
  const [sending, setSending] = useState(false);
  const firstName = (message.counterpart.name || message.counterpart.address).split(' ')[0] ?? '';
  const canSend = text.trim() !== '' && !sending;

  const submit = (event: SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!canSend) return;
    setSending(true);
    void api
      .queueMessage({
        accountId: message.accountId,
        to: [message.counterpart.address],
        cc: [],
        bcc: [],
        subject: replySubject(message.subject),
        body: text.trim(),
      })
      .then(() => {
        setText('');
        onSent();
      })
      .finally(() => {
        setSending(false);
      });
  };

  return (
    <form className="quick-reply" onSubmit={submit}>
      <label className="quick-reply__label">
        Quick reply to {firstName}
        <textarea
          className="field"
          rows={2}
          value={text}
          placeholder="Write a reply"
          onChange={(e) => {
            setText(e.target.value);
          }}
        />
      </label>
      <button type="submit" className="btn btn--primary btn--large" disabled={!canSend}>
        <Icon name="sent" size={16} />
        Send
      </button>
    </form>
  );
}
