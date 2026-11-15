import { forwardRef, useEffect, useRef, useState, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { errorTitle, toAppError } from '../../api/errors';
import type { Account, AppError, FolderRole, MessageDetail, MessageListItem } from '../../api/types';
import { formatBytes, formatListTime, formatLongDate, nowSeconds } from '../../lib/format';
import { useLoad } from '../../lib/useLoad';
import type { Theme } from '../../theme/useTheme';
import { Avatar } from '../../ui/Avatar';
import { Icon } from '../../ui/Icon';
import { formatMailbox, formatMailboxes, replyDraft, toolbarActions } from '../compose/seed';
import { useSendStatus } from '../outbox/useSendStatus';
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
  /** Phone layout: the message fills the screen and scrolls as one page. */
  narrow?: boolean;
  /** Phone layout: back to the message list. */
  onBack?: () => void;
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
  // Phone layout only.
  const [details, setDetails] = useState(false);
  const [menu, setMenu] = useState(false);
  const [replying, setReplying] = useState(false);
  const [pastTitle, setPastTitle] = useState(false);

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

  const attachmentList = message.attachments.length > 0 && (
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
  );

  const notices = (
    <>
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
    </>
  );

  const textBody = (
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
  );

  if (props.narrow) {
    const blocked = isHtml && message.blockedRemoteImages > 0 && !allowRemote;
    const toMe = account !== undefined && message.to.some((m) => m.address.toLowerCase() === account.email.toLowerCase());
    const firstTo = message.to[0];
    const toLabel = toMe ? 'to me' : firstTo ? `to ${firstTo.name ?? firstTo.address}` : 'no recipients';
    const subject = message.subject || '(no subject)';
    return (
      <article className="reader reader--phone" aria-label="Reading pane" ref={ref} tabIndex={-1}>
        <div className="phone-bar" role="toolbar" aria-label="Message actions">
          <button type="button" className="icon-btn" aria-label="Back to list" onClick={props.onBack}>
            <Icon name="back" />
          </button>
          <span className="phone-bar__title" aria-hidden="true">
            {pastTitle ? subject : ''}
          </span>
          {actions.includes('archive') && (
            <button type="button" className="icon-btn" aria-label="Archive" onClick={props.onArchive}>
              <Icon name="archive" />
            </button>
          )}
          {actions.includes('delete') && (
            <button type="button" className="icon-btn" aria-label="Delete" onClick={props.onDelete}>
              <Icon name="trash" />
            </button>
          )}
          {actions.includes('deleteForever') && (
            <button type="button" className="icon-btn" aria-label="Delete forever" onClick={props.onDeleteForever}>
              <Icon name="trash" />
            </button>
          )}
          {(actions.includes('flag') || actions.includes('read')) && (
            <div className="phone-menu">
              <button
                type="button"
                className="icon-btn"
                aria-label="More actions"
                aria-expanded={menu}
                onClick={() => {
                  setMenu((on) => !on);
                }}
              >
                <Icon name="more" />
              </button>
              {menu && (
                <div className="phone-menu__list">
                  {actions.includes('flag') && (
                    <button
                      type="button"
                      className="phone-menu__item"
                      onClick={() => {
                        setMenu(false);
                        props.onToggleFlag();
                      }}
                    >
                      <Icon name="flag" filled={item.flagged} />
                      {item.flagged ? 'Remove flag' : 'Flag'}
                    </button>
                  )}
                  {actions.includes('read') && (
                    <button
                      type="button"
                      className="phone-menu__item"
                      onClick={() => {
                        setMenu(false);
                        props.onToggleRead();
                      }}
                    >
                      <Icon name={item.unread ? 'envelopeOpen' : 'mail'} />
                      {item.unread ? 'Mark as read' : 'Mark as unread'}
                    </button>
                  )}
                </div>
              )}
            </div>
          )}
        </div>

        <div
          className="phone-scroll"
          onScroll={(e) => {
            // Once the subject has scrolled away, the bar shows it instead.
            setPastTitle(e.currentTarget.scrollTop > 48);
          }}
        >
          <div className="phone-head">
            <h2 className="reader__subject">{subject}</h2>
            <div className="phone-sender">
              <Avatar name={sender?.name ?? sender?.address ?? '?'} address={sender?.address ?? ''} size={32} />
              <div className="phone-sender__text">
                <p className="phone-sender__name">{sender ? (sender.name ?? sender.address) : 'Unknown sender'}</p>
                <button
                  type="button"
                  className="phone-sender__more"
                  aria-expanded={details}
                  onClick={() => {
                    setDetails((on) => !on);
                  }}
                >
                  {toLabel} · {formatListTime(message.date, nowSeconds())}
                  <Icon name="chevron" size={14} />
                </button>
              </div>
            </div>
            {details && (
              <div className="phone-details">
                {sender?.name && <p className="reader__meta">From {sender.address}</p>}
                <p className="reader__meta">To {formatMailboxes(message.to) || 'no recipients'}</p>
                {message.cc.length > 0 && <p className="reader__meta">Cc {formatMailboxes(message.cc)}</p>}
                {message.replyTo.length > 0 && !outgoing && (
                  <p className="reader__meta">Replies go to {message.replyTo.map(formatMailbox).join(', ')}</p>
                )}
                <p className="reader__meta">{formatLongDate(message.date)}</p>
                <div className="phone-details__row">
                  {account && (
                    <span className="account-chip" title={account.email}>
                      <span className="dot" style={{ background: account.color }} aria-hidden="true" />
                      <span className="account-chip__name">{account.displayName || account.email}</span>
                    </span>
                  )}
                  {isHtml && theme === 'dark' && (
                    <button
                      type="button"
                      className="btn btn--ghost btn--small"
                      aria-pressed={originalColours}
                      onClick={() => {
                        setOriginalColours((on) => !on);
                      }}
                    >
                      <Icon name={originalColours ? 'moon' : 'sun'} size={16} />
                      Show original colours
                    </button>
                  )}
                </div>
              </div>
            )}
            {blocked && (
              <div className="images-strip" role="status">
                <Icon name="image" size={16} />
                <span>
                  {message.blockedRemoteImages === 1 ? '1 image' : `${String(message.blockedRemoteImages)} images`} hidden
                </span>
                <button
                  type="button"
                  className="images-strip__show"
                  aria-label={`Show images (${String(message.blockedRemoteImages)} blocked)`}
                  onClick={() => {
                    setShowImages(true);
                  }}
                >
                  Show
                </button>
              </div>
            )}
            {attachmentList}
            {notices}
          </div>
          {isHtml ? (
            <SandboxedHtml
              title={`Message: ${message.subject}`}
              html={message.html ?? ''}
              theme={originalColours ? 'light' : theme}
              allowRemoteImages={allowRemote}
              fitContent
            />
          ) : (
            <div className="phone-text">{textBody}</div>
          )}
        </div>

        {replying && account ? (
          <QuickReply
            message={message}
            account={account}
            sheet={{
              onClose: () => {
                setReplying(false);
              },
              onFullEditor: () => {
                props.onReply(message, false);
              },
            }}
          />
        ) : (
          <div className="phone-actions" role="toolbar" aria-label="Reply">
            {actions.includes('edit') && (
              <button
                type="button"
                className="btn btn--primary phone-actions__main"
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
                className="btn btn--primary phone-actions__main"
                onClick={() => {
                  if (account) setReplying(true);
                  else props.onReply(message, false);
                }}
              >
                <Icon name="reply" />
                Reply
              </button>
            )}
            {actions.includes('replyAll') && (
              <button
                type="button"
                className="icon-btn phone-actions__round"
                aria-label="Reply all"
                onClick={() => {
                  props.onReply(message, true);
                }}
              >
                <Icon name="replyAll" />
              </button>
            )}
            {actions.includes('forward') && (
              <button
                type="button"
                className="icon-btn phone-actions__round"
                aria-label="Forward"
                onClick={() => {
                  props.onForward(message);
                }}
              >
                <Icon name="forward" />
              </button>
            )}
          </div>
        )}
      </article>
    );
  }

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
              <span className="account-chip" title={account.email}>
                <span className="dot" style={{ background: account.color }} aria-hidden="true" />
                <span className="account-chip__name">{account.displayName || account.email}</span>
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

        {attachmentList}

        {notices}

        {!isHtml && <div className="reader__body">{textBody}</div>}
      </div>

      {isHtml && (
        // No inline min-height: the stylesheet guarantees the email its share
        // of the pane (half on desktop, most of the screen on a phone).
        <div className="reader__frame" style={{ display: 'flex', flexDirection: 'column', flex: '1 1 auto' }}>
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

interface ReplySheet {
  onClose: () => void;
  onFullEditor: () => void;
}

/**
 * A short reply without leaving the reader; it goes through the same checks
 * and Outbox as compose. On a phone it is a panel over the bottom of the
 * message (`sheet`), opened from the Reply button.
 */
function QuickReply({ message, account, sheet }: { message: MessageDetail; account: Account; sheet?: ReplySheet }) {
  const api = useMailApi();
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const sending = useSendStatus(api);
  const sendPhase = sending.status?.phase;
  /** What was typed, so Undo can put it back. */
  const lastSent = useRef('');
  const [focused, setFocused] = useState(false);
  const box = useRef<HTMLTextAreaElement>(null);
  const isSheet = sheet !== undefined;
  useEffect(() => {
    // The phone panel opens because the user tapped Reply: start typing at once.
    if (isSheet) box.current?.focus();
  }, [isSheet]);
  // "Reply sent" is a confirmation: let it fade after a moment.
  const clearSending = sending.clear;
  useEffect(() => {
    if (sendPhase !== 'sent' && sendPhase !== 'waiting') return;
    const timer = window.setTimeout(clearSending, 4000);
    return () => {
      window.clearTimeout(timer);
    };
  }, [sendPhase, clearSending]);
  const firstName = (message.from?.name ?? message.from?.address ?? '').split(' ')[0] ?? '';

  /** Undo send: take the reply back out of the Outbox and restore the text. */
  const undo = () => {
    const status = sending.status;
    if (status?.phase !== 'scheduled') return;
    sending.clear();
    api.outboxCancel(status.id).then(
      () => {
        setText(lastSent.current);
        box.current?.focus();
      },
      (e: unknown) => {
        setError(toAppError(e));
      },
    );
  };

  const submit = (event: SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!text.trim() || busy) return;
    const base = replyDraft(message, account, false);
    const draft = { ...base, bodyText: `${text.trim()}${base.bodyText}` };
    setBusy(true);
    setError(null);
    sending.clear();
    api
      .checkDraft(draft)
      .then(() => api.sendMessage(draft))
      .then(
        (entry) => {
          lastSent.current = text;
          setText('');
          sending.track(entry);
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
    // One line until used, so it never crowds out the message above it.
    <form className={sheet ? 'quick-reply quick-reply--sheet' : 'quick-reply'} onSubmit={submit}>
      {sheet && (
        <div className="quick-reply__top">
          <span className="quick-reply__to">
            To <strong>{message.from?.name ?? message.from?.address ?? 'sender'}</strong>
          </span>
          <button type="button" className="btn btn--ghost btn--small" onClick={sheet.onFullEditor}>
            Full editor
          </button>
          <button type="button" className="icon-btn" aria-label="Close reply" onClick={sheet.onClose}>
            <Icon name="close" />
          </button>
        </div>
      )}
      <label className="quick-reply__label">
        <span className="visually-hidden">Quick reply to {firstName || 'sender'}</span>
        <textarea
          className="field"
          ref={box}
          rows={sheet || focused || text ? 4 : 1}
          value={text}
          placeholder={`Reply to ${firstName || 'sender'}`}
          onFocus={() => {
            setFocused(true);
          }}
          onBlur={() => {
            setFocused(false);
          }}
          onChange={(e) => {
            setText(e.target.value);
          }}
        />
      </label>
      <button type="submit" className="btn btn--primary" disabled={busy || !text.trim()}>
        <Icon name="sent" size={16} />
        Send
      </button>
      {sending.status && (
        <p
          className={`quick-reply__status ${sending.status.phase === 'failed' ? 'form-error' : 'form-hint'}`}
          role={sending.status.phase === 'failed' ? 'alert' : 'status'}
        >
          {sending.status.phase === 'scheduled' && (
            <>
              Sending your reply…{' '}
              <button type="button" className="btn btn--link btn--small" onClick={undo}>
                Undo
              </button>
            </>
          )}
          {sending.status.phase === 'sending' && 'Sending your reply…'}
          {sending.status.phase === 'sent' && 'Reply sent.'}
          {sending.status.phase === 'waiting' && 'No connection right now. Your reply will send by itself when you are back online.'}
          {sending.status.phase === 'failed' && `Reply not sent: ${sending.status.message} It is in the Outbox.`}
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
