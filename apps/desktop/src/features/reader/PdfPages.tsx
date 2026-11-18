/// <reference types="vite/client" />
import { useEffect, useRef, useState } from 'react';

// Loaded only when a PDF is opened, so the app starts as fast as before.
// The "legacy" build carries the extras older Android WebViews need.
const loadPdfJs = async () => {
  const [pdfjs, worker] = await Promise.all([
    import('pdfjs-dist/legacy/build/pdf.mjs'),
    import('pdfjs-dist/legacy/build/pdf.worker.min.mjs?url'),
  ]);
  pdfjs.GlobalWorkerOptions.workerSrc = worker.default;
  return pdfjs;
};

/** Pages drawn at most; longer files say so and offer the full app. */
const MAX_PAGES = 60;

interface Props {
  data: ArrayBuffer;
  fileName: string;
  /** The PDF couldn't be drawn; the viewer offers Open and Save instead. */
  onError: () => void;
}

/**
 * A PDF drawn page by page onto canvases. Nothing in the file can run:
 * PDF scripts and forms are handled by pdf.js's viewer, which isn't used;
 * this only draws the pages.
 */
export function PdfPages({ data, fileName, onError }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [pages, setPages] = useState<{ shown: number; total: number } | null>(null);
  const errorRef = useRef(onError);
  useEffect(() => {
    errorRef.current = onError;
  });

  useEffect(() => {
    const host = ref.current;
    if (!host) return;
    // Read through a function: the cleanup can change it between awaits.
    let cancelled = false;
    const stopped = () => cancelled;
    let destroy: (() => Promise<void>) | null = null;

    void (async () => {
      try {
        const pdfjs = await loadPdfJs();
        // pdf.js takes ownership of the buffer it is given; pass a copy.
        const task = pdfjs.getDocument({ data: new Uint8Array(data.slice(0)) });
        destroy = () => task.destroy();
        const pdf = await task.promise;
        if (stopped()) return;
        const total = pdf.numPages;
        const shown = Math.min(total, MAX_PAGES);
        setPages({ shown, total });
        const ratio = Math.min(window.devicePixelRatio || 1, 2);
        for (let n = 1; n <= shown && !stopped(); n++) {
          const page = await pdf.getPage(n);
          const width = Math.max(host.clientWidth, 240);
          const base = page.getViewport({ scale: 1 });
          const viewport = page.getViewport({ scale: (width / base.width) * ratio });
          const canvas = document.createElement('canvas');
          canvas.className = 'viewer__page';
          canvas.width = Math.floor(viewport.width);
          canvas.height = Math.floor(viewport.height);
          canvas.setAttribute('role', 'img');
          canvas.setAttribute('aria-label', `${fileName}, page ${String(n)} of ${String(total)}`);
          host.appendChild(canvas);
          await page.render({ canvas, viewport }).promise;
        }
      } catch {
        if (!stopped()) errorRef.current();
      }
    })();

    return () => {
      cancelled = true;
      void destroy?.();
      host.replaceChildren();
    };
  }, [data, fileName]);

  return (
    <>
      <div ref={ref} className="viewer__pages" />
      {pages && pages.total > pages.shown && (
        <p className="viewer__note">
          Showing the first {pages.shown} of {pages.total} pages. Open it in another app to see the rest.
        </p>
      )}
    </>
  );
}
