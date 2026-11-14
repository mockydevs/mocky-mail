/**
 * Content-Security-Policy for an email body: no scripts, no frames, no
 * network except images the sanitizer already allowed (https, inline cid:
 * and data:). Inline styles stay because nearly all email relies on them.
 */
export const EMAIL_CSP = "default-src 'none'; img-src cid: data: https:; style-src 'unsafe-inline'";

/**
 * Build the iframe document. The HTML must already be sanitized by the Rust
 * core; this is the second layer, not the first.
 */
export function emailDocument(sanitizedHtml: string): string {
  return [
    '<!doctype html><html><head><meta charset="utf-8">',
    `<meta http-equiv="Content-Security-Policy" content="${EMAIL_CSP}">`,
    '<meta name="referrer" content="no-referrer">',
    // Emails are designed for a light page; never let a dark app theme leak in.
    '<meta name="color-scheme" content="light">',
    '<base target="_blank">',
    '<style>:root{color-scheme:light}html,body{margin:0;background:#fff;color:#2b2b28}body{padding:16px 24px;font:15px/1.6 Inter,system-ui,sans-serif;overflow-wrap:anywhere}img{max-width:100%;height:auto}</style>',
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
export function SandboxedHtml({ html, title }: { html: string; title: string }) {
  return (
    <iframe
      className="email-frame"
      title={title}
      sandbox="allow-popups"
      referrerPolicy="no-referrer"
      srcDoc={emailDocument(html)}
      // Fill the reading pane: grow into the space left below the header; the frame scrolls itself.
      style={{ flex: '1 1 auto', width: '100%', minHeight: 0, border: 0 }}
    />
  );
}
