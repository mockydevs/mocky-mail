import { useEffect, useId, useRef, useState, type ClipboardEvent, type DragEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { AppError, Signature, SignatureImage } from '../../api/types';

/**
 * An account's email signature. Paste one from webmail (SOGo, Gmail, …) to
 * keep its formatting, links and logo, or type it. Pasted content goes
 * through the backend's sanitizer before it is shown here; nothing pasted is
 * put into the page as-is.
 */
export function SignatureEditor({ accountId }: { accountId: string }) {
  const api = useMailApi();
  const id = useId();
  const box = useRef<HTMLDivElement>(null);
  const [saved, setSaved] = useState<Signature | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [edited, setEdited] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const [done, setDone] = useState<string | null>(null);

  /** Show cleaned HTML from the backend in the box. */
  const show = (signature: Signature | null) => {
    if (box.current) box.current.innerHTML = signature?.html ?? '';
  };

  useEffect(() => {
    let active = true;
    api.getSignature(accountId).then(
      (signature) => {
        if (!active) return;
        setSaved(signature);
        show(signature);
        setLoaded(true);
      },
      (e: unknown) => {
        if (active) setError(toAppError(e));
      },
    );
    return () => {
      active = false;
    };
  }, [api, accountId]);

  /** Run a signature change and show what was stored. */
  const change = (run: () => Promise<Signature | null>, success: string, keepText = false) => {
    setBusy(true);
    setError(null);
    setDone(null);
    run().then(
      (signature) => {
        setSaved(signature);
        // Picture changes leave unsaved typing in the box alone.
        if (!keepText) show(signature);
        if (!keepText) setEdited(false);
        setBusy(false);
        setDone(success);
      },
      (e: unknown) => {
        setError(toAppError(e));
        setBusy(false);
      },
    );
  };

  const addPicture = () => {
    setBusy(true);
    setError(null);
    setDone(null);
    api.setSignatureImage(accountId).then(
      (signature) => {
        setBusy(false);
        // Null: the picker was cancelled.
        if (signature) {
          setSaved(signature);
          setDone('Picture added. It will show under your signature in messages you send.');
        }
      },
      (e: unknown) => {
        setError(toAppError(e));
        setBusy(false);
      },
    );
  };

  const store = (html: string, success: string) => {
    setBusy(true);
    setError(null);
    setDone(null);
    api.setSignature(accountId, html).then(
      (signature) => {
        setSaved(signature);
        show(signature);
        setEdited(false);
        setBusy(false);
        setDone(signature ? success : 'Signature removed.');
      },
      (e: unknown) => {
        setError(toAppError(e));
        setBusy(false);
      },
    );
  };

  // Pasting replaces the signature with the cleaned version of what was pasted.
  const paste = (event: ClipboardEvent<HTMLDivElement>) => {
    event.preventDefault();
    const html = event.clipboardData.getData('text/html');
    const text = event.clipboardData.getData('text/plain');
    if (!html && !text) return;
    store(html || textToHtml(text), 'Signature pasted and saved.');
  };

  const refuseDrop = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault();
  };

  return (
    <div className="signature-editor">
      <p className="form-field__label" id={`${id}-label`}>
        Signature
      </p>
      <p className="form-hint" id={`${id}-hint`}>
        Added to new messages, replies and forwards. Type it here or paste it from your webmail. To use a
        signature you designed (for example in Illustrator), export it as a PNG or JPG and add it as a picture.
      </p>
      <div
        ref={box}
        className="signature-editor__box field"
        role="textbox"
        aria-multiline="true"
        aria-labelledby={`${id}-label`}
        aria-describedby={`${id}-hint`}
        contentEditable={loaded && !busy}
        suppressContentEditableWarning
        tabIndex={0}
        onPaste={paste}
        onDrop={refuseDrop}
        onInput={() => {
          setEdited(true);
          setDone(null);
        }}
      />
      <div className="signature-editor__actions">
        <button
          type="button"
          className="btn btn--secondary"
          disabled={busy || !edited}
          onClick={() => {
            store(box.current?.innerHTML ?? '', 'Signature saved.');
          }}
        >
          Save signature
        </button>
        {saved && (
          <button
            type="button"
            className="btn btn--link"
            disabled={busy}
            onClick={() => {
              change(
                () => api.removeSignatureImage(accountId).then(() => api.setSignature(accountId, '')),
                'Signature removed.',
              );
            }}
          >
            Remove signature
          </button>
        )}
      </div>

      <div className="signature-editor__picture">
        <p className="form-field__label" id={`${id}-picture`}>
          Signature picture
        </p>
        {saved?.image ? (
          <>
            <img
              className="signature-editor__image"
              src={`data:${saved.image.mimeType};base64,${saved.image.data}`}
              alt="Your signature"
              style={{ width: saved.image.width }}
            />
            <label className="signature-editor__size">
              <span>Size in messages</span>
              <select
                className="field"
                value={saved.image.width}
                disabled={busy}
                onChange={(e) => {
                  const width = Number(e.target.value);
                  change(() => api.setSignatureImageWidth(accountId, width), 'Picture size saved.', true);
                }}
              >
                {widthChoices(saved.image).map((w) => (
                  <option key={w} value={w}>
                    {sizeLabel(w)}
                  </option>
                ))}
              </select>
            </label>
            <div className="signature-editor__actions">
              <button type="button" className="btn btn--secondary" disabled={busy} onClick={addPicture}>
                Replace picture
              </button>
              <button
                type="button"
                className="btn btn--link"
                disabled={busy}
                onClick={() => {
                  change(() => api.removeSignatureImage(accountId), 'Picture removed.', true);
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
              disabled={busy || !loaded}
              aria-describedby={`${id}-picture-hint`}
              onClick={addPicture}
            >
              Add a picture
            </button>
            <span className="form-hint" id={`${id}-picture-hint`}>
              PNG, JPG or GIF, up to 1 MB
            </span>
          </div>
        )}
      </div>
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
