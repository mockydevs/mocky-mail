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
    '<base target="_blank">',
    '<style>html,body{margin:0;font:15px/1.6 Inter,system-ui,sans-serif;color:#2b2b28;background:#fff;overflow-wrap:anywhere}img{max-width:100%;height:auto}</style>',
    '</head><body>',
    sanitizedHtml,
    '</body></html>',
  ].join('');
}

/**
 * Renders an HTML email in a fully sandboxed iframe: an empty `sandbox`
 * attribute blocks scripts, forms, popups, top navigation and same-origin
 * access to the app.
 */
export function SandboxedHtml({ html, title }: { html: string; title: string }) {
  return <iframe className="email-frame" title={title} sandbox="" referrerPolicy="no-referrer" srcDoc={emailDocument(html)} />;
}
