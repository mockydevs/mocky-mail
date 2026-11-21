import type { MailApi, Mailbox, MessageDetail } from '../api/types';
import { EMAIL_CSP, EMAIL_CSP_REMOTE_IMAGES } from '../features/reader/SandboxedHtml';
import { formatLongDate } from './format';

function escape(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function people(list: readonly Mailbox[]): string {
  return list.map((m) => (m.name ? `${m.name} <${m.address}>` : m.address)).join(', ');
}

/**
 * A plain page for printing one message: subject, who and when, then the
 * body. The HTML body is already sanitized by the backend; plain text is
 * escaped. Nothing in it runs scripts. The page carries the reader's own
 * policy, so printing never fetches remote images the reader is blocking;
 * `allowRemoteImages` is the choice made for this message there.
 */
export function printableHtml(message: MessageDetail, allowRemoteImages = false): string {
  const csp = allowRemoteImages ? EMAIL_CSP_REMOTE_IMAGES : EMAIL_CSP;
  const rows: [string, string][] = [
    ['From', message.from ? people([message.from]) : ''],
    ['To', people(message.to)],
    ['Cc', people(message.cc)],
    ['Date', message.date === null ? '' : formatLongDate(message.date)],
  ];
  const head = rows
    .filter(([, value]) => value !== '')
    .map(([label, value]) => `<tr><th>${label}</th><td>${escape(value)}</td></tr>`)
    .join('');
  const files = message.attachments.map((a) => escape(a.fileName)).join(', ');
  const body =
    message.html ?? `<div style="white-space:pre-wrap;font:14px/1.5 Arial,sans-serif">${escape(message.text ?? '')}</div>`;
  return `<!doctype html><html><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${csp}">
<meta name="referrer" content="no-referrer">
<title>${escape(message.subject)}</title>
<style>
body{font-family:Arial,Helvetica,sans-serif;color:#111;margin:24px}
h1{font-size:20px;margin:0 0 12px}
table.head{border-collapse:collapse;margin-bottom:16px;font-size:13px}
table.head th{text-align:left;color:#555;font-weight:600;padding:2px 12px 2px 0;vertical-align:top}
.files{font-size:13px;color:#555;margin-bottom:12px}
hr{border:0;border-top:1px solid #ccc;margin:12px 0 16px}
img{max-width:100%;height:auto}
</style></head><body>
<h1>${escape(message.subject || '(no subject)')}</h1>
<table class="head">${head}</table>
${files ? `<p class="files">Attachments: ${files}</p>` : ''}
<hr>
${body}
</body></html>`;
}

/** Android's WebView can't print a page itself; the backend shows the system print screen. */
function isAndroid(): boolean {
  return /Android/i.test(navigator.userAgent);
}

/**
 * Print a message. Desktop: the system print dialog, for a page holding
 * only the message. Android: the system print screen (also "Save as PDF").
 */
export async function printMessage(api: MailApi, message: MessageDetail, allowRemoteImages = false): Promise<void> {
  const html = printableHtml(message, allowRemoteImages);
  if (isAndroid()) {
    await api.printHtml(html, message.subject || 'Email');
    return;
  }
  const frame = document.createElement('iframe');
  // Same origin so the page can ask the frame to print; no scripts inside it.
  frame.setAttribute('sandbox', 'allow-same-origin allow-modals');
  frame.setAttribute('aria-hidden', 'true');
  frame.style.cssText = 'position:fixed;width:0;height:0;border:0;right:0;bottom:0;';
  await new Promise<void>((resolve) => {
    frame.addEventListener('load', () => {
      resolve();
    });
    frame.srcdoc = html;
    document.body.append(frame);
  });
  frame.contentWindow?.focus();
  frame.contentWindow?.print();
  // The dialog blocks until closed on most systems; remove the frame after.
  setTimeout(() => {
    frame.remove();
  }, 60_000);
}
