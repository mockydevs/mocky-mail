import { forwardRef, useState, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { errorTitle, toAppError } from '../../api/errors';
import type { Account, AppError, FolderRole, MessageDetail, MessageListItem } from '../../api/types';
import { formatBytes, formatLongDate } from '../../lib/format';
import { useLoad } from '../../lib/useLoad';
import type { Theme } from '../../theme/useTheme';
import { Avatar } from '../../ui/Avatar';
import { Icon } from '../../ui/Icon';
import { formatMailbox, formatMailboxes, replyDraft, toolbarActions } from '../compose/seed';
import { linkify } from './linkify';
import { SandboxedHtml } from './SandboxedHtml';

export interface ReaderActions {
  onReply: (message: MessageDetail, all: boolean) => void;
  onForward: (message: MessageDetail) => void;
  onEdit: (message: MessageDetail) => void;
  onArchive: () => void;
  onDelete: () => void;
  onDeleteForever: () => void;
  onToggleFlag: () => void;
  onToggleRead: () => void;
}

interface Props extends ReaderActions {
  /** The selected list item, with optimistic changes applied. */
  item: MessageListItem | null;
  account: Account | undefined;
  role: FolderRole | null;
  alwaysLoadImages: boolean;
  /** App theme; HTML emails are shown dark to match. */
  theme: Theme;
  /** Loaded message, so keyboard shortcuts can reply to it. */
  onLoaded: (message: MessageDetail | null) => void;
}

export const ReadingPane = forwardRef<HTMLElement, Props>(function ReadingPane(props, ref) {
  const { item } = props;
  if (!item) {
    return (
      <article className="reader reader--empty" aria-label="Reading pane" ref={ref} tabIndex={-1}>
        <Icon name="mail" size={48} />
        <p className="empty__title">No message selected</p>
        <p className="empty__text">Pick a message from the list to read it here.</p>
      </article>
    );
  }
  return <MessageView key={item.id} {...props} item={item} ref={ref} />;
});

const MessageView = forwardRef<HTMLElement, Props & { item: MessageListItem }>(function MessageView(props, ref) {
  const api = useMailApi();
  const { item, account, role, alwaysLoadImages, theme, onLoaded } = props;
  const [showImages, setShowImages] = useState(false);
  // Per message: dark mode can be switched off for one email that reads badly inverted.
  const [originalColours, setOriginalColours] = useState(false);
  const allowRemote = alwaysLoadImages || showImages;
  const loaded = useLoad(`message:${item.id}:${String(allowRemote)}`, 0, () =>
    api.getMessage(item.id, allowRemote).then((m) => {
      onLoaded(m);
      return m;
    }),
  );
  const [notice, setNotice] = useState<{ error: AppError | null; text: string | null }>({ error: null, text: null });

  if (loaded.status === 'error') {
    return (
      <article className="reader reader--empty" aria-label="Reading pane" ref={ref} tabIndex={-1}>
        <p className="empty__title">This message could not be opened</p>
        <p className="empty__text">{loaded.message}</p>
      </article>
    );
  }
  if (loaded.status === 'loading') {
    return (
      <article className="reader" aria-label="Reading pane" aria-busy="true" ref={ref} tabIndex={-1}>
        <div className="reader__scroll">
          <div className="skeleton skeleton--title" />
          <div className="skeleton skeleton--line" />
          <div className="skeleton skeleton--line" />
        </div>
      </article>
    );
  }

  const message = loaded.value;
  const isHtml = message.html !== null;
  const actions = toolbarActions(role);
  const sender = message.from;
  const outgoing = account !== undefined && sender?.address.toLowerCase() === account.email.toLowerCase();

  const open = (url: string) => {
    api.openExternal(url).catch((e: unknown) => {
      setNotice({ error: toAppError(e), text: null });
    });
  };

  const save = (index: number) => {
    setNotice({ error: null, text: null });
    api.saveAttachment(message.id, index).then(
      (path) => {
        if (path !== null) setNotice({ error: null, text: `Saved to ${path}` });
      },
      (e: unknown) => {
        setNotice({ error: toAppError(e), text: null });
      },
    );
  };

  return (
    <article className="reader" aria-label="Reading pane" ref={ref} tabIndex={-1}>
      <div className="toolbar" role="toolbar" aria-label="Message actions">
        {actions.includes('edit') && (
          <button
            type="button"
            className="btn btn--ghost"
            onClick={() => {
              props.onEdit(message);
            }}
          >
            <Icon name="pen" />
            Continue editing
          </button>
        )}
        {actions.includes('reply') && (
          <button
            type="button"
            className="btn btn--ghost"
            onClick={() => {
              props.onReply(message, false);
            }}
          >
            <Icon name="reply" />
            Reply
          </button>
        )}
        {actions.includes('replyAll') && (
          <button
            type="button"
            className="btn btn--ghost"
            onClick={() => {
              props.onReply(message, true);
            }}
          >
            <Icon name="replyAll" />
            Reply all
          </button>
        )}
        {actions.includes('forward') && (
          <button
            type="button"
            className="btn btn--ghost"
            onClick={() => {
              props.onForward(message);
            }}
          >
            <Icon name="forward" />
            Forward
          </button>
        )}
        <span className="toolbar__divider" aria-hidden="true" />
        {actions.includes('archive') && (
          <button type="button" className="icon-btn" aria-label="Archive" title="Archive (e)" onClick={props.onArchive}>
            <Icon name="archive" />
          </button>
        )}
        {actions.includes('delete') && (
          <button type="button" className="icon-btn" aria-label="Delete" title="Delete (#)" onClick={props.onDelete}>
            <Icon name="trash" />
          </button>
        )}
        {actions.includes('deleteForever') && (
          <button type="button" className="btn btn--ghost" onClick={props.onDeleteForever}>
            <Icon name="trash" />
            Delete forever
          </button>
        )}
        {actions.includes('flag') && (
          <button
            type="button"
            className="icon-btn"
            aria-label="Flag"
            title="Flag (s)"
            aria-pressed={item.flagged}
            onClick={props.onToggleFlag}
          >
            <Icon name="flag" filled={item.flagged} />
          </button>
        )}
        {actions.includes('read') && (
          <button
            type="button"
            className="icon-btn"
            aria-label={item.unread ? 'Mark as read' : 'Mark as unread'}
            title="Read or unread (u)"
            onClick={props.onToggleRead}
          >
            <Icon name={item.unread ? 'envelopeOpen' : 'mail'} />
          </button>
        )}
      </div>

      <div className={isHtml ? 'reader__head' : 'reader__scroll'}>
        <h2 className="reader__subject">{message.subject || '(no subject)'}</h2>
        <header className="reader__header">
          <Avatar name={sender?.name ?? sender?.address ?? '?'} address={sender?.address ?? ''} size={44} />
          <div className="reader__people">
            <p className="reader__name">
              {sender ? (
                <>
                  {sender.name ?? sender.address}{' '}
                  {sender.name && <span className="muted">&lt;{sender.address}&gt;</span>}
                </>
              ) : (
                'Unknown sender'
              )}
            </p>
            <p className="reader__meta">To {formatMailboxes(message.to) || 'no recipients'}</p>
            {message.cc.length > 0 && <p className="reader__meta">Cc {formatMailboxes(message.cc)}</p>}
            {message.replyTo.length > 0 && !outgoing && (
              <p className="reader__meta">Replies go to {message.replyTo.map(formatMailbox).join(', ')}</p>
            )}
          </div>
          <div className="reader__when">
            {account && (
              <span className="chip">
                <span className="dot" style={{ background: account.color }} aria-hidden="true" />
                {account.displayName || account.email}
              </span>
            )}
            <time dateTime={message.date === null ? undefined : new Date(message.date * 1000).toISOString()}>
              {formatLongDate(message.date)}
            </time>
            {isHtml && theme === 'dark' && (
              <button
                type="button"
                className="icon-btn"
                aria-pressed={originalColours}
                aria-label="Show original colours"
                title={originalColours ? 'Show this email in dark mode' : 'Show this email in its original colours'}
                onClick={() => {
                  setOriginalColours((on) => !on);
                }}
              >
                <Icon name={originalColours ? 'moon' : 'sun'} />
              </button>
            )}
          </div>
        </header>

        {message.html !== null && message.blockedRemoteImages > 0 && !allowRemote && (
          <div className="images-bar" role="status">
            <Icon name="image" />
            <span>
              {message.blockedRemoteImages === 1 ? '1 image was' : `${String(message.blockedRemoteImages)} images were`} blocked
              to protect your privacy.
            </span>
            <button
              type="button"
              className="btn btn--secondary btn--small"
              onClick={() => {
                setShowImages(true);
              }}
            >
              Show images ({message.blockedRemoteImages} blocked)
            </button>
          </div>
        )}

        {message.attachments.length > 0 && (
          <ul className="attachments" aria-label="Attachments">
            {message.attachments.map((file) => (
              <li key={file.index} className="attachment">
                <span className="attachment__icon" aria-hidden="true">
                  {fileBadge(file.fileName)}
                </span>
                <span className="attachment__text">
                  <span className="attachment__name">{file.fileName}</span>
                  <span className="muted small">{formatBytes(file.size)}</span>
                </span>
                <button
                  type="button"
                  className="icon-btn"
                  aria-label={`Save ${file.fileName}`}
                  onClick={() => {
                    save(file.index);
                  }}
                >
                  <Icon name="download" />
                </button>
              </li>
            ))}
          </ul>
        )}

        {notice.text && (
          <p className="form-hint" role="status">
            {notice.text}
          </p>
        )}
        {notice.error && (
          <p className="form-error" role="alert">
            {notice.error.message}
          </p>
        )}

        {!isHtml && (
          <div className="reader__body">
              <div className="reader__text">
                {linkify(message.text ?? '').map((part, i) =>
                  part.kind === 'text' ? (
                    <span key={i}>{part.text}</span>
                  ) : (
                    <a
                      key={i}
                      href={part.url}
                      onClick={(e) => {
                        e.preventDefault();
                        open(part.url);
                      }}
                    >
                      {part.text}
                    </a>
                  ),
                )}
              </div>
          </div>
        )}
      </div>

      {isHtml && (
        <div
          className="reader__frame"
          style={{ display: 'flex', flexDirection: 'column', flex: '1 1 auto', minHeight: 0 }}
        >
          <SandboxedHtml
            title={`Message: ${message.subject}`}
            html={message.html ?? ''}
            theme={originalColours ? 'light' : theme}
            allowRemoteImages={allowRemote}
          />
        </div>
      )}

      {actions.includes('reply') && account && <QuickReply message={message} account={account} />}
    </article>
  );
});

/** A short reply without leaving the reader; it goes through the same checks and Outbox as compose. */
function QuickReply({ message, account }: { message: MessageDetail; account: Account }) {
  const api = useMailApi();
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const [sent, setSent] = useState(false);
  const firstName = (message.from?.name ?? message.from?.address ?? '').split(' ')[0] ?? '';

  const submit = (event: SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!text.trim() || busy) return;
    const base = replyDraft(message, account, false);
    const draft = { ...base, bodyText: `${text.trim()}${base.bodyText}` };
    setBusy(true);
    setError(null);
    setSent(false);
    api
      .checkDraft(draft)
      .then(() => api.sendMessage(draft))
      .then(
        () => {
          setText('');
          setSent(true);
        },
        (e: unknown) => {
          setError(toAppError(e));
        },
      )
      .finally(() => {
        setBusy(false);
      });
  };

  return (
    <form className="quick-reply" onSubmit={submit}>
      <label className="quick-reply__label">
        Quick reply to {firstName || 'sender'}
        <textarea
          className="field"
          rows={2}
          value={text}
          placeholder="Write a reply"
          onChange={(e) => {
            setText(e.target.value);
            setSent(false);
          }}
        />
      </label>
      <button type="submit" className="btn btn--primary btn--large" disabled={busy || !text.trim()}>
        <Icon name="sent" size={16} />
        Send
      </button>
      {sent && (
        <p className="quick-reply__status form-hint" role="status">
          Reply queued in the Outbox.
        </p>
      )}
      {error && (
        <p className="quick-reply__status form-error" role="alert">
          {errorTitle(error.kind)}: {error.message}
        </p>
      )}
    </form>
  );
}

function fileBadge(name: string): string {
  const dot = name.lastIndexOf('.');
  return dot > 0 ? name.slice(dot + 1, dot + 5).toUpperCase() : 'FILE';
}
