import { screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import type { MessageDetail } from '../../api/types';
import { printableHtml } from '../../lib/print';
import { reader, renderApp, rows } from '../../test/renderApp';
import { EMAIL_CSP, EMAIL_CSP_REMOTE_IMAGES } from './SandboxedHtml';

async function more() {
  const ctx = renderApp();
  await rows();
  await within(reader()).findAllByRole('button', { name: /^Preview / });
  await ctx.user.click(within(reader()).getByRole('button', { name: 'More actions' }));
  return ctx;
}

describe('message tools', () => {
  it('saves the message as .eml and reports where', async () => {
    const { user } = await more();
    await user.click(within(reader()).getByRole('button', { name: 'Save as .eml' }));
    expect(await within(reader()).findByText(/^Saved: .*\.eml$/)).toBeInTheDocument();
  });

  it('shows the source in a dialog that can be copied and closed', async () => {
    const { user } = await more();
    await user.click(within(reader()).getByRole('button', { name: 'View source' }));
    const dialog = within(await screen.findByRole('dialog', { name: 'Message source' }));
    expect(dialog.getByText(/^From: .*Subject: /s)).toBeInTheDocument();
    await user.click(dialog.getByRole('button', { name: 'Close' }));
    expect(screen.queryByRole('dialog', { name: 'Message source' })).not.toBeInTheDocument();
  });

  it('prints through the system print screen on Android', async () => {
    vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Mozilla/5.0 (Linux; Android 14) wv');
    const { user, api } = await more();
    await user.click(within(reader()).getByRole('button', { name: 'Print' }));
    await vi.waitFor(() => {
      expect(api.printed).toHaveLength(1);
    });
    expect(api.printed[0]).toContain('<h1>');
  });

  it('prints under the reader’s image policy, so blocked remote images stay blocked', () => {
    const message = {
      subject: 'Offer',
      from: null,
      to: [],
      cc: [],
      date: null,
      html: '<p>Hi</p><img src="https://tracker.example/pixel.gif">',
      text: null,
      attachments: [],
    } as unknown as MessageDetail;
    const policy = (html: string) => /http-equiv="Content-Security-Policy" content="([^"]*)"/.exec(html)?.[1];
    expect(policy(printableHtml(message))).toBe(EMAIL_CSP);
    expect(policy(printableHtml(message))).not.toContain('https:');
    // Only when the user chose to show this message's images.
    expect(policy(printableHtml(message, true))).toBe(EMAIL_CSP_REMOTE_IMAGES);
  });

  it('builds a print page that escapes what senders wrote', () => {
    const message = {
      subject: 'Quote <b>now</b>',
      from: { name: 'Eve <x>', address: 'eve@x.example' },
      to: [{ name: null, address: 'me@work.example' }],
      cc: [],
      date: 1763000000,
      html: null,
      text: '<script>alert(1)</script>\nThanks',
      attachments: [{ index: 0, fileName: '<img>.pdf', mimeType: 'application/pdf', size: 1, preview: 'pdf' }],
    } as unknown as MessageDetail;
    const html = printableHtml(message);
    expect(html).toContain('Quote &lt;b&gt;now&lt;/b&gt;');
    expect(html).toContain('Eve &lt;x&gt; &lt;eve@x.example&gt;');
    expect(html).toContain('&lt;script&gt;alert(1)&lt;/script&gt;');
    expect(html).toContain('Attachments: &lt;img&gt;.pdf');
    expect(html).not.toContain('<script>');
  });
});
