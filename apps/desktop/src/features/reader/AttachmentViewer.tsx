import { lazy, Suspense, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { AppError, AttachmentMeta } from '../../api/types';
import { formatBytes } from '../../lib/format';
import { matchSystemBars, setSystemBars } from '../../theme/useTheme';
import { Icon } from '../../ui/Icon';

const PdfPages = lazy(() => import('./PdfPages').then((m) => ({ default: m.PdfPages })));

/** Characters of a text file shown; the rest is in "Open in another app". */
const MAX_TEXT_CHARS = 500_000;

interface Props {
  messageId: string;
  file: AttachmentMeta;
  onClose: () => void;
  onSave: () => void;
  onOpen: () => void;
  /** Result of the last Save or Open, shown along the bottom. */
  notice: { error: AppError | null; text: string | null };
}

type View = { state: 'loading' } | { state: 'ready'; data: ArrayBuffer } | { state: 'failed'; message: string };

/**
 * Full-screen preview of a photo, PDF or text attachment, like Gmail's:
 * tap an attachment, read it, close. Save and "Open in another app" stay
 * one tap away, and are what's offered whenever a file can't be shown.
 */
export function AttachmentViewer({ messageId, file, onClose, onSave, onOpen, notice }: Props) {
  const api = useMailApi();
  const [view, setView] = useState<View>({ state: 'loading' });
  const rootRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const onCloseRef = useRef(onClose);
  useEffect(() => {
    onCloseRef.current = onClose;
  });

  useEffect(() => {
    let live = true;
    api.previewAttachment(messageId, file.index).then(
      (data) => {
        if (live) setView({ state: 'ready', data });
      },
      (e: unknown) => {
        if (live) setView({ state: 'failed', message: toAppError(e).message });
      },
    );
    return () => {
      live = false;
    };
  }, [api, messageId, file.index]);

  // Esc closes, Tab stays inside, and the mail shortcuts behind (r, e, #…)
  // don't fire while it's open. Focus starts on Close and returns after.
  useEffect(() => {
    const root = rootRef.current;
    // Light clock and battery icons over the dark preview (Android).
    setSystemBars(true, '#1f1f22');
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    closeRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      event.stopPropagation();
      if (event.key === 'Escape') {
        closeRef.current?.click();
        return;
      }
      if (event.key !== 'Tab' || !root) return;
      const items = [...root.querySelectorAll<HTMLElement>('button:not([disabled])')];
      const first = items[0];
      const last = items[items.length - 1];
      if (!first || !last) return;
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener('keydown', onKeyDown, true);
    return () => {
      document.removeEventListener('keydown', onKeyDown, true);
      matchSystemBars();
      previous?.focus();
    };
  }, []);

  // Android's Back button closes the preview, as in Gmail: the preview adds
  // a history entry, and going back from it closes it. Close and Esc go
  // back through history too, so the entry never lingers.
  useEffect(() => {
    window.history.pushState({ ...(window.history.state as object | null), mockyViewer: true }, '');
    const onPop = () => {
      if (!isViewerEntry()) onCloseRef.current();
    };
    window.addEventListener('popstate', onPop);
    return () => {
      window.removeEventListener('popstate', onPop);
      // Closed some other way (e.g. another message opened): drop the marker.
      if (isViewerEntry()) {
        window.history.replaceState({ ...(window.history.state as object), mockyViewer: undefined }, '');
      }
    };
  }, []);

  const close = () => {
    if (isViewerEntry()) window.history.back();
    else onClose();
  };

  const failPdf = () => {
    setView({ state: 'failed', message: `${file.fileName} couldn't be shown here. Open it in another app or save it.` });
  };

  return createPortal(
    <div ref={rootRef} className="viewer" role="dialog" aria-modal="true" aria-label={`Preview of ${file.fileName}`}>
      <header className="viewer__bar">
        <button ref={closeRef} type="button" className="icon-btn viewer__btn" aria-label="Close preview" onClick={close}>
          <Icon name="close" />
        </button>
        <div className="viewer__title">
          <span className="viewer__name">{file.fileName}</span>
          <span className="viewer__size">{formatBytes(file.size)}</span>
        </div>
        <button
          type="button"
          className="icon-btn viewer__btn"
          aria-label={`Open ${file.fileName} in another app`}
          title="Open in another app"
          onClick={onOpen}
        >
          <Icon name="external" />
        </button>
        <button
          type="button"
          className="icon-btn viewer__btn"
          aria-label={`Save ${file.fileName}`}
          title="Save"
          onClick={onSave}
        >
          <Icon name="download" />
        </button>
      </header>
      <div className="viewer__body">
        {view.state === 'loading' && (
          <p className="viewer__status" role="status">
            Opening {file.fileName}…
          </p>
        )}
        {view.state === 'failed' && (
          <div className="viewer__status" role="alert">
            <p>{view.message}</p>
            <div className="viewer__fallback">
              <button type="button" className="btn btn--primary" onClick={onOpen}>
                Open in another app
              </button>
              <button type="button" className="btn btn--secondary" onClick={onSave}>
                Save
              </button>
            </div>
          </div>
        )}
        {view.state === 'ready' && file.preview === 'image' && <ImageView data={view.data} file={file} />}
        {view.state === 'ready' && file.preview === 'text' && <TextView data={view.data} />}
        {view.state === 'ready' && file.preview === 'pdf' && (
          <Suspense
            fallback={
              <p className="viewer__status" role="status">
                Opening {file.fileName}…
              </p>
            }
          >
            <PdfPages data={view.data} fileName={file.fileName} onError={failPdf} />
          </Suspense>
        )}
      </div>
      {(notice.text ?? notice.error) && (
        <p className={`viewer__notice${notice.error ? ' viewer__notice--error' : ''}`} role={notice.error ? 'alert' : 'status'}>
          {notice.error?.message ?? notice.text}
        </p>
      )}
    </div>,
    document.body,
  );
}

function isViewerEntry(): boolean {
  const state = window.history.state as { mockyViewer?: boolean } | null;
  return state?.mockyViewer === true;
}

function ImageView({ data, file }: { data: ArrayBuffer; file: AttachmentMeta }) {
  // The backend has checked the bytes are PNG, JPEG, GIF or WebP; the type
  // here only helps the decoder. An <img> never runs scripts.
  const type = file.mimeType.startsWith('image/') && !file.mimeType.includes('svg') ? file.mimeType : 'image/png';
  const url = useMemo(() => URL.createObjectURL(new Blob([data], { type })), [data, type]);
  useEffect(
    () => () => {
      URL.revokeObjectURL(url);
    },
    [url],
  );
  return <img className="viewer__image" src={url} alt={file.fileName} />;
}

function TextView({ data }: { data: ArrayBuffer }) {
  const { text, cut } = useMemo(() => {
    const all = new TextDecoder('utf-8').decode(data);
    return { text: all.slice(0, MAX_TEXT_CHARS), cut: all.length > MAX_TEXT_CHARS };
  }, [data]);
  return (
    <>
      <pre className="viewer__text">{text}</pre>
      {cut && <p className="viewer__note">This file is long; open it in another app to see all of it.</p>}
    </>
  );
}
