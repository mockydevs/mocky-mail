import { useEffect, useId, useRef, useState, type KeyboardEvent, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { errorTitle, toAppError } from '../../api/errors';
import type { Account, AppError, Draft, DraftAttachment, OutboxEntry, SignatureImage, SignatureSet } from '../../api/types';
import { formatBytes } from '../../lib/format';
import { Icon } from '../../ui/Icon';
import { Modal } from '../../ui/Modal';
import { RecipientField } from './RecipientField';
import { parseRecipients } from './recipients';
import { link, prefixLines, wrap, type Edit } from './formatting';
import { signatureBlock, swapSignature } from './seed';
import { SandboxedHtml } from '../reader/SandboxedHtml';

interface Props {
  initial: Draft;
  /** Load copies of this message's attachments into the draft (forwarding, continuing a draft). */
  attachmentsFrom?: string | null;
  title: string;
  accounts: readonly Account[];
  offlineNote: boolean;
  onClose: () => void;
  onSent: (entry: OutboxEntry) => void;
  /** Add the From account's signature (new messages, replies, forwards), and swap it with the account. */
  addSignature?: boolean;
}

type FieldName = 'to' | 'cc' | 'bcc';

function recipientError(input: string, required: boolean): string | null {
  const { valid, invalid } = parseRecipients(input);
  if (invalid.length > 0) return `Check ${invalid.join(', ')}. That does not look like an email address.`;
  if (required && valid.length === 0) return 'Add at least one recipient.';
  return null;
}

/** Rough encoded size of the attachments, for the hint only; the backend checks for real. */
function encodedEstimate(bytes: number): number {
  return Math.ceil(bytes / 3) * 4;
}

export function ComposeView({
  initial,
  attachmentsFrom = null,
  title,
  accounts,
  offlineNote,
  onClose,
  onSent,
  addSignature = false,
}: Props) {
  const api = useMailApi();
  const id = useId();
  const [draft, setDraft] = useState<Draft>(initial);
  /** The last saved (or loaded) state; changes are compared against it. */
  const [baseline, setBaseline] = useState<Draft>(initial);
  const [loadingAttachments, setLoadingAttachments] = useState(attachmentsFrom !== null);
  const [attachError, setAttachError] = useState<AppError | null>(null);
  const [showCc, setShowCc] = useState(initial.cc !== '');
  const [showBcc, setShowBcc] = useState(initial.bcc !== '');
  const [fieldErrors, setFieldErrors] = useState<Partial<Record<FieldName, string>>>({});
  const [error, setError] = useState<AppError | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [busy, setBusy] = useState<null | 'send' | 'save' | 'attach'>(null);
  const [confirmClose, setConfirmClose] = useState(false);

  useEffect(() => {
    if (attachmentsFrom === null) return;
    let active = true;
    const merge = (d: Draft, files: DraftAttachment[]): Draft => {
      const known = new Set(d.attachments.map((a) => a.path));
      return { ...d, attachments: [...d.attachments, ...files.filter((f) => !known.has(f.path))] };
    };
    api
      .attachmentsForDraft(attachmentsFrom)
      .then(
        (files) => {
          if (!active) return;
          setDraft((d) => merge(d, files));
          setBaseline((b) => merge(b, files));
        },
        (e: unknown) => {
          if (active) setAttachError(toAppError(e));
        },
      )
      .finally(() => {
        if (active) setLoadingAttachments(false);
      });
    return () => {
      active = false;
    };
  }, [api, attachmentsFrom]);

  // The signature now in the body, so changing From or the picker can swap it.
  const signature = useRef<string | null>(null);
  const signatureLoaded = useRef(false);
  // The From account's signatures, for the picker.
  const [signatures, setSignatures] = useState<SignatureSet | null>(null);
  const [signatureId, setSignatureId] = useState<string | null>(null);
  // Its picture, previewed under the body; it is added when the message is sent.
  const [signatureImage, setSignatureImage] = useState<{ text: string; image: SignatureImage } | null>(null);
  // Replies and forwards start with the reply signature.
  const isReply = initial.inReplyTo !== null || attachmentsFrom !== null;

  /** Put signature `id` (null: none) in the body in place of the current one. */
  const applySignature = (set: SignatureSet, id: string | null, atStart: boolean) => {
    const next = set.items.find((s) => s.id === id) ?? null;
    const previous = signature.current;
    const text = next && next.text !== '' ? next.text : null;
    signature.current = text;
    setSignatureId(next?.id ?? null);
    setSignatureImage(next?.image && text !== null ? { text, image: next.image } : null);
    setDraft((d) => ({ ...d, bodyText: swapSignature(d.bodyText, previous, text) }));
    // Adding it at the start isn't a change the user made.
    if (atStart) setBaseline((b) => ({ ...b, bodyText: swapSignature(b.bodyText, previous, text) }));
  };

  useEffect(() => {
    if (!addSignature) return;
    let active = true;
    api.listSignatures(draft.accountId).then(
      (set) => {
        if (!active) return;
        setSignatures(set);
        applySignature(set, isReply ? set.defaultReply : set.defaultNew, !signatureLoaded.current);
        signatureLoaded.current = true;
      },
      () => undefined,
    );
    return () => {
      active = false;
    };
    // applySignature only reads refs and setters; reloading on its identity would loop.
  }, [api, addSignature, draft.accountId, isReply]);

  const dirty = JSON.stringify(draft) !== JSON.stringify(baseline);
  const account = accounts.find((a) => a.id === draft.accountId);
  const attachmentBytes = draft.attachments.reduce((sum, a) => sum + a.size, 0);
  const overLimit =
    account?.sizeLimitBytes != null && encodedEstimate(attachmentBytes) > account.sizeLimitBytes;

  const update = (patch: Partial<Draft>) => {
    setDraft((d) => ({ ...d, ...patch }));
    setStatus(null);
  };

  const validate = (): boolean => {
    const errors: Partial<Record<FieldName, string>> = {};
    const to = recipientError(draft.to, draft.cc.trim() === '' && draft.bcc.trim() === '');
    const cc = recipientError(draft.cc, false);
    const bcc = recipientError(draft.bcc, false);
    if (to) errors.to = to;
    if (cc) errors.cc = cc;
    if (bcc) errors.bcc = bcc;
    setFieldErrors(errors);
    return Object.keys(errors).length === 0;
  };

  const send = (event: SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError(null);
    if (!validate()) return;
    setBusy('send');
    api
      .checkDraft(draft)
      .then(() => api.sendMessage(draft))
      .then(onSent, (e: unknown) => {
        setError(toAppError(e));
        setBusy(null);
      });
  };

  /** Save and remember the saved copy's id, so the next save or send replaces it. */
  const saveNow = (): Promise<void> => {
    const snapshot = draft;
    return api.saveDraft(snapshot).then(({ draftMessageId }) => {
      setDraft((d) => ({ ...d, draftMessageId }));
      setBaseline({ ...snapshot, draftMessageId });
    });
  };

  const save = () => {
    setError(null);
    setBusy('save');
    saveNow().then(
      () => {
        setBusy(null);
        setStatus('Draft saved.');
      },
      (e: unknown) => {
        setError(toAppError(e));
        setBusy(null);
      },
    );
  };

  const attach = () => {
    setError(null);
    setBusy('attach');
    api.pickAttachments().then(
      (files) => {
        setDraft((d) => {
          const known = new Set(d.attachments.map((a) => a.path));
          return { ...d, attachments: [...d.attachments, ...files.filter((f) => !known.has(f.path))] };
        });
        setBusy(null);
      },
      (e: unknown) => {
        setError(toAppError(e));
        setBusy(null);
      },
    );
  };

  const requestClose = () => {
    if (dirty) setConfirmClose(true);
    else onClose();
  };

  // Formatting: marks typed or added by the toolbar; Preview shows the result.
  const bodyRef = useRef<HTMLTextAreaElement>(null);
  const [preview, setPreview] = useState<string | null>(null);
  const format = (make: (value: string, start: number, end: number) => Edit) => {
    const box = bodyRef.current;
    if (!box) return;
    const edit = make(box.value, box.selectionStart, box.selectionEnd);
    update({ bodyText: edit.value });
    // After React writes the new value, put the cursor where the edit says.
    requestAnimationFrame(() => {
      box.focus();
      box.setSelectionRange(edit.start, edit.end);
    });
  };
  const tools: { label: string; key?: string; text: string; make: (v: string, s: number, e: number) => Edit }[] = [
    { label: 'Bold', key: 'b', text: 'B', make: (v, s, e) => wrap(v, s, e, '**', 'bold text') },
    { label: 'Italic', key: 'i', text: 'I', make: (v, s, e) => wrap(v, s, e, '*', 'italic text') },
    { label: 'Link', key: 'k', text: 'Link', make: link },
    { label: 'Bulleted list', text: '•', make: (v, s, e) => prefixLines(v, s, e, '- ') },
    { label: 'Numbered list', text: '1.', make: (v, s, e) => prefixLines(v, s, e, '', true) },
    { label: 'Quote', text: '❝', make: (v, s, e) => prefixLines(v, s, e, '> ') },
    { label: 'Code', text: '</>', make: (v, s, e) => wrap(v, s, e, '`', 'code') },
  ];
  const togglePreview = () => {
    if (preview !== null) {
      setPreview(null);
      return;
    }
    api.previewBody(draft.bodyText).then(setPreview, (e: unknown) => {
      setError(toAppError(e));
    });
  };

  const onKeyDown = (event: KeyboardEvent) => {
    const shortcut =
      (event.ctrlKey || event.metaKey) && !event.altKey && event.target === bodyRef.current
        ? tools.find((t) => t.key === event.key.toLowerCase())
        : undefined;
    if (shortcut) {
      event.preventDefault();
      format(shortcut.make);
      return;
    }
    if (event.key === 'Escape' && !confirmClose) {
      event.stopPropagation();
      requestClose();
    } else if (event.key === 'Enter' && (event.ctrlKey || event.metaKey) && busy === null && !loadingAttachments) {
      event.preventDefault();
      (event.currentTarget as HTMLFormElement).requestSubmit();
    }
  };

  const field = (name: FieldName, label: string, autoFocus: boolean) => {
    const fieldId = `${id}-${name}`;
    const message = fieldErrors[name];
    return (
      <div className="compose__row">
        <label htmlFor={fieldId} className="compose__label">
          {label}
        </label>
        <div className="compose__field">
          <RecipientField
            id={fieldId}
            label={label}
            value={draft[name]}
            invalid={message !== undefined}
            describedBy={message ? `${fieldId}-error` : undefined}
            // The first field of a view the user just opened.
            // eslint-disable-next-line jsx-a11y/no-autofocus
            autoFocus={autoFocus}
            onChange={(value) => {
              update({ [name]: value });
            }}
          />
          {message && (
            <p id={`${fieldId}-error`} className="compose__error" role="alert">
              {message}
            </p>
          )}
        </div>
        {name === 'to' && (!showCc || !showBcc) && (
          <span className="compose__copies">
            {!showCc && (
              <button
                type="button"
                className="btn btn--link"
                aria-label="Add Cc"
                onClick={() => {
                  setShowCc(true);
                }}
              >
                Cc
              </button>
            )}
            {!showBcc && (
              <button
                type="button"
                className="btn btn--link"
                aria-label="Add Bcc"
                onClick={() => {
                  setShowBcc(true);
                }}
              >
                Bcc
              </button>
            )}
          </span>
        )}
      </div>
    );
  };

  return (
    // The form handles Esc for its own fields; it is not an interactive control itself.
    // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions
    <form className="compose" aria-labelledby={`${id}-title`} onSubmit={send} onKeyDown={onKeyDown} noValidate>
      <div className="compose__card">
      <header className="compose__head">
        <h1 id={`${id}-title`} className="compose__title">
          {title}
        </h1>
        {status && (
          <span className="muted small" role="status">
            {status}
          </span>
        )}
        <button type="button" className="icon-btn compose__close" aria-label="Close" onClick={requestClose}>
          <Icon name="close" />
        </button>
      </header>

      <div className="compose__fields">
        <div className="compose__row">
          <label htmlFor={`${id}-from`} className="compose__label">
            From
          </label>
          <select
            id={`${id}-from`}
            className="compose__from"
            value={draft.accountId}
            onChange={(e) => {
              update({ accountId: e.target.value });
            }}
          >
            {accounts.map((a) => (
              <option key={a.id} value={a.id}>
                {a.displayName && a.displayName !== a.email ? `${a.displayName} <${a.email}>` : a.email}
              </option>
            ))}
          </select>
        </div>
        {field('to', 'To', initial.to === '')}
        {showCc && field('cc', 'Cc', false)}
        {showBcc && field('bcc', 'Bcc', false)}
        <div className="compose__row">
          <label htmlFor={`${id}-subject`} className="compose__label">
            Subject
          </label>
          <input
            id={`${id}-subject`}
            type="text"
            className="compose__input compose__input--subject"
            value={draft.subject}
            onChange={(e) => {
              update({ subject: e.target.value });
            }}
          />
        </div>
      </div>

      <div className="compose__tools">
        <button type="button" className="btn btn--ghost" disabled={busy !== null} onClick={attach}>
          <Icon name="paperclip" />
          Attach files
        </button>
        {signatures && signatures.items.length > 0 && (
          <label className="compose__signature-pick">
            <span>Signature</span>
            <select
              className="field"
              value={signatureId ?? ''}
              onChange={(e) => {
                applySignature(signatures, e.target.value || null, false);
              }}
            >
              {signatures.items.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
              <option value="">None</option>
            </select>
          </label>
        )}
        <span className="compose__hint">Ctrl+Enter to send</span>
      </div>

      <div className="compose__format" role="toolbar" aria-label="Formatting">
        {tools.map((tool) => (
          <button
            key={tool.label}
            type="button"
            className="compose__format-btn"
            style={tool.label === 'Italic' ? { fontStyle: 'italic', fontFamily: 'Georgia, serif' } : undefined}
            aria-label={tool.label}
            title={tool.key ? `${tool.label} (Ctrl+${tool.key.toUpperCase()})` : tool.label}
            disabled={preview !== null}
            onClick={() => {
              format(tool.make);
            }}
          >
            {tool.text}
          </button>
        ))}
        <button
          type="button"
          className="compose__format-btn compose__format-preview"
          aria-pressed={preview !== null}
          onClick={togglePreview}
        >
          {preview !== null ? 'Edit' : 'Preview'}
        </button>
      </div>

      {preview !== null && (
        <div className="compose__preview">
          <SandboxedHtml title="Preview of your message" html={preview} fitContent />
        </div>
      )}
      <label htmlFor={`${id}-body`} className="visually-hidden">
        Message
      </label>
      <textarea
        ref={bodyRef}
        id={`${id}-body`}
        hidden={preview !== null}
        className="compose__body"
        value={draft.bodyText}
        // Replies start in the body, since To is already filled.
        // eslint-disable-next-line jsx-a11y/no-autofocus
        autoFocus={initial.to !== ''}
        onFocus={(e) => {
          if (initial.to !== '' && e.currentTarget.selectionStart === e.currentTarget.value.length) {
            e.currentTarget.setSelectionRange(0, 0);
          }
        }}
        onChange={(e) => {
          update({ bodyText: e.target.value });
        }}
      />

      {signatureImage && draft.bodyText.includes(signatureBlock(signatureImage.text)) && (
        <figure className="compose__signature">
          <img
            src={`data:${signatureImage.image.mimeType};base64,${signatureImage.image.data}`}
            alt="Your signature"
            style={{ width: Math.min(signatureImage.image.width, 240) }}
          />
          <figcaption className="form-hint">Your signature picture is added under your signature when sent.</figcaption>
        </figure>
      )}

      {loadingAttachments && (
        <p className="compose__loading form-hint" role="status" aria-busy="true">
          Adding the original attachments
        </p>
      )}
      {attachError && (
        <div className="notice notice--error compose__attach-error" role="alert">
          <p className="notice__title">Couldn’t add the original attachments</p>
          <p>{attachError.message} You can still send the message, or add files with Attach files.</p>
        </div>
      )}

      {draft.attachments.length > 0 && (
        <div className="compose__attachments">
          <ul className="attachments" aria-label="Attached files">
            {draft.attachments.map((file) => (
              <li key={file.path} className="attachment">
                <span className="attachment__text">
                  <span className="attachment__name">{file.fileName}</span>
                  <span className="muted small">{formatBytes(file.size)}</span>
                </span>
                <button
                  type="button"
                  className="icon-btn"
                  aria-label={`Remove ${file.fileName}`}
                  onClick={() => {
                    update({ attachments: draft.attachments.filter((a) => a.path !== file.path) });
                  }}
                >
                  <Icon name="close" />
                </button>
              </li>
            ))}
          </ul>
          <p className={overLimit ? 'form-error' : 'form-hint'}>
            Attachments {formatBytes(attachmentBytes)}
            {account?.sizeLimitBytes != null &&
              `. ${account.email} accepts messages up to ${formatBytes(account.sizeLimitBytes)} once encoded`}
            {overLimit && '. This is probably too large to send.'}
          </p>
        </div>
      )}

      {error && (
        <div className="notice notice--error" role="alert">
          <p className="notice__title">{errorTitle(error.kind)}</p>
          <p>{error.message}</p>
        </div>
      )}

      <footer className="compose__foot">
        <button type="submit" className="btn btn--primary btn--large" disabled={busy !== null || loadingAttachments}>
          <Icon name="sent" size={16} />
          {busy === 'send' ? 'Sending' : 'Send'}
        </button>
        <button type="button" className="btn btn--secondary" disabled={busy !== null} onClick={save}>
          {busy === 'save' ? 'Saving' : 'Save draft'}
        </button>
        {offlineNote && (
          <span className="compose__note">
            <Icon name="clock" size={16} />
            No connection right now. It waits in the Outbox and sends by itself.
          </span>
        )}
        <span className="statusbar__spacer" />
        <button type="button" className="icon-btn" aria-label="Discard" onClick={requestClose}>
          <Icon name="trash" />
        </button>
      </footer>
      </div>

      {confirmClose && (
        <Modal
          title="Discard this message?"
          description="You have changes that are not saved."
          size="small"
          onClose={() => {
            setConfirmClose(false);
          }}
        >
          <div className="modal__actions">
            <button
              type="button"
              className="btn btn--secondary"
              onClick={() => {
                setConfirmClose(false);
              }}
            >
              Keep editing
            </button>
            <button
              type="button"
              className="btn btn--secondary"
              onClick={() => {
                setConfirmClose(false);
                saveNow().then(onClose, (e: unknown) => {
                  setError(toAppError(e));
                });
              }}
            >
              Save draft
            </button>
            <button type="button" className="btn btn--danger" onClick={onClose}>
              Discard
            </button>
          </div>
        </Modal>
      )}
    </form>
  );
}
