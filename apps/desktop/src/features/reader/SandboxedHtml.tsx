import { useEffect, useRef, useState } from 'react';

import type { Theme } from '../../theme/useTheme';
import { fixContrast } from './contrast';
import { fitToPhone, PHONE_STYLE } from './phoneLayout';

/**
 * Content-Security-Policy for an email body: no scripts, no frames and no
 * network. Images may only be inline (cid: and data:). Inline styles stay
 * because nearly all email relies on them.
 */
export const EMAIL_CSP = "default-src 'none'; img-src cid: data:; style-src 'unsafe-inline'";

/**
 * The same policy once the user chose to load remote images for this message
 * (or all messages): https images may load too, still nothing else.
 */
export const EMAIL_CSP_REMOTE_IMAGES = "default-src 'none'; img-src cid: data: https:; style-src 'unsafe-inline'";

/** Base styles. Emails are designed for a light page, so they always start from one. */
const LIGHT_STYLE =
  ':root{color-scheme:light}html,body{margin:0;background:#fff;color:#2b2b28}body{padding:16px 24px;font:15px/1.6 Inter,system-ui,sans-serif;overflow-wrap:break-word}img{max-width:100%;height:auto}';

/**
 * Dark theme: render the light email, then invert the whole page and turn the
 * hue back, so white backgrounds go dark and dark text goes light while
 * colours keep their hue. Photos, logos and background images are inverted a
 * second time so they look as sent. The page starts from #e8e7e4, which lands
 * on the app's dark surface (#17181b) after inversion.
 */
const DARK_STYLE =
  'html,body{background:#e8e7e4}html{filter:invert(1) hue-rotate(180deg)}' +
  'img,video,picture,svg,[background],[style*="background-image"],[style*="background:url"],[style*="background: url"]{filter:invert(1) hue-rotate(180deg)}' +
  '[background] img,[style*="background-image"] img,[style*="background:url"] img,[style*="background: url"] img{filter:none}';

/**
 * Build the iframe document. The HTML must already be sanitized by the Rust
 * core; this is the second layer, not the first.
 */
export function emailDocument(
  sanitizedHtml: string,
  theme: Theme = 'light',
  allowRemoteImages = false,
  phone = false,
): string {
  // A second lock on remote images: if the sanitizer ever misses a tracking
  // pixel, the page itself still refuses to fetch it.
  const csp = allowRemoteImages ? EMAIL_CSP_REMOTE_IMAGES : EMAIL_CSP;
  return [
    '<!doctype html><html><head><meta charset="utf-8">',
    `<meta http-equiv="Content-Security-Policy" content="${csp}">`,
    '<meta name="referrer" content="no-referrer">',
    // Never let the app's colour scheme leak in; dark mode is applied below instead.
    '<meta name="color-scheme" content="light">',
    '<base target="_blank">',
    `<style>${LIGHT_STYLE}${theme === 'dark' ? DARK_STYLE : ''}${phone ? PHONE_STYLE : ''}</style>`,
    '</head><body>',
    sanitizedHtml,
    '</body></html>',
  ].join('');
}

/** Before the email has been measured: about a screenful, so nothing jumps much. */
const UNMEASURED_HEIGHT = 480;

/**
 * Renders an HTML email in a sandboxed iframe. Scripts, forms and top
 * navigation are always blocked; `allow-popups` lets link clicks become
 * new-window requests that the app hands to the system browser.
 *
 * Once loaded, text the email made unreadable (white on a background picture
 * the sanitizer removed, pale grey footers) is given enough contrast. Normally
 * the frame then fills the space it is given and scrolls itself; with
 * `fitContent` (the phone layout) the email is laid out for the narrow screen
 * and the frame grows to its full height, so the message scrolls as one page.
 *
 * Reading the rendered email needs `allow-same-origin`. Scripts stay blocked
 * by the sandbox and the CSP, so the email still cannot run code or reach the
 * app; only the app reads the email.
 */
export function SandboxedHtml({
  html,
  title,
  theme = 'light',
  allowRemoteImages = false,
  fitContent = false,
}: {
  html: string;
  title: string;
  theme?: Theme;
  allowRemoteImages?: boolean;
  fitContent?: boolean;
}) {
  const frame = useRef<HTMLIFrameElement>(null);
  const [height, setHeight] = useState<number | null>(null);

  useEffect(() => {
    const element = frame.current;
    if (!element) return;
    let observer: ResizeObserver | null = null;
    const measure = () => {
      const doc = element.contentDocument;
      if (!doc?.documentElement) return;
      setHeight(Math.ceil(doc.documentElement.scrollHeight));
    };
    const onLoad = () => {
      const doc = element.contentDocument;
      if (!doc?.body) return;
      if (fitContent) fitToPhone(doc);
      fixContrast(doc);
      if (!fitContent) return;
      // Phone: size the frame to the laid-out email.
      measure();
      observer?.disconnect();
      // The frame's own ResizeObserver: it watches the email's layout, which
      // also changes when images arrive.
      const view = element.contentWindow as (Window & typeof globalThis) | null;
      const body = element.contentDocument?.body;
      if (view?.ResizeObserver && body) {
        const next = new view.ResizeObserver(measure);
        next.observe(body);
        observer = next;
      }
    };
    element.addEventListener('load', onLoad);
    onLoad();
    return () => {
      element.removeEventListener('load', onLoad);
      observer?.disconnect();
    };
  }, [fitContent, html, theme, allowRemoteImages]);

  return (
    <iframe
      ref={frame}
      className="email-frame"
      title={title}
      sandbox="allow-popups allow-same-origin"
      referrerPolicy="no-referrer"
      srcDoc={emailDocument(html, theme, allowRemoteImages, fitContent)}
      style={
        fitContent
          ? // As tall as the email; the page around it scrolls.
            { width: '100%', height: `${String(height ?? UNMEASURED_HEIGHT)}px`, border: 0, flex: 'none' }
          : // Fill the reading pane: grow into the space left below the header; the frame scrolls itself.
            { flex: '1 1 auto', width: '100%', minHeight: 0, border: 0 }
      }
    />
  );
}
