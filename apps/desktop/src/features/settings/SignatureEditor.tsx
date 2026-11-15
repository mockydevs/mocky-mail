import { useEffect, useId, useRef, useState, type ClipboardEvent, type DragEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { AppError, Signature } from '../../api/types';

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
        Added to new messages, replies and forwards. Paste it from your webmail (for example SOGo's signature
        settings) to keep its logo and links, or type it here.
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
              store('', 'Signature removed.');
            }}
          >
            Remove signature
          </button>
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

/** Plain pasted text as HTML that keeps its lines. */
function textToHtml(text: string): string {
  const escape = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  return text
    .split(/\r?\n/)
    .map((line) => `<div>${escape(line) || '<br>'}</div>`)
    .join('');
}
