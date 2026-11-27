import { forwardRef, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { errorTitle, toAppError } from '../../api/errors';
import type {
  Account,
  AppError,
  AttachmentMeta,
  Category,
  Focus,
  FolderRole,
  ImageSender,
  Importance,
  MessageDetail,
  MessageListItem,
} from '../../api/types';
import { formatBytes, formatListTime, formatLongDate, nowSeconds } from '../../lib/format';
import { useLoad } from '../../lib/useLoad';
import type { Theme } from '../../theme/useTheme';
import { Avatar } from '../../ui/Avatar';
import { Icon, type IconName } from '../../ui/Icon';
import { CategoryMenu } from '../categories/CategoryMenu';
import { FlagMenu } from '../mailbox/FlagMenu';
import { dueLabel, isOverdue } from '../mailbox/flagDue';
import { colourOf } from '../mailbox/MessageList';
import type { FlagValue } from '../mailbox/useMessageActions';
import { formatMailbox, formatMailboxes, replyDraft, toolbarActions, withSignature } from '../compose/seed';
import { useSendStatus } from '../outbox/useSendStatus';
import { printMessage } from '../../lib/print';
import { Modal } from '../../ui/Modal';
import { AttachmentViewer } from './AttachmentViewer';
import { linkify } from './linkify';
import { SandboxedHtml } from './SandboxedHtml';
import { InvitationCard } from '../calendar/InvitationCard';
import { ThreadHistory } from './ThreadHistory';
import { ReadReceiptBar } from './ReadReceiptBar';
import { MessageSecurity } from './MessageSecurity';

export interface ReaderActions {
  onReply: (message: MessageDetail, all: boolean) => void;
  onForward: (message: MessageDetail) => void;
  onEdit: (message: MessageDetail) => void;
  onArchive: () => void;
  onDelete: () => void;
  onDeleteForever: () => void;
  onToggleFlag: () => void;
  onToggleRead: () => void;
  onTogglePin?: () => void;
  /** Flag with a due date, without one, or clear the flag. */
  onFlagDue?: (flag: FlagValue) => void;
  onCategory?: (name: string, add: boolean) => void;
  onCategoryCreated?: (categories: Category[], name: string) => void;
  onImportance?: (importance: Importance) => void;
  /** true: Junk; false: Not junk. */
  onJunk?: (junk: boolean) => void;
  /** Computers: the message in a window of its own. */
  onOpenWindow?: (() => void) | undefined;
  /** "Create rule…": a new rule started from this message. */
  onCreateRule?: ((message: MessageDetail) => void) | undefined;
}

interface Props extends ReaderActions {
  /** The selected list item, with optimistic changes applied. */
  item: MessageListItem | null;
  /**
   * The message to show when it is not the row's own: the one the user opened
   * in a conversation whose row has since been renamed by a newer reply.
   */
  messageId?: string | null;
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
  /** Focused Inbox: the half this message is shown in, or null when the inbox isn't split. */
  focusTab?: Focus | null;
  /** Always put this sender's mail in the other half. */
  onFocusRule?: (sender: string, focus: Focus) => void;
  /** Colours for category chips and the Categorise menu. */
  categories?: readonly Category[];
  /** Unix seconds, for due dates. */
  now?: number;
  /** Computers with the reading pane off: a Back button returns to the list. */
  showBack?: boolean;
  /** Work Offline is on: a message not yet on this device can't be fetched. */
  workOffline?: boolean;
  /** Settings › Reading: what to do when a message asks for a read receipt. Default: ask. */
  readReceipts?: 'ask' | 'always' | 'never';
  /** Open Settings › Security (a locked message needs a key added there). */
  onOpenSecurity?: (() => void) | undefined;
}

/** Free mail services: trusting the whole domain would trust everyone on it. */
const PUBLIC_DOMAINS = new Set([
  'gmail.com',
  'googlemail.com',
  'outlook.com',
  'hotmail.com',
  'live.com',
  'yahoo.com',
  'icloud.com',
  'me.com',
  'aol.com',
  'proton.me',
  'protonmail.com',
  'gmx.com',
  'mail.com',
]);

/** The trusted entry that lets this address's pictures load: the address itself, or its domain. */
function trustedEntry(senders: readonly ImageSender[], address: string): ImageSender | undefined {
  const a = address.toLowerCase();
  const domain = a.slice(a.lastIndexOf('@') + 1);
  return (
    senders.find((s) => !s.wholeDomain && s.sender === a) ??
    senders.find((s) => s.wholeDomain && (domain === s.sender || domain.endsWith(`.${s.sender}`)))
  );
}

const IMPORTANCE_ACTIONS: readonly [Importance, string][] = [
  ['high', 'Mark as high importance'],
  ['normal', 'Mark as normal importance'],
  ['low', 'Mark as low importance'],
];

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
  // Keyed by the message shown, not the row: a reply arriving in the open
  // conversation must not start the reader (and a reply being typed) over.
  const id = props.messageId ?? item.id;
  return <MessageView key={id} {...props} item={item} id={id} ref={ref} />;
});

const MessageView = forwardRef<HTMLElement, Props & { item: MessageListItem; id: string }>(function MessageView(props, ref) {
  const api = useMailApi();
  const { item, id, account, role, alwaysLoadImages, theme, onLoaded } = props;
  const [showImages, setShowImages] = useState(false);
  // Per message: dark mode can be switched off for one email that reads badly inverted.
  const [originalColours, setOriginalColours] = useState(false);
  const allowRemote = alwaysLoadImages || showImages;
  /** Bumped when the sender is trusted or no longer trusted: the message loads again. */
  const [trustRevision, setTrustRevision] = useState(0);
  // With or without remote images it is the same message: the image variant
  // loads as a new revision, so the message stays on screen meanwhile and a
  // quick reply being typed (or an opened earlier message) is not thrown away.
  const loaded = useLoad(`message:${id}`, Number(allowRemote) + 2 * trustRevision, () => api.getMessage(id, allowRemote));
  // Tell the shell what is on screen once it has settled. From an effect, so
  // a slow answer for a message the user has already left is never reported;
  // a layout effect, so the shell knows before a key (r, a, f) can be pressed.
  const settled = loaded.status === 'ready' ? loaded.value : null;
  useLayoutEffect(() => {
    onLoaded(settled);
  }, [settled, onLoaded]);
  // Pictures that load because the sender is trusted: which entry did it, for "Stop".
  const trustedNow = settled?.imagesTrusted === true;
  const trustList = useLoad(`image-senders:${id}:${String(trustedNow)}`, trustRevision, () =>
    trustedNow ? api.listImageSenders() : Promise.resolve<ImageSender[]>([]),
  );
  const [notice, setNotice] = useState<{ error: AppError | null; text: string | null }>({ error: null, text: null });
  // The attachment shown full screen, if any.
  const [viewing, setViewing] = useState<AttachmentMeta | null>(null);
  // "View source": the message as received.
  const [source, setSource] = useState<string | null>(null);
  // Desktop "More actions" menu.
  const [more, setMore] = useState(false);
  // Phone layout only.
  const [details, setDetails] = useState(false);
  const [menu, setMenu] = useState(false);
  const [replying, setReplying] = useState(false);
  const [pastTitle, setPastTitle] = useState(false);

  if (loaded.status === 'error') {
    return (
      <article className="reader reader--empty" aria-label="Reading pane" ref={ref} tabIndex={-1}>
        <p className="empty__title">{props.workOffline ? "You're working offline" : 'This message could not be opened'}</p>
        <p className="empty__text">
          {props.workOffline
            ? "This message isn't on this device yet. Turn off Work Offline to download it."
            : loaded.message}
        </p>
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
        if (path !== null) setNotice({ error: null, text: `Saved: ${path}` });
      },
      (e: unknown) => {
        setNotice({ error: toAppError(e), text: null });
      },
    );
  };

  const openInApp = (index: number) => {
    setNotice({ error: null, text: null });
    api.openAttachment(message.id, index).catch((e: unknown) => {
      setNotice({ error: toAppError(e), text: null });
    });
  };

  // Tap: photos, PDFs and text open in the preview; anything else goes
  // straight to the app the device uses for it (Word, Excel, zip…).
  const view = (file: AttachmentMeta) => {
    if (file.preview === null) openInApp(file.index);
    else {
      setNotice({ error: null, text: null });
      setViewing(file);
    }
  };

  const attachmentList = message.attachments.length > 0 && (
    <ul className="attachments" aria-label="Attachments">
      {message.attachments.map((file) => (
        <li key={file.index} className="attachment">
          <button
            type="button"
            className="attachment__open"
            aria-label={file.preview === null ? `Open ${file.fileName} in another app` : `Preview ${file.fileName}`}
            onClick={() => {
              view(file);
            }}
          >
            <span className="attachment__icon" aria-hidden="true">
              {fileBadge(file.fileName)}
            </span>
            <span className="attachment__text">
              <span className="attachment__name">{file.fileName}</span>
              <span className="muted small">{formatBytes(file.size)}</span>
            </span>
          </button>
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

  const viewer = viewing && (
    <AttachmentViewer
      messageId={message.id}
      file={viewing}
      onClose={() => {
        setViewing(null);
      }}
      onSave={() => {
        save(viewing.index);
      }}
      onOpen={() => {
        openInApp(viewing.index);
      }}
      notice={notice}
    />
  );

  const failed = (e: unknown) => {
    setNotice({ error: toAppError(e), text: null });
  };

  // Pictures: always from this sender or domain, or no longer.
  const fromAddress = message.from?.address ?? '';
  const fromDomain = fromAddress.includes('@') ? fromAddress.slice(fromAddress.lastIndexOf('@') + 1).toLowerCase() : '';
  const trusted = message.imagesTrusted === true;
  const remoteShown = allowRemote || trusted;
  const trustedBy = trusted
    ? (trustedEntry(trustList.status === 'ready' ? trustList.value : [], fromAddress) ?? {
        sender: fromAddress.toLowerCase(),
        wholeDomain: false,
      })
    : null;
  const trust = (wholeDomain: boolean) => {
    setNotice({ error: null, text: null });
    api.trustImageSender(fromAddress, wholeDomain).then(() => {
      setTrustRevision((r) => r + 1);
    }, failed);
  };
  const untrust = () => {
    if (!trustedBy) return;
    setNotice({ error: null, text: null });
    api.untrustImageSender(trustedBy.sender).then(() => {
      setShowImages(false);
      setTrustRevision((r) => r + 1);
    }, failed);
  };
  const trustButtons = fromAddress.includes('@') && !outgoing && (
    <>
      <button
        type="button"
        className="btn btn--ghost btn--small"
        onClick={() => {
          trust(false);
        }}
      >
        Always show from {fromAddress}
      </button>
      {fromDomain && !PUBLIC_DOMAINS.has(fromDomain) && (
        <button
          type="button"
          className="btn btn--ghost btn--small"
          onClick={() => {
            trust(true);
          }}
        >
          Always show from {fromDomain}
        </button>
      )}
    </>
  );
  const trustedNote = trustedBy && isHtml && (
    <p className="images-note">
      <Icon name="image" size={16} />
      <span>
        Pictures shown because you trust {trustedBy.wholeDomain ? `everyone at ${trustedBy.sender}` : trustedBy.sender} ·{' '}
      </span>
      <button
        type="button"
        className="btn btn--link btn--small"
        aria-label={`Stop showing pictures from ${trustedBy.sender} automatically`}
        onClick={untrust}
      >
        Stop
      </button>
    </p>
  );

  // Whole-message actions, in the More menu on both layouts.
  const senderAddress = message.from?.address;
  const moveTo: Focus | null = props.focusTab === 'focused' ? 'other' : props.focusTab === 'other' ? 'focused' : null;
  const now = props.now ?? nowSeconds();
  const junkAction = role === 'junk' ? 'notJunk' : role === 'drafts' || role === 'sent' ? null : 'junk';
  const extras: { key: string; label: string; icon: IconName; run: () => void }[] = [
    ...(props.onOpenWindow
      ? [
          {
            key: 'window',
            label: 'Open in new window',
            icon: 'external' as const,
            run: () => {
              props.onOpenWindow?.();
            },
          },
        ]
      : []),
    ...(moveTo && senderAddress && props.onFocusRule
      ? [
          {
            key: 'focus',
            label: moveTo === 'other' ? 'Always move this sender to Other' : 'Always move this sender to Focused',
            icon: 'inbox' as const,
            run: () => {
              props.onFocusRule?.(senderAddress, moveTo);
            },
          },
        ]
      : []),
    {
      key: 'print',
      label: 'Print',
      icon: 'print',
      run: () => {
        printMessage(api, message, remoteShown).catch(failed);
      },
    },
    {
      key: 'eml',
      label: 'Save as .eml',
      icon: 'download',
      run: () => {
        setNotice({ error: null, text: null });
        api.saveMessage(message.id).then((path) => {
          if (path !== null) setNotice({ error: null, text: `Saved: ${path}` });
        }, failed);
      },
    },
    {
      key: 'source',
      label: 'View source',
      icon: 'code',
      run: () => {
        api.messageSource(message.id).then(setSource, failed);
      },
    },
    ...(props.onCreateRule && role !== 'drafts' && role !== 'sent' && !outgoing
      ? [
          {
            key: 'rule',
            label: 'Create rule…',
            icon: 'move' as const,
            run: () => {
              props.onCreateRule?.(message);
            },
          },
        ]
      : []),
    ...(props.onImportance && role !== 'drafts'
      ? IMPORTANCE_ACTIONS.filter(([value]) => value !== item.importance).map(([value, label]) => ({
          key: `importance-${value}`,
          label,
          icon: value === 'low' ? ('arrowDown' as const) : ('alert' as const),
          run: () => {
            props.onImportance?.(value);
          },
        }))
      : []),
  ];

  /** Importance, a flag's due date and the categories, under the sender. */
  const labels = (item.importance !== 'normal' ||
    (item.flagged && item.flagDue !== null) ||
    item.categories.length > 0) && (
    <div className="reader__labels">
      {item.importance === 'high' && <span className="badge badge--high">High importance</span>}
      {item.importance === 'low' && <span className="badge badge--low">Low importance</span>}
      {item.flagged && item.flagDue !== null && (
        <span className={`badge badge--due${isOverdue(item.flagDue, now) ? ' badge--overdue' : ''}`}>
          <Icon name="flag" size={14} filled />
          {dueLabel(item.flagDue, now)}
        </span>
      )}
      {item.categories.length > 0 && (
        <ul className="reader__categories" aria-label="Categories">
          {item.categories.map((c) => (
            <li key={c} className="chip" style={{ '--chip': colourOf(props.categories, c) } as CSSProperties}>
              {c}
            </li>
          ))}
        </ul>
      )}
    </div>
  );

  const categoryMenu = props.onCategory && props.onCategoryCreated && role !== 'drafts' && (
    <CategoryMenu
      categories={props.categories ?? []}
      rows={[item]}
      onToggle={props.onCategory}
      onCreated={props.onCategoryCreated}
    />
  );
  const flagMenu = props.onFlagDue && actions.includes('flag') && (
    <FlagMenu flagged={item.flagged} due={item.flagDue} now={now} onFlag={props.onFlagDue} />
  );

  const sourceDialog = source !== null && (
    <Modal
      title="Message source"
      description="The message exactly as it arrived: headers first, then the body."
      onClose={() => {
        setSource(null);
      }}
    >
      <pre className="source-view">{source}</pre>
      <div className="modal__actions">
        <button
          type="button"
          className="btn btn--secondary"
          onClick={() => {
            void navigator.clipboard.writeText(source).catch(() => undefined);
          }}
        >
          Copy
        </button>
        <button
          type="button"
          className="btn btn--primary"
          onClick={() => {
            setSource(null);
          }}
        >
          Close
        </button>
      </div>
    </Modal>
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

  // Signature and encryption: what checking them found, above the message.
  const security = (
    <MessageSecurity
      message={message}
      onChanged={() => {
        setTrustRevision((r) => r + 1);
      }}
      onOpenSecurity={props.onOpenSecurity}
    />
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
            title={part.url}
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
    const blocked = isHtml && message.blockedRemoteImages > 0 && !remoteShown;
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
          {flagMenu}
          {categoryMenu}
          {
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
                  {props.onTogglePin && role !== 'drafts' && (
                    <button
                      type="button"
                      className="phone-menu__item"
                      onClick={() => {
                        setMenu(false);
                        props.onTogglePin?.();
                      }}
                    >
                      <Icon name="pin" filled={item.pinned} />
                      {item.pinned ? 'Unpin' : 'Pin'}
                    </button>
                  )}
                  {props.onJunk && junkAction && (
                    <button
                      type="button"
                      className="phone-menu__item"
                      onClick={() => {
                        setMenu(false);
                        props.onJunk?.(junkAction === 'junk');
                      }}
                    >
                      <Icon name="junk" />
                      {junkAction === 'junk' ? 'Junk' : 'Not junk'}
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
                  {extras.map((extra) => (
                    <button
                      key={extra.key}
                      type="button"
                      className="phone-menu__item"
                      onClick={() => {
                        setMenu(false);
                        extra.run();
                      }}
                    >
                      <Icon name={extra.icon} />
                      {extra.label}
                    </button>
                  ))}
                </div>
              )}
            </div>
          }
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
                  {toLabel} · {formatListTime(message.date, now)}
                  <Icon name="chevron" size={14} />
                </button>
              </div>
            </div>
            {labels}
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
            {security}
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
                {trustButtons}
              </div>
            )}
            {trustedNote}
            {message.invitation && <InvitationCard key={message.id} messageId={message.id} initial={message.invitation} />}
            {attachmentList}
            {viewer}
            {sourceDialog}
            <ReadReceiptBar message={message} mode={props.readReceipts ?? 'ask'} />
            {notices}
            <ThreadHistory messageId={message.id} theme={theme} />
          </div>
          {isHtml ? (
            <SandboxedHtml
              title={`Message: ${message.subject}`}
              html={message.html ?? ''}
              theme={originalColours ? 'light' : theme}
              allowRemoteImages={remoteShown}
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
        {props.showBack && (
          <button type="button" className="icon-btn" aria-label="Back to list" title="Back to list (Esc)" onClick={props.onBack}>
            <Icon name="back" />
          </button>
        )}
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
        {props.onJunk && junkAction && (
          <button
            type="button"
            className={junkAction === 'junk' ? 'icon-btn' : 'btn btn--ghost'}
            aria-label={junkAction === 'junk' ? 'Junk' : 'Not junk'}
            title={junkAction === 'junk' ? 'Junk: move to Spam' : 'Not junk: back to the Inbox'}
            onClick={() => {
              props.onJunk?.(junkAction === 'junk');
            }}
          >
            <Icon name="junk" />
            {junkAction === 'notJunk' && 'Not junk'}
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
        {flagMenu}
        {props.onTogglePin && role !== 'drafts' && (
          <button
            type="button"
            className="icon-btn"
            aria-label={item.pinned ? 'Unpin' : 'Pin'}
            title={item.pinned ? 'Unpin' : 'Pin to the top of the folder'}
            aria-pressed={item.pinned}
            onClick={props.onTogglePin}
          >
            <Icon name="pin" filled={item.pinned} />
          </button>
        )}
        {categoryMenu}
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
        <div className="phone-menu toolbar__more">
          <button
            type="button"
            className="icon-btn"
            aria-label="More actions"
            aria-expanded={more}
            title="Print, save, view source"
            onClick={() => {
              setMore((on) => !on);
            }}
          >
            <Icon name="more" />
          </button>
          {more && (
            <div className="phone-menu__list">
              {extras.map((extra) => (
                <button
                  key={extra.key}
                  type="button"
                  className="phone-menu__item"
                  onClick={() => {
                    setMore(false);
                    extra.run();
                  }}
                >
                  <Icon name={extra.icon} />
                  {extra.label}
                </button>
              ))}
            </div>
          )}
        </div>
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
            {labels}
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

        {security}

        {message.html !== null && message.blockedRemoteImages > 0 && !remoteShown && (
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
            {trustButtons}
          </div>
        )}
        {trustedNote}

        {message.invitation && <InvitationCard key={message.id} messageId={message.id} initial={message.invitation} />}
        {attachmentList}
        {viewer}
        {sourceDialog}

        <ReadReceiptBar message={message} mode={props.readReceipts ?? 'ask'} />
        {notices}

        <ThreadHistory messageId={message.id} theme={theme} />

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
            allowRemoteImages={remoteShown}
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
    setBusy(true);
    setError(null);
    sending.clear();
    let draft = base;
    api
      .getSignature(account.id, true)
      // A quick reply carries the signature like any other reply.
      .catch(() => null)
      .then((signature) => {
        draft = { ...base, bodyText: `${text.trim()}${withSignature(base.bodyText, signature?.text ?? null)}` };
        return api.checkDraft(draft);
      })
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
