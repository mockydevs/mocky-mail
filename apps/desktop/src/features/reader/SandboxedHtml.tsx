import type { Theme } from '../../theme/useTheme';

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
  ':root{color-scheme:light}html,body{margin:0;background:#fff;color:#2b2b28}body{padding:16px 24px;font:15px/1.6 Inter,system-ui,sans-serif;overflow-wrap:anywhere}img{max-width:100%;height:auto}';

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
export function emailDocument(sanitizedHtml: string, theme: Theme = 'light', allowRemoteImages = false): string {
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
    `<style>${LIGHT_STYLE}${theme === 'dark' ? DARK_STYLE : ''}</style>`,
    '</head><body>',
    sanitizedHtml,
    '</body></html>',
  ].join('');
}

/**
 * Renders an HTML email in a sandboxed iframe. The only permission is
 * `allow-popups`, so link clicks become new-window requests that the app
 * hands to the system browser; scripts, forms, top navigation and
 * same-origin access to the app stay blocked.
 */
export function SandboxedHtml({
  html,
  title,
  theme = 'light',
  allowRemoteImages = false,
}: {
  html: string;
  title: string;
  theme?: Theme;
  allowRemoteImages?: boolean;
}) {
  return (
    <iframe
      className="email-frame"
      title={title}
      sandbox="allow-popups"
      referrerPolicy="no-referrer"
      srcDoc={emailDocument(html, theme, allowRemoteImages)}
      // Fill the reading pane: grow into the space left below the header; the frame scrolls itself.
      style={{ flex: '1 1 auto', width: '100%', minHeight: 0, border: 0 }}
    />
  );
}
