import { useEffect, useId, useRef, useState, type ClipboardEvent, type DragEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { AppError, NamedSignature, SignatureImage, SignatureSet } from '../../api/types';
import { ConfirmDialog } from '../../ui/Modal';

/**
 * An account's signatures: several named ones (for example "Full" and
 * "Short"), each with words and an optional picture, and which one new
 * messages and which one replies and forwards start with.
 */
export function SignatureEditor({ accountId }: { accountId: string }) {
  const api = useMailApi();
  const id = useId();
  const [set, setSet] = useState<SignatureSet | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let active = true;
    api.listSignatures(accountId).then(
      (loaded) => {
        if (active) setSet(loaded);
      },
      (e: unknown) => {
        if (active) setError(toAppError(e));
      },
    );
    return () => {
      active = false;
    };
  }, [api, accountId]);

  /** Run a change; show what was stored. `null` result: cancelled. */
  const change = (run: () => Promise<SignatureSet | null>, success: string) => {
    setBusy(true);
    setError(null);
    setDone(null);
    run().then(
      (next) => {
        setBusy(false);
        if (next) {
          setSet(next);
          setDone(success);
        }
      },
      (e: unknown) => {
        setBusy(false);
        setError(toAppError(e));
      },
    );
  };

  const options = (set?.items ?? []).map((s) => (
    <option key={s.id} value={s.id}>
      {s.name}
    </option>
  ));

  return (
    <div className="signature-editor">
      <p className="form-field__label">Signatures</p>
      <p className="form-hint">
        Added to new messages, replies and forwards. Type one or paste it from your webmail. To use a signature you
        designed (for example in Illustrator), export it as a PNG or JPG and add it as a picture. Make more than one
        (say, Full and Short) and pick which to use while writing.
      </p>

      {set?.items.map((signature) => (
        <OneSignature
          key={signature.id}
          accountId={accountId}
          signature={signature}
          busy={busy}
          onChange={change}
        />
      ))}

      <button
        type="button"
        className="btn btn--secondary"
        disabled={busy || set === null}
        onClick={() => {
          change(
            () => api.saveSignature(accountId, null, '', ''),
            'New signature added. Type it below, or add a picture.',
          );
        }}
      >
        {set?.items.length ? 'Add another signature' : 'Add a signature'}
      </button>

      {set && set.items.length > 0 && (
        <div className="signature-editor__defaults">
          <label className="form-field">
            <span className="form-field__label" id={`${id}-new`}>
              For new messages
            </span>
            <select
              className="field"
              aria-labelledby={`${id}-new`}
              value={set.defaultNew ?? ''}
              disabled={busy}
              onChange={(e) => {
                const value = e.target.value || null;
                change(() => api.setSignatureDefaults(accountId, value, set.defaultReply), 'Saved.');
              }}
            >
              {options}
              <option value="">No signature</option>
            </select>
          </label>
          <label className="form-field">
            <span className="form-field__label" id={`${id}-reply`}>
              For replies and forwards
            </span>
            <select
              className="field"
              aria-labelledby={`${id}-reply`}
              value={set.defaultReply ?? ''}
              disabled={busy}
              onChange={(e) => {
                const value = e.target.value || null;
                change(() => api.setSignatureDefaults(accountId, set.defaultNew, value), 'Saved.');
              }}
            >
              {options}
              <option value="">No signature</option>
            </select>
          </label>
        </div>
      )}

      {error && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      {done && (
        <p className="form-hint" role="status">
          {done}
        </p>
      )}
    </div>
  );
}

function OneSignature({
  accountId,
  signature,
  busy,
  onChange,
}: {
  accountId: string;
  signature: NamedSignature;
  busy: boolean;
  onChange: (run: () => Promise<SignatureSet | null>, success: string) => void;
}) {
  const api = useMailApi();
  const id = useId();
  const box = useRef<HTMLDivElement>(null);
  const [name, setName] = useState(signature.name);
  const [edited, setEdited] = useState(false);
  const [confirm, setConfirm] = useState(false);

  // Show the cleaned HTML the backend stored (never pasted content as-is).
  useEffect(() => {
    if (box.current) box.current.innerHTML = signature.html;
  }, [signature.html]);

  const save = (html: string, success: string) => {
    setEdited(false);
    onChange(() => api.saveSignature(accountId, signature.id, name, html), success);
  };

  // Pasting replaces the words with the cleaned version of what was pasted.
  const paste = (event: ClipboardEvent<HTMLDivElement>) => {
    event.preventDefault();
    const html = event.clipboardData.getData('text/html');
    const text = event.clipboardData.getData('text/plain');
    if (!html && !text) return;
    save(html || textToHtml(text), 'Signature pasted and saved.');
  };

  const refuseDrop = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault();
  };

  return (
    <section className="signature-editor__item" aria-label={`Signature: ${signature.name}`}>
      <label className="form-field">
        <span className="form-field__label">Name</span>
        <input
          className="field"
          value={name}
          maxLength={60}
          onChange={(e) => {
            setName(e.target.value);
            setEdited(true);
          }}
        />
      </label>
      <p className="form-field__label" id={`${id}-words`}>
        Words
      </p>
      <div
        ref={box}
        className="signature-editor__box field"
        role="textbox"
        aria-multiline="true"
        aria-labelledby={`${id}-words`}
        contentEditable={!busy}
        suppressContentEditableWarning
        tabIndex={0}
        onPaste={paste}
        onDrop={refuseDrop}
        onInput={() => {
          setEdited(true);
        }}
      />
      <div className="signature-editor__actions">
        <button
          type="button"
          className="btn btn--secondary"
          disabled={busy || !edited}
          onClick={() => {
            save(box.current?.innerHTML ?? '', 'Signature saved.');
          }}
        >
          Save signature
        </button>
        <button
          type="button"
          className="btn btn--link btn--danger-text"
          disabled={busy}
          onClick={() => {
            setConfirm(true);
          }}
        >
          Delete signature
        </button>
      </div>

      <div className="signature-editor__picture">
        <p className="form-field__label">Signature picture</p>
        {signature.image ? (
          <>
            <img
              className="signature-editor__image"
              src={`data:${signature.image.mimeType};base64,${signature.image.data}`}
              alt={`Signature design: ${signature.name}`}
              style={{ width: signature.image.width }}
            />
            <label className="signature-editor__size">
              <span>Size in messages</span>
              <select
                className="field"
                value={signature.image.width}
                disabled={busy}
                onChange={(e) => {
                  const width = Number(e.target.value);
                  onChange(() => api.setSignatureImageWidth(accountId, signature.id, width), 'Picture size saved.');
                }}
              >
                {widthChoices(signature.image).map((w) => (
                  <option key={w} value={w}>
                    {sizeLabel(w)}
                  </option>
                ))}
              </select>
            </label>
            <div className="signature-editor__actions">
              <button
                type="button"
                className="btn btn--secondary"
                disabled={busy}
                onClick={() => {
                  onChange(() => api.setSignatureImage(accountId, signature.id), 'Picture replaced.');
                }}
              >
                Replace picture
              </button>
              <button
                type="button"
                className="btn btn--link"
                disabled={busy}
                onClick={() => {
                  onChange(() => api.removeSignatureImage(accountId, signature.id), 'Picture removed.');
                }}
              >
                Remove picture
              </button>
            </div>
          </>
        ) : (
          <div className="signature-editor__actions">
            <button
              type="button"
              className="btn btn--secondary"
              disabled={busy}
              aria-describedby={`${id}-picture-hint`}
              onClick={() => {
                onChange(
                  () => api.setSignatureImage(accountId, signature.id),
                  'Picture added. It will show under your signature in messages you send.',
                );
              }}
            >
              Add a picture
            </button>
            <span className="form-hint" id={`${id}-picture-hint`}>
              PNG, JPG or GIF, up to 1 MB
            </span>
          </div>
        )}
      </div>

      {confirm && (
        <ConfirmDialog
          title={`Delete ${signature.name}?`}
          description="Messages you already sent keep it."
          confirmLabel="Delete signature"
          danger
          onCancel={() => {
            setConfirm(false);
          }}
          onConfirm={() => {
            setConfirm(false);
            onChange(() => api.deleteSignature(accountId, signature.id), 'Signature deleted.');
          }}
        />
      )}
    </section>
  );
}

const WIDTHS = [160, 240, 320, 400, 480, 600];

/** Sizes offered: never wider than the picture, always including the current one. */
function widthChoices(image: SignatureImage): number[] {
  const fits = WIDTHS.filter((w) => w <= image.naturalWidth);
  return [...new Set([...fits, image.width])].sort((a, b) => a - b);
}

function sizeLabel(width: number): string {
  const name = width <= 160 ? 'Small' : width <= 320 ? 'Medium' : width <= 480 ? 'Large' : 'Extra large';
  return `${name} (${String(width)} px wide)`;
}

/** Plain pasted text as HTML that keeps its lines. */
function textToHtml(text: string): string {
  const escape = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  return text
    .split(/\r?\n/)
    .map((line) => `<div>${escape(line) || '<br>'}</div>`)
    .join('');
}
